# PRD: Phase 8 - Mixed Sender & Edge Case Tests

## Introduction

Phase 8 adds integration tests for deployment scripts that use **multiple sender types** in a single run (e.g., wallet + governor, wallet + safe). It also covers routing error paths — depth limit enforcement, threshold query failure handling, and hardware wallet signer error messages. These tests exercise `partition_into_runs()` → `reduce_queue()` → `execute_plan()` with heterogeneous sender configurations, verifying that the routing pipeline correctly partitions, classifies, and executes transactions for each sender type independently within the same script invocation.

This phase depends on Phase 6 (Safe deployment) and Phase 7 (Governor deployment), reusing `deploy_safe()` and `deploy_governor()` e2e helpers for infrastructure setup.

## Goals

1. **Mixed sender correctness**: Verify that a single Forge script broadcasting from both a wallet address and a Safe/Governor address produces correctly partitioned transaction runs, with wallet transactions marked EXECUTED and Safe/Governor transactions routed through their respective pipelines.
2. **Routing error coverage**: Confirm that the `MAX_ROUTE_DEPTH=4` limit produces a clear error, that threshold queries against non-Safe addresses fail gracefully, and that hardware wallet signers produce descriptive error messages when unavailable.
3. **Registry consistency under mixed senders**: Assert that `deployments.json`, `transactions.json`, `safe-txs.json`, and `governor-txs.json` are all internally consistent when a single script produces records across multiple sender types.
4. **Re-enable remaining ignored tests**: Bring the ignored test count to the minimum possible before Phase 9 (fork infrastructure fixes).

## User Stories

### P8-US-001 — Create mixed-sender Forge deployment scripts

**Description**: Add two new Forge scripts to the test fixture project that broadcast from multiple sender addresses within a single `run()` function. These scripts are the fixtures for the mixed-sender integration tests in US-002 and US-003.

**Deliverables**:
- `tests/fixtures/project/script/DeployMixedWalletSafe.s.sol` — deploys one Counter via `vm.startBroadcast()` (default wallet sender) and another Counter via `vm.startBroadcast(safeAddress)` (Safe sender), emitting `ContractDeployed` and `TransactionSimulated` events for each.
- `tests/fixtures/project/script/DeployMixedWalletGovernor.s.sol` — deploys one Counter via `vm.startBroadcast()` (default wallet sender) and another Counter via `vm.startBroadcast(timelockAddress)` (Governor sender, using timelock as `broadcast_address()`), emitting events for each.

**Acceptance Criteria**:
- [ ] Both scripts compile with `forge build` in the fixture project
- [ ] `DeployMixedWalletSafe.s.sol` reads `SAFE_ADDRESS` from env, uses two separate `vm.startBroadcast()`/`vm.stopBroadcast()` blocks with different sender addresses
- [ ] `DeployMixedWalletGovernor.s.sol` reads `TIMELOCK_ADDRESS` from env, uses two separate broadcast blocks with the default sender and the timelock address
- [ ] Each script emits distinct `ContractDeployed` events with different labels (e.g., "WalletCounter" / "SafeCounter") so registry assertions can distinguish them
- [ ] Scripts follow the existing `DeployViaSafe.s.sol` / `DeployViaGovernor.s.sol` patterns (same struct definitions, event signatures)
- [ ] `cargo clippy --workspace --all-targets` passes

---

### P8-US-002 — Integration test: mixed wallet + Safe(1/1) senders on fork

**Description**: Write an e2e test that deploys a Safe(1/1), configures `treb.toml` with both a wallet sender and the Safe sender, runs `DeployMixedWalletSafe.s.sol` through `treb run --broadcast --non-interactive`, and verifies that:
- `partition_into_runs()` produces separate runs for the wallet and Safe addresses
- Wallet transactions have status EXECUTED
- Safe(1/1) transactions have status EXECUTED (1/1 executes directly via `execTransaction`)
- Both deployment records appear in `deployments.json` with correct sender addresses

