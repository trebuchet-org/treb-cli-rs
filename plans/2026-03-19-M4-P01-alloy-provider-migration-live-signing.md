# PRD: Phase 1 - Alloy Provider Migration & Live Signing

## Introduction

Phase 1 replaces the raw `reqwest` JSON-RPC calls in `broadcast_wallet_run` and `broadcast_routable_txs` with `alloy-provider`, and implements the live-network signing branch that currently returns an error (`"live network broadcast through routing is not yet supported"`). After this phase, `treb run --broadcast` will work against both fork (impersonation) and live (signed transaction) targets for wallet senders.

This is the foundational phase of the master plan. Every subsequent phase that touches broadcasting, checkpointing, e2e tests, or provider migration depends on the signing infrastructure built here. The 14 ignored tests gated behind the `TODO: re-enable after live broadcast signing is implemented` comment will be re-enabled.

## Goals

1. **Live wallet broadcast works end-to-end:** `treb run --broadcast` against a plain Anvil node (no impersonation) signs transactions and receives receipts for all wallet sender types (PrivateKey, Ledger, Trezor).
2. **Zero fork-mode regressions:** The existing impersonation path in `broadcast_wallet_run` and `broadcast_routable_txs` remains unchanged; all currently-passing tests continue to pass.
3. **Signer resolution uses `ResolvedSender` directly:** Live signing uses the `WalletSigner` already present in `ResolvedSender::Wallet` / `ResolvedSender::InMemory` instead of round-tripping through `extract_signing_key` and reconstructing a signer from a hex string.
4. **14 ignored tests re-enabled and green:** 13 `integration_run` tests and 1 `integration_non_interactive` test pass in CI.
5. **Type-checked alloy provider usage:** All new RPC calls use typed alloy-provider methods instead of hand-rolled `serde_json::json!` payloads.

## User Stories

### P1-US-001: Add alloy provider dependencies to treb-forge

**Description:** Add `alloy-provider`, `alloy-consensus`, `alloy-network`, and `alloy-rpc-types` as dependencies to `crates/treb-forge/Cargo.toml` at version `"1.1"` (workspace `[patch.crates-io]` already pins all alloy crates to v1.1.1). Verify that the workspace builds cleanly.

**Acceptance Criteria:**
- `alloy-provider`, `alloy-consensus`, `alloy-network`, `alloy-rpc-types` appear in `crates/treb-forge/Cargo.toml` `[dependencies]`
- Versions are `"1.1"` to match the existing alloy pin strategy
- `cargo check --workspace --all-targets` passes with no errors
- No unintended version resolution changes in `Cargo.lock` (alloy crates stay at v1.1.1)

---

### P1-US-002: Create `resolve_wallet_for_address()` helper

**Description:** Add a function in `crates/treb-forge/src/sender.rs` that looks up a `ResolvedSender` by `Address` from a `&HashMap<String, ResolvedSender>`, walks the sender chain to the leaf wallet (following Safe→signer and Governor→proposer), and returns an `EthereumWallet` wrapping the `WalletSigner`. This replaces the `extract_signing_key` pattern for live signing — instead of extracting a hex key string and rebuilding a signer, we use the already-resolved `WalletSigner` directly.

**Acceptance Criteria:**
- Function signature: `pub fn resolve_wallet_for_address(address: Address, resolved_senders: &HashMap<String, ResolvedSender>) -> Result<EthereumWallet, TrebError>`
- For `ResolvedSender::Wallet(ws)` / `ResolvedSender::InMemory(ws)`: wraps `ws` in `EthereumWallet`
- For `ResolvedSender::Safe { signer, .. }`: recursively resolves the signer
- For `ResolvedSender::Governor { proposer, .. }`: recursively resolves the proposer
- Returns `TrebError::Forge` with a descriptive message if no matching address is found or if the leaf sender has no wallet signer
- Unit tests cover: direct wallet lookup, Safe→Wallet chain, Governor→Wallet chain, missing address error
- `cargo check --workspace --all-targets` passes

---

### P1-US-003: Implement live signing in `broadcast_wallet_run`

