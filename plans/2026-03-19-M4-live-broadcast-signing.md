# Master Plan: Live Broadcast Signing & Routing Integration Tests

Implement real transaction signing (`eth_sendRawTransaction`) in the queue-reduction routing model so treb can broadcast on both Anvil and live networks. Migrate RPC communication to `alloy-provider`. Add integration tests for all routing scenarios. Re-enable the 64 ignored tests as each phase unlocks them.

**Current state:** 1,783 tests passing, 64 ignored. The queue-reduction model (`reduce_queue` → `execute_plan`) works correctly but only in fork/impersonation mode. Live signing, non-interactive fork simulation, and routing integration tests are missing.

---

## Phase 1 -- Alloy Provider Migration & Live Signing

Replace raw `reqwest` JSON-RPC calls in `broadcast_routable_txs` and `broadcast_wallet_run` with `alloy-provider`. Implement the `!is_fork` signing branch using `TransactionBuilder::build(wallet)` → `encoded_2718()` → `send_raw_transaction`. Support all signer types (PrivateKey, Ledger, Trezor) by using `WalletSigner` directly from `ResolvedSender`, not `extract_signing_key`.

**Deliverables**
- Add `alloy-provider`, `alloy-consensus`, `alloy-network`, `alloy-rpc-types` to `treb-forge/Cargo.toml`
- New `resolve_wallet_for_address()` helper: looks up `ResolvedSender` by address, returns `EthereumWallet`
- `broadcast_routable_txs`: fork branch unchanged, non-fork branch signs + `send_raw_transaction`
- `broadcast_wallet_run`: same treatment
- `execute_plan`: pass `RouteContext` through to broadcast functions
- Re-enable: 13 `integration_run` tests, 1 `integration_non_interactive` test

**User stories:** 5
**Dependencies:** none

---

## Phase 2 -- Non-Interactive Fork Simulation & --skip-fork-execution

Change `handle_queued_executions()` so non-interactive mode defaults to YES for fork simulation (matching Go behavior). Add `--skip-fork-execution` CLI flag that saves queued items without simulating, enabling `treb fork exec` workflow testing.

**Deliverables**
- `handle_queued_executions`: non-interactive fork → auto-simulate (not skip)
- New `--skip-fork-execution` flag on `treb run`
- Thread flag through `ExecuteScriptOpts` → `handle_queued_executions`
- Update `display_script_broadcast_summary` to show "⏳" for governor/safe proposed results
- Re-enable: 3 `e2e_fork_workflow` tests (they use `--non-interactive`)

**User stories:** 4
**Dependencies:** Phase 1

---

## Phase 3 -- Broadcast File Checkpointing & Resume

Restructure broadcast file writing so `ScriptSequence` is built BEFORE routing and updated per-transaction during broadcast. Each confirmed receipt triggers a checkpoint save to `run-latest.json`. Enhance `--resume` to poll pending hashes and re-send dropped transactions.

**Deliverables**
- Build `ScriptSequence` before `execute_plan`, pass as `&mut` through broadcast loop
- After each receipt: update `sequence.transactions[i].hash`, push receipt, call `save(true, false)`
- `load_resume_state`: poll `pending` hashes, detect dropped txs, handle nonce conflicts
- Re-enable: compose tests that depend on broadcast file state (subset of 15 ignored compose tests)

**User stories:** 6
**Dependencies:** Phase 1

---

## Phase 4 -- E2E Broadcast Tests (Wallet Path)

Re-enable and fix the wallet broadcast e2e tests. These test the full pipeline: init → run → broadcast → registry verification. With Phase 1's live signing, they should pass against plain Anvil.

**Deliverables**
- Re-enable: 6 `e2e_workflow` tests
- Re-enable: 3 `e2e_deployment_workflow` tests
- Re-enable: 4 `e2e_registry_consistency` tests
- Re-enable: 1 `e2e_prune_reset_workflow` test
- Fix any golden file drift from routing output changes
- Verify broadcast files written correctly in Foundry-compatible format

**User stories:** 4
**Dependencies:** Phase 1

---

## Phase 5 -- Compose Test Fixes

Fix the 12 ignored `cli_compose` tests and 15 ignored `integration_compose` tests. The compose command behavior is correct — simulation (no `--broadcast`) runs the full pipeline (compile, execute scripts, collect broadcastable transactions). The tests fail because they pass bare YAML files without a foundry project setup. Fix by adding `foundry.toml` + `treb init` to tests that need it, and updating assertions for the removed `--dry-run` and `--dump-command` flags. Do NOT change compose behavior.

**Deliverables**
- Re-enable: 12 `cli_compose` tests — add foundry project setup where needed
- Remove references to `--dry-run` and `--dump-command` in test assertions
- Re-enable: 15 `integration_compose` tests (broadcast-dependent, need Phase 1)
- Verify no compose behavior changes — only test setup changes

**User stories:** 4
**Dependencies:** Phase 1

---

## Phase 6 -- Deploy Real Safe Contract on Anvil

Deploy a real Gnosis Safe (1/1 and 2/3 threshold) on the test Anvil fixture. Create a deployment script that sets up Safe contracts so integration tests can exercise actual threshold queries, `execTransaction`, and Safe TX Service proposal paths.