**Acceptance Criteria**:
- [ ] Test file: `crates/treb-cli/tests/e2e_mixed_sender.rs` (new file, shared by US-002 and US-003)
- [ ] Test function: `mixed_wallet_safe_broadcast_on_fork`
- [ ] Uses `spawn_anvil_or_skip()`, `deploy_safe()` (from `e2e/deploy_safe.rs`), `tokio::task::spawn_blocking`
- [ ] Configures `treb.toml` with `[accounts.deployer]` (wallet, private_key) and `[accounts.safe_deployer]` (safe, signer = "deployer") in `[namespace.default.senders]`
- [ ] Runs `treb init` → `treb fork enter` → `treb run script/DeployMixedWalletSafe.s.sol --broadcast --non-interactive`
- [ ] Asserts `deployments.json` has 2 entries with distinct `contractName` values
- [ ] Asserts all entries in `transactions.json` have `status: "EXECUTED"`
- [ ] Asserts wallet-sender transaction's `sender` field (case-insensitive) matches the wallet address
- [ ] Asserts safe-sender transaction's `sender` field (case-insensitive) matches the Safe proxy address
- [ ] Calls `assert_registry_consistent()` from `e2e/mod.rs`
- [ ] `cargo test -p treb-cli --test e2e_mixed_sender` passes
- [ ] `cargo clippy --workspace --all-targets` passes

---

### P8-US-003 — Integration test: mixed wallet + Governor senders on fork

**Description**: Write an e2e test that deploys a Governor stack, configures `treb.toml` with both a wallet sender and the Governor sender, runs `DeployMixedWalletGovernor.s.sol` through `treb run --broadcast --non-interactive`, and verifies that:
- Wallet transactions have status EXECUTED
- Governor transactions have status QUEUED with a `GovernorProposed` record in `governor-txs.json`
- Both deployment records appear in `deployments.json`

**Acceptance Criteria**:
- [ ] Test function: `mixed_wallet_governor_on_fork` in `e2e_mixed_sender.rs`
- [ ] Uses `spawn_anvil_or_skip()`, `deploy_safe()` (for the proposer), `deploy_governor()`, `tokio::task::spawn_blocking`
- [ ] Configures `treb.toml` with `[accounts.deployer]` (wallet), `[accounts.governance]` (governance, with timelock + proposer = "deployer"), and both in `[namespace.default.senders]`
- [ ] Runs `treb init` → `treb fork enter` → `treb run script/DeployMixedWalletGovernor.s.sol --broadcast --non-interactive`
- [ ] Asserts `deployments.json` has 2 entries
- [ ] Asserts wallet-sender transaction status is `"EXECUTED"`
- [ ] Asserts governor-sender transaction status is `"QUEUED"` and `sender` (case-insensitive) matches the timelock address
- [ ] Asserts `governor-txs.json` has 1 entry with `governorAddress`, `timelockAddress`, and `forkExecutedAt` set
- [ ] Asserts `governor-txs.json` entry's `transactionIds` array links to the QUEUED transactions
- [ ] Calls `assert_registry_consistent()`
- [ ] `cargo test -p treb-cli --test e2e_mixed_sender` passes
- [ ] `cargo clippy --workspace --all-targets` passes

---

### P8-US-004 — Integration tests: routing error paths and edge cases

**Description**: Add focused tests for routing error conditions that don't require full e2e workflow setup. These test the error messages users see when sender configuration is invalid or infrastructure is missing.

