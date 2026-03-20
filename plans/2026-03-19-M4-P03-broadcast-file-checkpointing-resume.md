# PRD: Phase 3 - Broadcast File Checkpointing & Resume

## Introduction

Phase 3 restructures broadcast file writing so that `ScriptSequence` is built **before** routing and updated incrementally during broadcast. Today, `build_script_sequence()` runs after all routing completes, which means a crash mid-broadcast loses all receipt data. This phase moves construction earlier, adds per-transaction checkpoint saves to `run-latest.json`, and enhances `--resume` to poll pending hashes and re-send dropped transactions.

This phase depends on Phase 1 (Alloy Provider & Live Signing), which established the alloy provider pattern and live `send_raw_transaction` flow. The per-transaction checkpoint saves slot into the existing `broadcast_wallet_run_live` and `broadcast_routable_txs` loops, and the enhanced resume uses the same alloy provider to poll receipt status.

## Goals

1. **Crash-safe broadcast state**: After each confirmed receipt, `run-latest.json` is written to disk so a crash mid-broadcast preserves all prior progress.
2. **Accurate resume**: `--resume` polls pending tx hashes via `eth_getTransactionReceipt`, distinguishes confirmed/pending/dropped transactions, and only re-sends what's needed.
3. **Foundry-compatible output**: The pre-built `ScriptSequence` produces identical JSON structure to the current post-routing construction — no format regressions.
4. **Re-enable broadcast-dependent compose tests**: At least 5 of the 15 ignored `integration_compose` tests that depend on broadcast file state pass after these changes.

## User Stories

### P3-US-001: Build ScriptSequence before routing

**Description**: Restructure `build_script_sequence()` to construct a `ScriptSequence` from `BroadcastableTransactions` and `RecordedTransaction` metadata **before** routing results exist. Transactions are initialized with `hash: None` and no receipts. This "skeleton" sequence serves as the mutable checkpoint target during broadcast.

**Changes**:
- Add `build_pre_routing_sequence()` in `crates/treb-forge/src/pipeline/broadcast_writer.rs` that takes `btxs`, `recorded_txs`, `PipelineContext`, and paths — but NOT `run_results`
- Each `TransactionWithMetadata` is built from `btxs[i].transaction` with contract metadata from `recorded_txs[i]`, `hash: None`
- Keep existing `build_script_sequence()` unchanged (post-routing path used by compose and as reference)
- Add unit tests: verify transaction count matches `btxs` length, verify `hash` is `None` on all entries, verify `receipts` is empty

**Acceptance Criteria**:
- [ ] `build_pre_routing_sequence()` produces a `ScriptSequence` with `transactions.len() == btxs.len()`
- [ ] All `transactions[i].hash` are `None`
- [ ] `receipts` is empty, `pending` is empty
- [ ] Contract metadata (name, address, opcode) matches existing `build_script_sequence()` output for the same inputs
- [ ] Unit tests pass
- [ ] `cargo clippy --workspace --all-targets` passes
- [ ] `cargo test -p treb-forge` passes

---

### P3-US-002: Thread &mut ScriptSequence through execute_plan

**Description**: Modify `execute_plan()` and broadcast functions to accept an optional `&mut ScriptSequence` that gets updated as each transaction is confirmed. The `RouteContext` gains an optional sequence reference. When present, after each `BroadcastReceipt` is produced in the broadcast loop, the corresponding `sequence.transactions[tx_idx].hash` is set and the receipt is appended to `sequence.receipts`.

**Changes**:
- Add `sequence: Option<&mut ScriptSequence>` field to `RouteContext` in `crates/treb-forge/src/pipeline/routing.rs`
- Pass through to `broadcast_wallet_run()`, `broadcast_wallet_run_fork()`, `broadcast_wallet_run_live()`, and `broadcast_routable_txs()` (add `sequence: Option<&mut ScriptSequence>` parameter)
- Inside the per-tx loop in `broadcast_wallet_run_live()` (line ~1212) and `broadcast_wallet_run_fork()` (line ~1091): after pushing the receipt, call a new helper `update_sequence_checkpoint(sequence, tx_idx, &receipt)` that sets `hash` and appends the raw receipt
- All existing callers pass `None` for now (no behavioral change) — actual checkpoint saving is wired in US-003
- Update `route_all()`, `route_all_with_queued()`, `route_all_with_resume()` signatures to thread the sequence through

**Acceptance Criteria**:
- [ ] `RouteContext` has `sequence: Option<&mut ScriptSequence>` field
- [ ] Broadcast functions accept the optional sequence parameter
- [ ] `update_sequence_checkpoint()` helper exists and sets `hash` + appends receipt
- [ ] All existing callers compile with `sequence: None`
- [ ] All existing tests pass unchanged (`cargo test --workspace --all-targets`)
- [ ] `cargo clippy --workspace --all-targets` passes

---

### P3-US-003: Checkpoint save after each confirmed receipt