**Deliverables**
- `tests/fixtures/project/script/DeploySafe.s.sol` — deploys Safe proxy + singleton
- `tests/fixtures/project/src/safe/` — Safe contract ABIs or minimal stubs
- Helper function to deploy Safe with configurable threshold/owners
- Integration test: Safe(1/1) wallet broadcast on fork
- Integration test: Safe(n/m) proposal on fork with `QueuedExecution::SafeProposal`

**User stories:** 6
**Dependencies:** Phase 2

---

## Phase 7 -- Deploy Real Governor Stack on Anvil

Deploy GovernanceToken + TrebTimelock + TrebGovernor on the test Anvil fixture (matching Go CLI's test fixtures). Create scripts that exercise the full governor routing path.

**Deliverables**
- `tests/fixtures/project/src/governance/` — GovernanceToken, TrebTimelock, TrebGovernor
- `tests/fixtures/project/script/DeployGovernance.s.sol` — deploys full stack
- `tests/fixtures/project/script/DeployWithGovernor.s.sol` — routes through governor
- Integration test: Governor → Wallet propose() broadcast
- Integration test: Governor → Safe(1/1) propose() chain
- Integration test: `QueuedExecution::GovernanceProposal` with fork simulation

**User stories:** 7
**Dependencies:** Phase 6

---

## Phase 8 -- Mixed Sender & Edge Case Tests

Test scripts with multiple sender types (wallet + governor, wallet + safe). Test routing depth limits, error paths, and edge cases.

**Deliverables**
- Integration test: mixed wallet + governor senders (partition_into_runs → reduce_queue)
- Integration test: mixed wallet + safe senders
- Integration test: Governor depth limit (>4 levels) error
- Integration test: threshold query failure handling
- Integration test: hardware wallet signer error message (if no key available)
- Re-enable: remaining ignored tests from all categories

**User stories:** 5
**Dependencies:** Phase 7

---

## Phase 9 -- Fork Infrastructure Test Fixes

Fix the 3 ignored fork tests (restart/revert port unreachable, status with active fork) and the 2 compatibility alias tests (config show format, fork history filter).

**Deliverables**
- Fix `fork_restart_port_unreachable` and `fork_revert_port_unreachable` golden files
- Fix `fork_status_with_active_fork` output format
- Fix `bare_config_matches_config_show` assertion (config show output changed)
- Enhance `fork history` to show network/namespace per run entry
- Re-enable: `cli_version` dirty-state test (verify it passes after clean commit)

**User stories:** 5
**Dependencies:** none (can run in parallel with other phases)

---

## Phase 10 -- Migrate All RPC Communication to Alloy Provider

Replace remaining raw `reqwest` JSON-RPC calls throughout the codebase with `alloy-provider`. This includes `fork_routing.rs` (AnvilRpc), `routing.rs` (fetch_receipt), and any other manual RPC calls.

**Deliverables**
- Replace `AnvilRpc` in `fork_routing.rs` with alloy provider
- Replace `fetch_receipt` in `routing.rs` with provider receipt polling
- Remove `reqwest` dependency from `treb-forge` (if fully replaced)
- Consistent provider construction pattern across all RPC-touching code

**User stories:** 6
**Dependencies:** Phase 1

---

## Dependency Graph

```
Phase 1 (Live Signing)
  ├── Phase 2 (Non-Interactive + --skip-fork-execution)
  │     └── Phase 6 (Deploy Safe)
  │           └── Phase 7 (Deploy Governor)
  │                 └── Phase 8 (Mixed Sender Tests)
  ├── Phase 3 (Checkpoint Saves)
  ├── Phase 4 (E2E Broadcast Tests)
  ├── Phase 5 (Compose Fixes)
  └── Phase 10 (Full Provider Migration)

Phase 9 (Fork Fixes) — independent, can run anytime
```

---

## Summary Table

| Phase | Title | Stories | Depends On | Tests Re-enabled |
|------:|-------|--------:|------------|-----------------|
| 1 | Alloy Provider & Live Signing | 5 | -- | 14 |
| 2 | Non-Interactive Fork Sim | 4 | 1 | 3 |
| 3 | Checkpoint Saves & Resume | 6 | 1 | ~5 |
| 4 | E2E Broadcast Tests | 4 | 1 | 14 |
| 5 | Compose Test Fixes | 4 | 1 | 27 |
| 6 | Deploy Real Safe | 6 | 2 | 0 (new tests) |
| 7 | Deploy Real Governor | 7 | 6 | 0 (new tests) |
| 8 | Mixed Sender & Edge Cases | 5 | 7 | ~1 |
| 9 | Fork Infrastructure Fixes | 5 | -- | 6 |
| 10 | Full Provider Migration | 6 | 1 | 0 |
| **Total** | | **52** | | **~64 + new** |

---

## Ignored Test Inventory

For tracking which tests get re-enabled in which phase:

| Test Binary | Count | Phase |
|------------|------:|-------|
| integration_run | 13 | 1 |
| integration_compose | 15 | 5 |
| integration_non_interactive | 1 | 1 |
| integration_fork | 3 | 9 |
| cli_compatibility_aliases | 2 | 9 |
| cli_compose | 12 | 5 |
| cli_version | 1 | 9 |
| e2e_deployment_workflow | 3 | 4 |
| e2e_fork_workflow | 3 | 2 |
| e2e_prune_reset_workflow | 1 | 4 |
| e2e_registry_consistency | 4 | 4 |
| e2e_workflow | 6 | 4 |
| **Total** | **64** | |
