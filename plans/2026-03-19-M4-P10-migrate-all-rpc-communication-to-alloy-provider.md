# PRD: Phase 10 — Migrate All RPC Communication to Alloy Provider

## Introduction

Phase 10 replaces every remaining raw `reqwest` JSON-RPC call in the treb workspace with `alloy-provider`. Phase 1 migrated live-mode wallet broadcasts to alloy, but fork-mode broadcasting, Anvil manipulation, governor polling, receipt fetching, compose replay, fork management commands, and auto-funding still use hand-rolled `reqwest::Client` + `serde_json::json!()` RPC envelopes. This phase eliminates that duplication by introducing a shared provider construction helper and converting all call sites to use alloy's typed `Provider` trait and `AnvilApi` extension trait, giving the codebase a single, consistent RPC abstraction.

After Phase 10, `reqwest` remains only where it is used for HTTP REST calls (e.g., `treb-safe` Safe Transaction Service client) and synchronous health polling (`ureq` in `anvil.rs`). All JSON-RPC communication goes through alloy.

## Goals

1. **Zero raw JSON-RPC envelopes** — no remaining `serde_json::json!({"jsonrpc":"2.0", ...})` patterns in production code across the workspace.
2. **Single provider construction pattern** — a shared helper module so every call site builds providers the same way, replacing the duplicated inline `ProviderBuilder` calls from Phase 1 and the `AnvilRpc` struct from fork routing.
3. **Full Anvil API coverage via alloy** — all `anvil_*` and `evm_*` RPC methods use `alloy_provider::ext::AnvilApi` instead of manual JSON-RPC.
4. **Existing test suite stays green** — `cargo test --workspace --all-targets` and `cargo clippy --workspace --all-targets` pass with no regressions.
5. **Remove `reqwest` from `treb-forge`** — if all JSON-RPC usage is replaced, drop the `reqwest` dependency from `treb-forge/Cargo.toml` (keep it in workspace for `treb-safe` and `treb-cli`).

## User Stories

### P10-US-001 — Create shared provider construction module

**Problem:** Phase 1 duplicates the `ProviderBuilder::new().wallet(w).connect_http(url)` pattern in two broadcast functions. Fork-mode code uses `AnvilRpc` (a bespoke reqwest wrapper). There is no single place to construct a provider from an RPC URL.

**Solution:** Create `crates/treb-forge/src/provider.rs` with two public helpers:

- `build_http_provider(rpc_url: &str) -> Result<impl Provider, TrebError>` — for read-only and fork-mode use (no wallet).
- `build_wallet_provider(rpc_url: &str, wallet: EthereumWallet) -> Result<impl Provider, TrebError>` — for live-mode signed broadcasts.

Refactor the two existing live broadcast functions (`broadcast_wallet_run_live`, `broadcast_routable_txs_live` in `routing.rs`) to use `build_wallet_provider` instead of inline construction.

**Acceptance Criteria:**
- `crates/treb-forge/src/provider.rs` exists with both helpers
- `provider.rs` is added to `crates/treb-forge/src/lib.rs` module tree
- `broadcast_wallet_run_live()` and `broadcast_routable_txs_live()` call `build_wallet_provider()` — no inline `ProviderBuilder` remains in those functions
- Both helpers parse the URL string and return `TrebError::Forge` on invalid URLs
- `cargo test --workspace --all-targets` passes
- `cargo clippy --workspace --all-targets` passes

---

### P10-US-002 — Replace `AnvilRpc` in fork_routing.rs with alloy provider

**Problem:** `AnvilRpc` in `fork_routing.rs` is a 170-line reqwest wrapper with 11 hand-rolled JSON-RPC methods (`eth_call`, `eth_sendTransaction`, `eth_getTransactionReceipt`, `eth_estimateGas`, `eth_getBlockByNumber`, `anvil_setBalance`, `anvil_impersonateAccount`, `anvil_stopImpersonatingAccount`, `anvil_setCode`, `evm_increaseTime`, `anvil_mine`) plus a generic `rpc_call` dispatcher. All of this is available through alloy's `Provider` trait and `AnvilApi` extension trait.