**Description**: Wire the pre-built `ScriptSequence` into the `treb run` broadcast path. After each receipt updates the sequence (via US-002), call `sequence.save(true, false)` to write `run-latest.json` to disk. The `true` flag writes the `-latest` file, `false` skips the timestamped copy (only the final save writes the timestamped copy). Update `apply_routing_results_with_queued()` in the orchestrator to accept an already-populated `&mut ScriptSequence` instead of building one from scratch.

**Changes**:
- In `crates/treb-cli/src/commands/run.rs`: build the skeleton sequence via `build_pre_routing_sequence()` before calling `route_all_with_queued()`; pass `Some(&mut sequence)` in the `RouteContext`
- In `update_sequence_checkpoint()` (from US-002): after setting hash and appending receipt, call `sequence.save(true, false)` — wrap errors as `TrebError::Forge`
- Ensure broadcast/cache directories are created before the first checkpoint (reuse the `create_dir_all` logic from `write_broadcast_artifacts()`)
- In `apply_routing_results_with_queued()`: skip `build_script_sequence()` when an existing populated sequence is provided; only call `sequence.save(true, true)` for the final write (with timestamped copy)
- Add integration test: run a 2-tx script with `--broadcast`, verify `run-latest.json` exists with 2 transactions and 2 receipts

**Acceptance Criteria**:
- [ ] `run-latest.json` is written after **each** confirmed receipt, not only at the end
- [ ] Final save produces both `-latest.json` and timestamped copy
- [ ] A crash after tx 1 of 2 leaves `run-latest.json` with 1 hash set and 1 receipt
- [ ] `cargo test --workspace --all-targets` passes
- [ ] `cargo clippy --workspace --all-targets` passes

---

### P3-US-004: Poll pending hashes in load_resume_state

**Description**: Enhance `load_resume_state()` to accept an RPC URL and poll `eth_getTransactionReceipt` for transactions that have a hash but no matching receipt in the saved sequence. This distinguishes three states: **confirmed** (receipt exists), **pending** (hash set, no receipt yet), **unsent** (hash is `None`). Add a `PendingTxStatus` enum to `ResumeState` to track per-hash status.

**Changes**:
- Make `load_resume_state()` async, add `rpc_url: &str` parameter in `crates/treb-forge/src/pipeline/broadcast_writer.rs`
- After loading the sequence, for each `tx.hash` that has no corresponding receipt: call `provider.get_transaction_receipt(hash)` via alloy provider
  - If receipt found → add to `completed_tx_hashes`, append receipt to sequence
  - If no receipt → add to a new `pending_tx_hashes: HashSet<B256>` field on `ResumeState`
  - If hash is `None` → transaction was never sent (unsent)
- Add `pending_tx_hashes` field to `ResumeState`
- Update callers in `orchestrator.rs` (both `RunPipeline` and `SessionPipeline`) to pass `rpc_url`
- Add unit test: mock a sequence with 3 txs (1 with receipt, 1 with hash but no receipt, 1 with no hash), verify categorization

**Acceptance Criteria**:
- [ ] `load_resume_state()` is async and accepts `rpc_url`
- [ ] Transactions with receipts on-chain are moved to `completed_tx_hashes`
- [ ] Transactions with hash but no on-chain receipt are in `pending_tx_hashes`
- [ ] Transactions with `hash: None` are neither completed nor pending (unsent)
- [ ] All callers updated
- [ ] `cargo test --workspace --all-targets` passes
- [ ] `cargo clippy --workspace --all-targets` passes

---

### P3-US-005: Detect dropped transactions and re-send in resume

**Description**: Enhance `route_all_with_resume()` to handle the three states from US-004. For **confirmed** txs: skip (existing behavior). For **pending** txs: poll once more, if still pending treat as live. For **unsent** txs: re-send. Detect nonce conflicts (a different tx confirmed at the same nonce) by comparing the sender's current nonce against the transaction's expected position — if the nonce is already consumed but the hash doesn't match, log a warning and skip.

**Changes**:
- In `route_all_with_resume()` in `crates/treb-forge/src/pipeline/routing.rs`: use `ResumeState.pending_tx_hashes` to identify runs that are partially complete
- For runs where some txs are confirmed and others are unsent: build a partial `TransactionRun` containing only the unsent `tx_indices` and broadcast those
- For runs where a tx has a pending hash: poll receipt one more time; if confirmed, reconstruct receipt; if still pending, wait and retry (up to 3 attempts with 2s delay)
- Add nonce conflict detection: query `eth_getTransactionCount(from, "latest")` and compare against the expected nonce for each unsent tx; if nonce consumed, warn and mark as skipped
- Add unit tests for the partial-run construction and nonce comparison logic

**Acceptance Criteria**:
- [ ] Confirmed txs are skipped (existing behavior preserved)
- [ ] Unsent txs are re-broadcast
- [ ] Pending txs are polled with retry
- [ ] Nonce conflicts produce a warning, not an error
- [ ] Partial runs only re-send the unsent subset
- [ ] `cargo test --workspace --all-targets` passes
- [ ] `cargo clippy --workspace --all-targets` passes

---

### P3-US-006: Re-enable broadcast-dependent compose tests