**Description:** Replace the `!is_fork` error branch in `broadcast_wallet_run` (`crates/treb-forge/src/pipeline/routing.rs`, ~line 1066) with a signing implementation. Construct an alloy `ProviderBuilder` with the sender's `EthereumWallet` (from `resolve_wallet_for_address`), build each transaction using alloy types, and send via `provider.send_transaction().await`. The fork branch (impersonation via `anvil_impersonateAccount`) remains unchanged.

The function signature needs to accept `resolved_senders: &HashMap<String, ResolvedSender>` (or the full `RouteContext`) so it can look up the wallet for the sender address.

**Acceptance Criteria:**
- `broadcast_wallet_run` no longer returns an error for `!is_fork`
- Live branch constructs an alloy `RootProvider` (or filler-based provider) connected to `rpc_url`
- For each transaction in the run: builds a `TransactionRequest` with `from`, `to`, `value`, `input`, sends via `provider.send_transaction(...).await`, awaits receipt
- Populates `BroadcastReceipt` from the alloy `TransactionReceipt` (hash, block_number, gas_used, status, contract_address)
- Fork branch (`is_fork == true`) behavior is unchanged — still uses impersonation via raw reqwest
- `cargo test --workspace --all-targets` passes with no new failures
- `cargo clippy --workspace --all-targets` passes

---

### P1-US-004: Implement live signing in `broadcast_routable_txs` and thread `RouteContext`

**Description:** Apply the same alloy-provider signing treatment to `broadcast_routable_txs` (`crates/treb-forge/src/pipeline/routing.rs`, ~line 1547). The function currently only supports fork mode. Add the `!is_fork` branch that signs `RoutableTx` items and broadcasts via `send_raw_transaction`. Update `execute_plan` to pass the necessary context (resolved senders) through to both broadcast functions.

**Acceptance Criteria:**
- `broadcast_routable_txs` no longer returns an error for `!is_fork`
- Live branch resolves the wallet for the `from` address, builds `TransactionRequest` from each `RoutableTx`, signs and sends via alloy provider
- `execute_plan` passes `RouteContext` (or its `resolved_senders` field) to `broadcast_wallet_run` and `broadcast_routable_txs`
- Fork branch behavior unchanged (impersonation via raw reqwest)
- Safe `RoutingAction::Exec` on live networks (1/1 threshold) broadcasts the pre-built `execTransaction` calldata through the new signing path
- `cargo test --workspace --all-targets` passes with no new failures
- `cargo clippy --workspace --all-targets` passes

---

### P1-US-005: Re-enable ignored integration tests

**Description:** Remove `#[ignore]` from the 13 `integration_run` tests and 1 `integration_non_interactive` test. Fix any test failures caused by output format differences, golden file drift, or missing test infrastructure now that live signing works.

**Acceptance Criteria:**
- All 13 `integration_run` tests are no longer `#[ignore]`: `run_basic`, `run_dry_run`, `run_basic_json`, `run_verbose`, `run_multi_operation`, `run_verbose_json`, `run_dump_command`, `run_missing_script`, `run_governor_human`, `run_governor_json`, `run_governor_dry_run`, `run_governor_verbose`, `run_bad_signature`
- The 1 `integration_non_interactive` test is no longer `#[ignore]`: `exit_code_zero_on_json_success`
- `cargo test -p treb-cli` passes — all 14 re-enabled tests are green
- Golden files updated (via `UPDATE_GOLDEN=1`) if output format changed
- No other tests broken by the changes
- `cargo test --workspace --all-targets` passes

## Functional Requirements