**Deliverables**:
- **Governor depth limit test**: Configure a `treb.toml` with a chain of 5 governor senders (Gov-A → Gov-B → Gov-C → Gov-D → Gov-E → Wallet), run `treb run --broadcast --non-interactive`, and assert the error contains `"routing queue depth exceeded (4)"`.
- **Threshold query failure test**: On fork, run a script that broadcasts from an address configured as a Safe sender but where no Safe contract is actually deployed at that address. Assert the error contains `"decode threshold"` (the `eth_call` to `getThreshold()` returns garbage or reverts).
- **Hardware wallet signer error test**: Configure a `treb.toml` with `type = "ledger"` sender, attempt `treb run --broadcast --non-interactive` (non-fork, to trigger live signing path), and assert the error contains `"failed to connect to Ledger device"` or `"live signing is not yet supported for hardware wallet signers"`.

**Acceptance Criteria**:
- [ ] Test file: `crates/treb-cli/tests/e2e_routing_errors.rs` (new file)
- [ ] Test `governor_depth_limit_error`: configures 5-level governor chain in `treb.toml`, asserts CLI exits with non-zero status and stderr contains `"routing queue depth exceeded (4)"` or `"check sender configuration for circular references"`
- [ ] Test `threshold_query_failure_on_fork`: deploys no Safe contract, configures `treb.toml` with Safe sender pointing to an EOA address, asserts CLI exits with non-zero status and stderr contains a threshold-related error
- [ ] Test `hardware_wallet_signer_error`: configures `type = "ledger"` in `treb.toml`, asserts CLI exits with non-zero status and stderr contains `"Ledger"` (the exact message depends on whether sender resolution or live-signing-path triggers first)
- [ ] All three tests use `spawn_anvil_or_skip()` or equivalent setup as needed
- [ ] `cargo test -p treb-cli --test e2e_routing_errors` passes
- [ ] `cargo clippy --workspace --all-targets` passes

---

### P8-US-005 — Re-enable remaining ignored tests and verify test inventory

**Description**: Audit all `#[ignore]` annotations in the test suite. Re-enable any tests that now pass after Phases 1–8. For tests that remain ignored (expected: 7 tests destined for Phase 9), verify the ignore reason is documented and accurate.

**Known remaining ignored tests** (7 total, all Phase 9 scope):
- `fixture_compile.rs` — `fixture_forge_build` (requires `forge` installed — re-enable if forge is available in the test environment)
- `cli_version.rs` — `version_json_uses_git_describe_output_in_untagged_checkouts` (fails with uncommitted changes)
- `integration_fork.rs` — `fork_status_with_active_fork`, `fork_revert_port_unreachable`, `fork_restart_port_unreachable` (golden refresh needed)
- `cli_compatibility_aliases.rs` — `bare_config_matches_config_show`, `fork_history_network_filter` (output format changed)

**Acceptance Criteria**:
- [ ] Run `cargo test --workspace --all-targets 2>&1 | grep -c "ignored"` and document the count
- [ ] `fixture_forge_build` is re-enabled if `forge` is available (it should be, since Phases 6–7 use `forge script` in tests)
- [ ] No test outside the Phase 9 list above remains ignored without a documented reason
- [ ] Each remaining `#[ignore]` has an inline comment explaining which phase will address it
- [ ] `cargo test --workspace --all-targets` passes (all non-ignored tests green)
- [ ] `cargo clippy --workspace --all-targets` passes

## Functional Requirements

- **FR-1**: Mixed-sender Forge scripts must use separate `vm.startBroadcast(addr)` / `vm.stopBroadcast()` blocks for each sender address, matching how `partition_into_runs()` detects sender transitions by `from` address changes.
- **FR-2**: `partition_into_runs()` must produce at least 2 `TransactionRun` entries for mixed-sender scripts — one per distinct `broadcast_address()`.
- **FR-3**: Mixed wallet + Safe(1/1) tests must verify both sender types produce `status: "EXECUTED"` in `transactions.json` (Safe 1/1 executes directly via `execTransaction` on fork).
- **FR-4**: Mixed wallet + Governor tests must verify wallet transactions get `status: "EXECUTED"` while governor-routed transactions get `status: "QUEUED"`, with a corresponding entry in `governor-txs.json`.
- **FR-5**: Governor depth limit must trigger `TrebError::Forge` with message `"routing queue depth exceeded (4); check sender configuration for circular references"` when a sender chain exceeds `MAX_ROUTE_DEPTH=4` levels.
- **FR-6**: Safe threshold query against a non-Safe address must produce a decode error (not a panic or hang).
- **FR-7**: Hardware wallet signer configuration must produce a user-facing error message mentioning the device type (Ledger/Trezor) when the device is unavailable.
- **FR-8**: All mixed-sender tests must use `eq_ignore_ascii_case` (or equivalent) for address comparisons, per Phase 7 learnings about `to_checksum()` vs `format!("{:#x}")` case mismatch.