**Description**: Re-enable the subset of ignored `integration_compose` tests that can now pass with the checkpoint/resume infrastructure. Update test assertions to match the current CLI surface (no `--dry-run`, simulation-only = omit `--broadcast`). Focus on the resume-related test (`compose_dry_run_resume_verbose`) and the error-path tests that don't depend on broadcast output format.

**Changes**:
- Remove `#[ignore]` from: `compose_error_file_not_found`, `compose_error_invalid_yaml`, `compose_error_empty_components`, `compose_error_cycle`, `compose_error_unknown_dep`, `compose_error_self_dep` (6 error tests — these test compose YAML validation, not broadcast)
- Remove `#[ignore]` from `compose_dry_run_resume_verbose` — update assertions for current flag surface (replace `--dry-run` with simulation-only invocation, update expected output)
- Update golden files if output format changed
- Use `SpinnerNormalizer` for any test that exercises broadcast/simulation output

**Acceptance Criteria**:
- [ ] At least 5 previously-ignored `integration_compose` tests pass
- [ ] `compose_dry_run_resume_verbose` (or its updated equivalent) exercises resume state loading
- [ ] No new golden file drift in other tests
- [ ] `cargo test -p treb-cli` passes
- [ ] `cargo clippy --workspace --all-targets` passes

## Functional Requirements

- **FR-1**: `ScriptSequence` must be constructible from `BroadcastableTransactions` + `RecordedTransaction` metadata without routing results.
- **FR-2**: Each confirmed receipt during broadcast must trigger a `save(true, false)` call that writes `run-latest.json` to disk.
- **FR-3**: The final broadcast save must produce both `run-latest.json` and a timestamped copy (`run-{timestamp}.json`).
- **FR-4**: `load_resume_state()` must poll `eth_getTransactionReceipt` for transactions that have a hash but no local receipt.
- **FR-5**: Resume must distinguish three tx states: confirmed (skip), pending (poll), unsent (re-send).
- **FR-6**: Nonce conflicts during resume must produce a warning log, not a fatal error.
- **FR-7**: The checkpoint file format must be identical to the current `ScriptSequence` JSON format — no breaking changes to Foundry compatibility.
- **FR-8**: Directory creation (`broadcast/` and `cache/`) must happen before the first checkpoint save, not only at final write.

## Non-Goals

- **Full compose test re-enablement**: The 8 remaining compose tests that depend on `--dry-run`/`--dump-command` removal are Phase 5 scope.
- **Safe/Governor resume**: Resume for proposed Safe and Governor transactions is out of scope — only wallet broadcast txs support checkpoint/resume.
- **Multi-script session checkpointing**: The `SessionPipeline` (compose) already has `SessionState` for cross-script progress tracking. This phase does not change session-level persistence.
- **Automatic retry on RPC failure**: If an RPC call fails during broadcast, the error propagates. Automatic retry with backoff is out of scope.
- **Replacing reqwest with alloy provider for fork broadcasts**: Phase 10 handles that migration.

## Technical Considerations

### Dependencies
- **Phase 1 (complete)**: Alloy provider pattern and `broadcast_wallet_run_live()` are the primary integration points for checkpoint saves.
- **forge_script_sequence crate**: `ScriptSequence::save(latest, timestamped)` is a Foundry method. Verify that `save(true, false)` writes only the `-latest` file without creating a timestamped copy. Check if `save()` requires `paths` to be set.
- **alloy-provider**: Used for `get_transaction_receipt(hash)` polling in resume. Reuse the established `ProviderBuilder::new().connect_http(url)` pattern (no wallet needed for read-only calls).

### Key Integration Points
- `crates/treb-forge/src/pipeline/broadcast_writer.rs`: New `build_pre_routing_sequence()`, enhanced `load_resume_state()`
- `crates/treb-forge/src/pipeline/routing.rs`: `RouteContext` gains sequence field, broadcast loops call checkpoint helper, `route_all_with_resume()` handles partial runs
- `crates/treb-forge/src/pipeline/orchestrator.rs`: `apply_routing_results_with_queued()` accepts pre-built sequence
- `crates/treb-cli/src/commands/run.rs`: Constructs skeleton sequence and passes into route context

### Constraints
- `ScriptSequence.save()` is Foundry's method — do not duplicate its serialization logic. If it doesn't support `save(true, false)` cleanly, write the `-latest` file manually using `serde_json::to_writer_pretty` with the same format.
- `RouteContext` already has many fields. Adding `sequence: Option<&mut ScriptSequence>` introduces a mutable borrow that must not alias with other context fields. Consider whether the sequence should be passed separately to `execute_plan()` rather than embedded in the context.
- The `tx_idx` in `broadcast_wallet_run_live()` is a btx index, not a sequence index. If the sequence is built from all btxs (including non-wallet ones), the indices align. If only wallet txs are in the sequence, a mapping is needed. The pre-routing skeleton must include ALL btxs to maintain index alignment.
- RPC URL for resume polling must be resolved via `resolve_rpc_url_for_chain_id()` — never pass a raw foundry.toml alias.