- **FR-1:** `broadcast_wallet_run` shall sign transactions using the `WalletSigner` from the caller's `ResolvedSender` when `is_fork == false`, and continue using `anvil_impersonateAccount` when `is_fork == true`.
- **FR-2:** `broadcast_routable_txs` shall sign transactions using the `WalletSigner` resolved from the `from` address when `is_fork == false`, and continue using impersonation when `is_fork == true`.
- **FR-3:** Transaction signing shall use alloy's `TransactionRequest` → `send_transaction` flow (letting the provider fill nonce, gas, chain ID) rather than manually constructing raw transaction bytes.
- **FR-4:** `BroadcastReceipt` shall be populated from alloy's typed `TransactionReceipt` with the same fields as the current fork-mode path: `hash`, `block_number`, `gas_used`, `status`, `contract_address`.
- **FR-5:** `resolve_wallet_for_address` shall support all sender chain shapes: direct Wallet, Safe→Wallet, Governor→Wallet, Governor→Safe→Wallet.
- **FR-6:** The alloy provider shall be constructed once per broadcast call (not per transaction) to reuse the underlying HTTP connection.
- **FR-7:** Errors from the alloy provider (connection failures, reverts, nonce conflicts) shall be wrapped in `TrebError::Forge` with the RPC method context preserved.
- **FR-8:** All alloy crate versions shall resolve to v1.1.1 via the existing workspace `[patch.crates-io]` overrides — no new version pins required.

## Non-Goals

- **Not migrating all RPC calls:** Only `broadcast_wallet_run` and `broadcast_routable_txs` are migrated. The `AnvilRpc` helper in `fork_routing.rs`, `fetch_receipt`, and other raw reqwest calls are left for Phase 10.
- **Not removing `extract_signing_key`:** The function is still used by Safe signing (`sign_safe_tx`) and the propose flow. It will be replaced incrementally in later phases.
- **Not removing `reqwest` from treb-forge:** The fork-mode impersonation path and other RPC helpers still use reqwest. Full removal is Phase 10.
- **Not implementing non-interactive fork simulation:** The `handle_queued_executions` non-interactive behavior is Phase 2.
- **Not implementing broadcast file checkpointing:** Writing `ScriptSequence` during broadcast is Phase 3.
- **Not supporting hardware wallet live signing in tests:** Ledger/Trezor signers are structurally supported through `WalletSigner` but cannot be exercised in CI. Manual testing guidance is out of scope.
- **Not changing the Safe proposal or Governor proposal paths:** Only the wallet broadcast path and Safe(1/1) exec broadcast are affected. Multi-sig proposal signing stays on the existing `extract_signing_key` path.

## Technical Considerations

### Dependencies
- **alloy v1.1.1 pin:** The workspace `[patch.crates-io]` section already pins 31 alloy crates to v1.1.1 (required for foundry v1.5.1 compatibility with alloy-evm). The four new crates (`alloy-provider`, `alloy-consensus`, `alloy-network`, `alloy-rpc-types`) are already in the patch list — they just need to be declared as dependencies in `treb-forge/Cargo.toml`.
- **`EthereumWallet`:** Provided by `alloy-network`. Wraps any `alloy_signer::Signer` (which `WalletSigner` implements) into a wallet that can be attached to a provider.
- **`ProviderBuilder`:** From `alloy-provider`. Constructs a provider with `.wallet(ethereum_wallet)` so `send_transaction` automatically signs.

### Integration Points
- **`execute_plan` → broadcast functions:** The `RouteContext` struct already carries `resolved_senders`. The broadcast functions need access to this map; either pass `&RouteContext` or add `resolved_senders` to their parameter lists.
- **`BroadcastReceipt` mapping:** Alloy's `TransactionReceipt` has `transaction_hash`, `block_number`, `gas_used`, `status`, `contract_address` — direct field mapping to `BroadcastReceipt`.
- **Fork branch isolation:** The fork impersonation path uses Anvil-specific RPC methods (`anvil_impersonateAccount`, `anvil_stopImpersonatingAccount`) that don't exist on live nodes. The `is_fork` branch guard must remain in place.

### Constraints
- **No `extract_signing_key` for new code:** The new signing path must use `WalletSigner` from `ResolvedSender` directly. This ensures Ledger and Trezor signers work without requiring a raw private key string.
- **Provider construction cost:** An HTTP provider per broadcast call is acceptable. Per-transaction provider construction is not.
- **Test infrastructure:** The 14 ignored tests run against Anvil nodes spawned by `TestContext`. Live signing against Anvil works because Anvil accepts signed transactions (not just impersonated ones). No additional test infrastructure is needed.