## Non-Goals

- **No new routing logic**: Phase 8 is test-only. No changes to `partition_into_runs()`, `reduce_queue()`, `execute_plan()`, or `route_all()`.
- **No Safe(n/m) mixed tests**: Multi-sig Safe (threshold > 1) mixed with wallet is out of scope — Phase 6 already covers Safe(2/3) proposal in isolation.
- **No live network tests**: All mixed-sender tests run on Anvil fork. Live signing mixed-sender tests are out of scope.
- **No Phase 9 fork fixes**: The 6 fork/alias ignored tests are explicitly Phase 9 scope.
- **No Governor → Safe → Governor circular chains**: Only testing the depth limit, not arbitrary circular sender topologies beyond what triggers `MAX_ROUTE_DEPTH`.
- **No behavioral changes to the CLI**: Only new test files and fixture scripts are added.

## Technical Considerations

### Dependencies
- **Phase 7 (Governor stack)**: `deploy_governor()` helper and governance contract stubs must be merged.
- **Phase 6 (Safe contracts)**: `deploy_safe()` helper and Safe contract stubs must be merged.
- **Phase 2 (`--skip-fork-execution`)**: Available but not required — mixed tests use fork simulation, not skip.

### Infrastructure Reuse
- Reuse `e2e/deploy_safe.rs::deploy_safe()` → returns `DeployedSafe { proxy_address, singleton_address, factory_address, multisend_address, owners, threshold }`.
- Reuse `e2e/deploy_governor.rs::deploy_governor()` → returns `DeployedGovernor { governor_address, timelock_address, token_address, timelock_delay }`.
- Reuse `e2e/mod.rs::spawn_anvil_or_skip()`, `read_registry_file()`, `assert_registry_consistent()`.

### Sender Configuration Constraints
- All accounts in a sender chain must appear in `[namespace.default.senders]` — e.g., for wallet + governor, the map needs entries for both the wallet role AND the governance role (and the governance entry's `proposer` must also be in the map).
- Governor + Timelock senders use the **timelock address** as the `broadcast_address()`, not the governor — so Forge scripts must `vm.startBroadcast(timelockAddress)`.
- Address comparisons across registry files must be case-insensitive (checksum vs lowercase).

### Test Execution
- All e2e tests use `#[tokio::test(flavor = "multi_thread")]` + `tokio::task::spawn_blocking` for blocking CLI subprocess calls.
- Tests that need Anvil use `spawn_anvil_or_skip()` and return early if Anvil is unavailable.
- CreateCall helper is already deployed at `0xCCCC...01` in `fork_routing.rs` for CREATE-through-Safe/Governor.

### Governor Depth Limit Test Design
- The depth limit test does **not** need Anvil or actual contract deployment — it only needs a `treb.toml` with a chain of governor senders where each governor's `proposer` points to the next governor. The error triggers during `reduce_queue()` before any on-chain interaction.
- Configuration: Gov-A (proposer: Gov-B) → Gov-B (proposer: Gov-C) → Gov-C (proposer: Gov-D) → Gov-D (proposer: Gov-E) → Gov-E (proposer: wallet). Depth 0→1→2→3→4 hits the limit at depth 4.