**Solution:** Replace the `AnvilRpc` struct with a type alias or thin wrapper around an alloy `Provider` instance constructed via `build_http_provider()` from US-001. Migrate each method:

| Old `AnvilRpc` method | Alloy replacement |
|---|---|
| `eth_call(to, data, from)` | `provider.call(tx_req)` |
| `impersonate_send_tx(from, to, data, value)` | `provider.anvil_impersonate_account(from)` + `provider.anvil_set_balance(from, ...)` + `provider.send_transaction(tx_req)` + `provider.get_transaction_receipt(hash)` + `provider.anvil_stop_impersonating_account(from)` |
| `set_balance(addr, balance)` | `provider.anvil_set_balance(addr, balance)` |
| `impersonate(addr)` | `provider.anvil_impersonate_account(addr)` |
| `stop_impersonating(addr)` | `provider.anvil_stop_impersonating_account(addr)` |
| `set_code(addr, code)` | `provider.anvil_set_code(addr, code)` |
| `increase_time(secs)` | `provider.evm_increase_time(secs)` |
| `mine_blocks(count)` | `provider.anvil_mine(Some(count), None)` |
| `estimate_gas(from, to, data)` | `provider.estimate_gas(tx_req)` |
| `block_gas_limit()` | `provider.get_block_by_number(...)` → extract gas_limit |
| `get_receipt(hash)` | `provider.get_transaction_receipt(hash)` |
| `rpc_call(method, params)` | Removed — no longer needed |

Update all callers in `fork_routing.rs` (Safe execution, Governor execution, CreateCall deployment, timelock simulation) to use the new provider-based methods. Map alloy receipt types to the existing `BroadcastReceipt` struct to keep downstream code unchanged.

**Acceptance Criteria:**
- `AnvilRpc` struct and its `rpc_call` method are removed from `fork_routing.rs`
- All 11 RPC methods replaced with alloy provider calls
- `impersonate_send_tx` composite method preserved as a helper function that orchestrates alloy calls in the same sequence (set_balance → impersonate → send → receipt → stop_impersonate)
- `BroadcastReceipt` mapping from alloy `TransactionReceipt` is consistent with the existing mapping in live broadcast code
- All public entry points (`exec_safe_from_registry`, `exec_governance_from_registry`, `query_safe_threshold`, `query_safe_nonce`, `simulate_governance_on_fork`) continue to work
- `cargo test --workspace --all-targets` passes
- `cargo clippy --workspace --all-targets` passes

---

### P10-US-003 — Replace fork-mode broadcast RPC calls in routing.rs

**Problem:** Four functions in `routing.rs` use raw reqwest JSON-RPC for fork-mode operations:
- `broadcast_wallet_run_fork()` — `anvil_impersonateAccount` + `eth_sendTransaction` + `eth_getTransactionReceipt` + `anvil_stopImpersonatingAccount`
- `broadcast_routable_txs_fork()` — same pattern for routable transaction batches
- `fetch_receipt()` — standalone `eth_getTransactionReceipt` with manual hex parsing
- `poll_pending_receipt()` — retry loop calling `eth_getTransactionReceipt`

**Solution:** Replace all four functions with alloy provider calls:
- `broadcast_wallet_run_fork` / `broadcast_routable_txs_fork`: use `build_http_provider()`, then `provider.anvil_impersonate_account()` + `provider.send_transaction()` + `provider.get_transaction_receipt()` + `provider.anvil_stop_impersonating_account()`
- `fetch_receipt`: use `provider.get_transaction_receipt(hash)` — remove 70+ lines of manual hex field parsing
- `poll_pending_receipt`: same alloy receipt call in a retry loop

Thread the provider instance through `RouteContext` or construct it at the call site from `ctx.rpc_url` using `build_http_provider()`. Unify the `BroadcastReceipt` mapping in one place (shared with US-002's mapping).

**Acceptance Criteria:**
- `broadcast_wallet_run_fork()` uses alloy provider — no `serde_json::json!()` RPC envelopes
- `broadcast_routable_txs_fork()` uses alloy provider — no `serde_json::json!()` RPC envelopes
- `fetch_receipt()` uses `provider.get_transaction_receipt()` — manual hex parsing removed
- `poll_pending_receipt()` uses `provider.get_transaction_receipt()` — manual hex parsing removed
- The `reqwest::Client` parameter is removed from `fetch_receipt` and `poll_pending_receipt` signatures (replaced with provider or rpc_url)
- `BroadcastReceipt` mapping from alloy receipt is extracted into a shared helper (used by both US-002 and US-003)
- `cargo test --workspace --all-targets` passes
- `cargo clippy --workspace --all-targets` passes

---

### P10-US-004 — Replace RPC calls in fund.rs, governor.rs, broadcast_writer.rs, and compose.rs

**Problem:** Four additional files in `treb-forge` contain isolated raw reqwest JSON-RPC calls:
- `fund.rs` — `fund_senders_on_fork()` uses `anvil_setBalance` via reqwest
- `governor.rs` — `query_proposal_state()` uses `eth_call` via reqwest to poll governor contract state
- `pipeline/broadcast_writer.rs` — `poll_receipt_exists()` uses `eth_getTransactionReceipt` via reqwest
- `pipeline/compose.rs` — `replay_transactions_on_fork()` uses `anvil_impersonateAccount` + `eth_sendTransaction` + `anvil_stopImpersonatingAccount` via reqwest

**Solution:** Migrate each call site to use alloy provider:
- `fund_senders_on_fork`: construct provider via `build_http_provider()`, call `provider.anvil_set_balance()` for each sender
- `query_proposal_state`: construct provider, use `provider.call()` with the Governor `state(proposalId)` calldata — preserve the detailed error handling and `ProposalStatus` enum mapping
- `poll_receipt_exists`: use `provider.get_transaction_receipt()` in the polling loop
- `replay_transactions_on_fork`: use `provider.anvil_impersonate_account()` + `provider.send_transaction()` + `provider.anvil_stop_impersonating_account()`

Update function signatures: replace `reqwest::Client` parameters with an rpc_url string (provider constructed internally) or accept a provider reference where construction cost matters. The `governor.rs` error formatting helpers (`format_rpc_error`, `rpc_error_detail`, `rpc_error_indicates_revert`) should be adapted to handle alloy error types instead of raw JSON.

**Acceptance Criteria:**
- `fund_senders_on_fork()` uses alloy provider — no `serde_json::json!()` RPC envelope
- `query_proposal_state()` uses alloy provider `call()` — no raw reqwest
- `poll_receipt_exists()` uses alloy provider `get_transaction_receipt()` — no raw reqwest
- `replay_transactions_on_fork()` uses alloy provider — no `serde_json::json!()` RPC envelopes
- `query_proposal_state()` error handling preserves existing `format_rpc_error` / revert detection behavior adapted to alloy error types
- All callers of these functions are updated if signatures changed
- `cargo test --workspace --all-targets` passes
- `cargo clippy --workspace --all-targets` passes

---

### P10-US-005 — Replace RPC calls in treb-cli fork.rs

**Problem:** `crates/treb-cli/src/commands/fork.rs` contains five functions that make raw reqwest JSON-RPC calls for fork management operations:
- `fetch_chain_id()` — `eth_chainId` with a 10-second timeout
- `json_rpc_call()` — generic reqwest JSON-RPC dispatcher (similar to `AnvilRpc::rpc_call`)
- `evm_snapshot_http()` — `evm_snapshot`
- `evm_revert_http()` — `evm_revert`
- `deploy_createx_http()` — `eth_getCode` + `anvil_setCode`

**Solution:** Add `alloy-provider` as a dependency of `treb-cli` (or reuse through `treb-forge`). Replace each function:
- `fetch_chain_id`: use `provider.get_chain_id()` — preserve the 10-second timeout via provider transport configuration or a `tokio::time::timeout` wrapper
- `evm_snapshot_http`: use `provider.evm_snapshot()`
- `evm_revert_http`: use `provider.evm_revert(snapshot_id)`
- `deploy_createx_http`: use `provider.get_code_at(address)` + `provider.anvil_set_code(address, code)`
- `json_rpc_call`: remove entirely — all callers now use typed provider methods

**Acceptance Criteria:**
- `fetch_chain_id()` uses alloy provider with timeout — no raw reqwest
- `evm_snapshot_http()` uses alloy provider — no raw reqwest
- `evm_revert_http()` uses alloy provider — no raw reqwest
- `deploy_createx_http()` uses alloy provider — no raw reqwest
- `json_rpc_call()` helper is removed (no callers remain)
- The 10-second timeout on `fetch_chain_id` is preserved (via `tokio::time::timeout` or provider configuration)
- `cargo test --workspace --all-targets` passes
- `cargo clippy --workspace --all-targets` passes

---

### P10-US-006 — Remove reqwest from treb-forge and verify migration completeness

**Problem:** After US-001 through US-005, `treb-forge` should have no remaining raw reqwest JSON-RPC calls. The `reqwest` dependency in `treb-forge/Cargo.toml` may still be needed for URL parsing (alloy's `connect_http` takes a `reqwest::Url`) or may be fully replaceable with `url::Url`. Additionally, the codebase should be audited for any stray raw RPC calls missed in previous stories.

**Solution:**
1. Audit the full workspace for any remaining `serde_json::json!({"jsonrpc"` patterns in production code (exclude tests and test fixtures)
2. If `treb-forge` no longer imports `reqwest` directly, remove it from `crates/treb-forge/Cargo.toml`
3. If `reqwest::Url` is still used for `connect_http`, either keep `reqwest` or switch to `url::Url` if alloy accepts it
4. Verify `treb-cli`'s `reqwest` usage — it may still need reqwest for the fork.rs timeout builder or the `reqwest::Url` type; if all fork.rs calls are migrated, check if reqwest can be removed from `treb-cli` too
5. Ensure `treb-safe` retains `reqwest` (it uses REST, not JSON-RPC)
6. Verify the ureq-based synchronous health polling in `anvil.rs` is intentionally kept (it runs in a blocking context where async is inappropriate)

**Acceptance Criteria:**
- `grep -r 'serde_json::json.*jsonrpc' crates/` returns zero matches in production code (test code excluded)
- `reqwest` is removed from `crates/treb-forge/Cargo.toml` if no longer imported, OR a comment documents why it's kept (e.g., URL type reuse)
- `crates/treb-forge/src/anvil.rs` ureq usage is preserved (documented as intentional: synchronous blocking health check)
- `crates/treb-safe/Cargo.toml` retains `reqwest` for REST API calls
- `cargo test --workspace --all-targets` passes
- `cargo clippy --workspace --all-targets` passes
- `cargo build --workspace` produces no unused-dependency warnings related to reqwest

## Functional Requirements

- **FR-1:** All JSON-RPC communication in `treb-forge` uses `alloy-provider` — no manual JSON-RPC envelope construction.
- **FR-2:** All Anvil-specific RPC methods (`anvil_setBalance`, `anvil_impersonateAccount`, `anvil_stopImpersonatingAccount`, `anvil_setCode`, `anvil_mine`, `evm_increaseTime`, `evm_snapshot`, `evm_revert`) use `alloy_provider::ext::AnvilApi`.
- **FR-3:** All standard Ethereum RPC methods (`eth_call`, `eth_sendTransaction`, `eth_getTransactionReceipt`, `eth_estimateGas`, `eth_getBlockByNumber`, `eth_chainId`, `eth_getCode`) use typed `Provider` trait methods.
- **FR-4:** Provider construction is centralized in `treb-forge/src/provider.rs` with `build_http_provider` and `build_wallet_provider` helpers.
- **FR-5:** `BroadcastReceipt` mapping from alloy's `TransactionReceipt` is a shared helper used by both fork-mode and live-mode code paths.
- **FR-6:** Fork-mode `impersonate_send_tx` composite operation preserves the exact sequence: set_balance → impersonate → send → get_receipt → stop_impersonate.
- **FR-7:** `query_proposal_state` in `governor.rs` preserves detailed error handling (revert detection, HTTP status checks) adapted to alloy error types.
- **FR-8:** `fetch_chain_id` in `fork.rs` preserves the 10-second timeout behavior.
- **FR-9:** `reqwest` is removed from `treb-forge/Cargo.toml` if no longer used; kept with a comment if still needed for URL type.
- **FR-10:** Synchronous `ureq`-based health polling in `anvil.rs` is explicitly preserved (not migrated).

## Non-Goals

- **No behavior changes** — every RPC call site produces the same observable behavior (same requests, same error messages modulo transport-level details, same return values). This is a pure infrastructure migration.
- **No new RPC capabilities** — this phase only replaces existing calls, it does not add new RPC methods or endpoints.
- **No treb-safe migration** — the Safe Transaction Service client uses REST HTTP, not JSON-RPC. It stays on reqwest.
- **No anvil.rs health check migration** — the `ureq`-based synchronous polling runs in a blocking thread; migrating it to async alloy would require restructuring the health check loop, which is out of scope.
- **No test re-enablement** — this phase re-enables 0 ignored tests. All test fixes are in earlier phases.
- **No RouteContext refactoring** — the `rpc_url: &str` field in `RouteContext` stays as-is. Providers are constructed from it at call sites rather than caching a provider in the context (avoids lifetime complexity).

## Technical Considerations

**Dependencies:**
- Phase 1 must be merged first — it introduced the `alloy-provider`, `alloy-consensus`, `alloy-network`, and `alloy-rpc-types` dependencies and established the live-mode broadcast pattern.
- Alloy crates are pinned to v1.1.1 via workspace `[patch.crates-io]` to match foundry v1.5.1 — do not upgrade.

**Alloy `AnvilApi` availability:**
- The `alloy_provider::ext::AnvilApi` trait provides typed methods for all `anvil_*` and `evm_*` calls used in the codebase. Verify that `evm_snapshot`, `evm_revert`, `evm_increase_time`, and `anvil_mine` are available in v1.1.1; if any are missing, use `provider.raw_request()` as a fallback for that specific method.

**Provider type erasure:**
- `alloy_provider::ProviderBuilder::new().connect_http(url)` returns a concrete type with generics. The `provider.rs` helpers should return `impl Provider` or box-erase if the concrete type causes ergonomic issues in function signatures.

**Receipt type mapping:**
- Alloy's `TransactionReceipt` has typed fields (`block_number: Option<u64>`, `gas_used: u128`, `status: bool`, `contract_address: Option<Address>`). The existing `BroadcastReceipt` struct expects `u64` gas, `bool` status, `Option<Address>` contract address. A `From<TransactionReceipt>` impl or mapping function should handle the conversion once, shared across fork and live paths.

**Error mapping:**
- Alloy provider errors are richer than raw reqwest errors. Map `alloy_provider` transport/RPC errors to `TrebError::Forge(String)` with the same granularity as the existing error messages (e.g., `"eth_call error: ..."`, `"send tx failed: ..."`).
- `governor.rs` has specialized error handling (`rpc_error_indicates_revert`) that inspects JSON error data for revert markers. With alloy, RPC errors may surface as `alloy_json_rpc::RpcError` variants — adapt the revert detection to match alloy's error structure.

**Timeout handling:**
- `fetch_chain_id` in `fork.rs` uses `reqwest::Client::builder().timeout(Duration::from_secs(10))`. With alloy, wrap the provider call in `tokio::time::timeout(Duration::from_secs(10), provider.get_chain_id())` to preserve the same behavior.

**Integration with treb-cli:**
- `treb-cli/src/commands/fork.rs` needs alloy-provider access. Either add `alloy-provider` directly to `treb-cli/Cargo.toml`, or expose the needed fork helpers (snapshot, revert, deploy_createx, chain_id) from `treb-forge` and call them from `treb-cli`. The latter keeps RPC abstraction in one crate.

**Risk: alloy v1.1.1 API surface gaps:**
- If any Anvil/EVM method is not available in alloy v1.1.1's `AnvilApi`, use `provider.raw_request::<_, serde_json::Value>("method_name", params)` as a typed escape hatch. This is still better than raw reqwest because it reuses the provider's transport and error handling.
