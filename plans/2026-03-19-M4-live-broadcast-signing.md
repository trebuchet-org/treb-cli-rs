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

**Notes from Phase 10:**
- Provider helpers already exist at `crates/treb-forge/src/provider.rs`: use `build_http_provider(url)` for read-only and `build_wallet_provider(url, wallet)` for signing — do NOT add new `ProviderBuilder` inline calls
- `reqwest` is fully removed from `treb-forge` — all RPC in treb-forge now goes through alloy provider. Phase 1 signing should build on this foundation
- `EthereumWallet` comes from `alloy_network`, not `alloy_provider` — already imported in provider.rs
- Fork broadcast functions already use `provider.send_transaction().get_receipt()` pattern — live signing branch should mirror this with wallet-signed transactions
- `query_proposal_state()` signature changed: takes only `rpc_url` (no `reqwest::Client`) — callers in run.rs already updated
- `broadcast_wallet_run_live()` and `broadcast_routable_txs_live()` already use `build_wallet_provider()` — Phase 1 adds the signing logic within these existing provider-based functions
- `receipt_to_broadcast_receipt()` is shared between fork and live paths — Phase 1 should continue using it for receipt conversion
- `reqwest` remains in `treb-cli` (run.rs, prune.rs, dev.rs, networks.rs, register.rs) — Phase 1 may encounter reqwest usage in CLI-level code that is separate from the forge RPC migration

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
- Re-enable: compose tests that depend on broadcast file state (subset of 7 remaining ignored `integration_compose` tests — Phase 5 already re-enabled 6 and deleted 2)

**User stories:** 6
**Dependencies:** Phase 1

**Notes from Phase 10:**
- Receipt polling in `routing.rs` now uses `provider.get_transaction_receipt()` returning `Ok(Some(_))` / `Ok(None)` — resume code should use this same pattern for polling pending hashes
- `poll_pending_receipt()` already takes `&impl Provider` and uses the new alloy receipt polling — checkpoint resume can reuse this function directly
- `fetch_receipt()` wraps `Ok(None)` as `Err` for the retry loop pattern — resume logic should follow the same convention

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

**Notes from Phase 10:**
- Any mock JSON-RPC servers in e2e tests must include `Connection: close` header and complete alloy-compatible receipt JSON (transactionIndex, from, to, cumulativeGasUsed, effectiveGasPrice, logs, logsBloom, type) — alloy's deserializer rejects partial receipts
- CLI-level provider usage: `alloy-provider` and `alloy-primitives` are now workspace deps in `crates/treb-cli/Cargo.toml` — e2e tests can use `treb_forge::provider::build_http_provider()` directly

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

**Learnings from implementation:**
- Phase 5 was partially completable without Phase 1: 12 `cli_compose` + 6 `integration_compose` golden tests re-enabled (18 total), 2 `integration_compose` dump-command tests deleted. 7 `integration_compose` tests remain ignored (broadcast-dependent, still need Phase 1)
- Simulation mode (no `--broadcast`) bypasses config/init resolution entirely — tests that previously relied on compose failing without `foundry.toml`/init now need `--broadcast --network localhost --non-interactive` to test the execution path
- `dry_run` parameter was fully removed from compose's `run()`, `should_prompt_for_broadcast_confirmation()`, `should_reject_interactive_json_broadcast()`, `print_compose_banner()`, and shared `deployment_banner_mode()` in run.rs
- `--dump-command` code block (~48 lines) and its 2 golden test directories were deleted entirely
- compose.rs and run.rs each have their own `should_prompt_for_broadcast_confirmation()` and `should_reject_interactive_json_broadcast()` — they're separate functions, not shared. But `deployment_banner_mode()` in run.rs IS shared (called via `super::run::`)
- Spinner cleanup `eprint!("\x1b[2K\r")` in compose.rs was running unconditionally — must guard with `if !json` to avoid polluting JSON stderr output. Both simulation (~line 1074) and broadcast (~line 1368) phases needed this fix
- `strip_ansi` in test helpers must handle `\x1b[2K` (clear-line) and `\r` (carriage return), not just `\x1b[N;Nm` color codes — future test work should use the updated regex
- `simulate_all` returns empty `failed_name` for compilation-phase errors because compilation is project-wide, not component-specific
- Banner plan format uses `[N] name → script` (bracketed), simulation execution plan uses `N. name → script` (dotted) — different renderers
- Golden tests with `expect_err(true)` panic with "Unexpected success" before golden comparison runs, so the golden file won't be refreshed with `UPDATE_GOLDEN=1` until `expect_err` is fixed first
- `inject_env_vars` is called at compose.rs:903 BEFORE the setup loop — env validation errors bypass the component failure renderer entirely

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

**Learnings from implementation:**
- Two pipeline bugs were fixed that affect all routing paths (Phase 7/8 benefit):
  1. `wants_broadcast` in `execute_script()` previously excluded Safe/Governor senders — removed the exclusion since `ScriptConfig.broadcast=false` already prevents forge's direct broadcast
  2. `apply_receipts` must run BEFORE `update_transaction_statuses_from_routing` in orchestrator — otherwise placeholder receipts overwrite Queued status with Executed
- CREATE transactions through Safe require DelegateCall to a CreateCall helper — CreateCall runtime bytecode is deployed at `0xCCCC...01` via `anvil_setCode` in `fork_routing.rs`. Governor CREATE txs will also need this path.
- `build_btxs_from_routable` always sets a `to` field (CALL), losing CREATE info — original btxs must be passed alongside reconstructed ones for CREATE detection
- Fork propose path must use pre-computed EIP-712 hash from `QueuedExecution::SafeProposal`, not `B256::random()` — was generating random hashes before this fix
- Safe sender treb.toml config: `signer` field references a **role key** in the namespace senders map, not an account name; both the Safe and its signer wallet must appear in `[namespace.default.senders]`
- Forge broadcast artifacts: CREATE addresses in `transactions[].contractAddress`, factory-created (CREATE2) in `transactions[].additionalContracts[].address` — both must be checked
- `cast call <addr> "fn()(rettype)" --rpc-url <url>` returns decoded output; `cast chain-id --rpc-url <url>` returns numeric chain ID for locating broadcast artifacts
- Reusable e2e helper at `crates/treb-cli/tests/e2e/deploy_safe.rs` — `deploy_safe(project_dir, rpc_url, owners, threshold)` returns `DeployedSafe` with all infrastructure addresses
- Safe contract stubs use sentinel linked-list pattern (`SENTINEL_OWNERS = address(0x1)`) for owner traversal — governor stubs should use similarly minimal but correct implementations
- `treb_safe` is a direct dependency of `treb-cli`, so integration tests can import `treb_safe::SafeTx` and `compute_safe_tx_hash()` for independent EIP-712 hash verification
- TransactionStatus serializes as SCREAMING_CASE ("EXECUTED", "QUEUED") in registry JSON — assert accordingly

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

**Notes from Phase 6:**
- Follow the same pattern as Safe stubs: minimal but functionally correct Solidity contracts in `src/governance/`, compiled with `pragma solidity =0.8.30;`
- Reuse `deploy_safe()` helper from `e2e/deploy_safe.rs` for the Governor → Safe(1/1) chain test
- CreateCall helper is already deployed at `0xCCCC...01` in `fork_routing.rs` — governor CREATE txs use the same DelegateCall wrapping path
- The `wants_broadcast` fix from Phase 6 means governor senders now correctly go through `broadcast_all()` — no additional routing plumbing needed
- Create a `deploy_governor()` e2e helper mirroring `deploy_safe()`: use `forge script` directly (not `treb run`) to avoid registry entries for infrastructure setup
- Governor sender treb.toml config will need the proposer role in `[namespace.default.senders]` alongside the governor entry, similar to how Safe `signer` references a role key
- The `apply_receipts` ordering fix ensures GovernorProposed results correctly retain Queued status

**Learnings from implementation:**
- Governor contract stubs needed ERC6372 (`clock()`/`CLOCK_MODE()`) for Governor compatibility, and a `queued` field in ProposalCore separate from time-based states for correct `state()` returns
- OZ lib in fixture only has `contracts/proxy/` — governance and ERC20 contracts must be stubbed manually (GovernanceToken, TrebTimelock, TrebGovernor)
- `fork_routing.rs` bypasses `Governor.propose()` entirely — it schedules directly on timelock. `Governor.propose()` is only used in the off-fork routing path
- Timelock constructor must grant `DEFAULT_ADMIN_ROLE` to `address(this)` for `grantRole` to work when impersonating the timelock
- Deployment order matters: token → timelock (empty proposers) → governor → `grantRole(PROPOSER_ROLE, governor)` — governor address unknown at timelock deploy time
- **Bug fix:** Address comparison in `run.rs::handle_queued_executions()` must use `eq_ignore_ascii_case` — `to_checksum()` produces mixed-case while `format!("{:#x}")` produces lowercase
- Governor routing always produces `GovernorProposed` result → transactions get status QUEUED (not EXECUTED), even after fork simulation. Only `forkExecutedAt` is set on the governor proposal
- `forkExecutedAt` uses `skip_serializing_if = "Option::is_none"` — field is absent from JSON when not set; test with `.get("forkExecutedAt").is_none() || .is_null()`
- `DeployViaGovernor.s.sol` broadcasts from `timelockAddress` (not governor) because `broadcast_address()` returns the timelock for Governor+Timelock senders
- Fixture scripts define local `SimTx` struct with `string senderId` vs treb-sol's `bytes32 senderId` — events decode as Unknown but routing uses broadcast `from` addresses, not event senderId
- Governor→Wallet on fork: wallet broadcasts `propose()` on fork, then `simulate_governance_on_fork` impersonates timelock to execute actions directly
- Governor→Safe(1/1) treb.toml requires all three accounts in `namespace.default.senders`: governance, proposer_safe, signer_wallet — recursive resolution needs them all visible
- No code changes were needed for Governor→Safe(1/1) depth-2 routing or `--skip-fork-execution` — the existing routing pipeline handled both correctly out of the box
- `deploy_governor()` helper returns `DeployedGovernor { governor_address, timelock_address, token_address, timelock_delay }` — simpler than `deploy_safe()` (no owners/threshold)
- `PROPOSER_ROLE` is `keccak256("PROPOSER_ROLE")` = `0xb09aa5aeb3702cfd50b6b62bc4532604938f21248a27a1d5ca736082b6819cc1`
- Governance broadcast artifacts have 3 CREATEs (token, timelock, governor) + 1 CALL (grantRole) — only CREATEs have `contractAddress`

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

**Notes from Phase 6:**
- Safe(1/1) produces `RunResult::SafeBroadcast` (executed on fork), Safe(n/m where n>1) produces `RunResult::SafeProposed` (queued) — mixed sender tests must handle both result types
- `safe_context` is NOT populated on transaction records for SafeBroadcast results; it only exists for SafeProposed results that create SafeTransaction records
- Reuse `deploy_safe()` and (from Phase 7) `deploy_governor()` helpers to set up infrastructure; wrap in `spawn_blocking` from async tests

**Notes from Phase 7:**
- `deploy_governor()` helper is at `crates/treb-cli/tests/e2e/deploy_governor.rs` — returns `DeployedGovernor { governor_address, timelock_address, token_address, timelock_delay }`
- Mixed sender treb.toml must include ALL accounts referenced in the sender chain in `namespace.default.senders` — for wallet+governor, that's the wallet role, governance entry, AND the proposer account
- Governor routing always produces `GovernorProposed` (QUEUED), while wallet routing produces Executed — mixed tests must assert different statuses per sender type
- Address comparisons must be case-insensitive throughout (to_checksum vs format!("{:#x}") mismatch) — use `eq_ignore_ascii_case` when comparing addresses from different sources
- `partition_into_runs()` maps transactions by `from` address using `broadcast_address()` — for Governor+Timelock senders, the partition key is the timelock address, not the governor

**Learnings from implementation:**
- **Bug fix:** Orchestrator's `deployer_is_governor` gate prevented governor proposal persistence in mixed-sender configs where deployer is a wallet. Changed to `has_governor_sender` (checks ANY sender is governor), consistent with how safe transactions are always persisted without a `deployer_is_safe` gate. Fix applied at two places in `orchestrator.rs`: main pipeline path (~line 331) and multi-script path (~line 1907)
- For mixed-sender tests, keep `deployer=wallet` — if `deployer=governance`, ALL default `vm.startBroadcast()` (no args) calls go through the governor, preventing true wallet+governor mixing
- Deployment `transactionId` (from Solidity script's keccak256 hash) differs from transaction `id` (pipeline-generated) — match transactions by sender address, not by ID lookups
- Safe(1/1) transactions on fork get EXECUTED status (not QUEUED) because the single owner can approve+execute immediately
- Each fixture script re-declares all structs/events locally — no shared Solidity library to import; `keccak256(abi.encode(block.chainid, block.number, address(counter)))` generates unique txIds per deployment within the same script
- Governor depth limit (`MAX_ROUTE_DEPTH=4`) triggers during `reduce_queue` routing, not during sender resolution — fake addresses work for depth limit tests since the error fires before any on-chain interaction
- Safe threshold query on an EOA returns empty bytes → ABI decode fails with "decode threshold:" error, not an RPC error
- Ledger sender resolution fails immediately with "failed to connect to Ledger device" — this error happens before any script compilation or RPC calls
- Governor proposer chain in v2 treb.toml: each account's `proposer` field references another account name; `find_proposer_role()` follows this chain during routing
- `cargo test --workspace --all-targets` compiles golden `integration_*.rs` tests but does NOT execute them — they must be targeted explicitly with `--test <name>`
- Re-enabled 2 previously-ignored tests: `fixture_forge_build` (forge now available) and `version_json_uses_git_describe_output_in_untagged_checkouts` (dirty suffix no longer an issue)
- 5 remaining `#[ignore]` tests audited and annotated with Phase 9 references — see Phase 9 notes below

---

## Phase 9 -- Fork Infrastructure Test Fixes

Fix the 3 ignored fork tests (restart/revert port unreachable, status with active fork) and the 2 compatibility alias tests (config show format, fork history filter).

**Deliverables**
- Fix `fork_restart_port_unreachable` and `fork_revert_port_unreachable` golden files
- Fix `fork_status_with_active_fork` output format
- Fix `bare_config_matches_config_show` assertion (config show output changed)
- Fix `fork_history_network_filter` (--network filter was removed from fork history)
- Re-enable: 5 remaining ignored tests (3 integration_fork + 2 cli_compatibility_aliases)

**User stories:** 5
**Dependencies:** none (can run in parallel with other phases)

**Notes from Phase 8:**
- Phase 8 audited all 5 remaining `#[ignore]` tests and documented specific failure reasons:
  - `fork_status_with_active_fork` (integration_fork.rs): Uptime drift in golden output
  - `fork_revert_port_unreachable` (integration_fork.rs): golden file mismatch after fork command changes
  - `fork_restart_port_unreachable` (integration_fork.rs): command now succeeds when it was expected to error — behavior changed
  - `bare_config_matches_config_show` (cli_compatibility_aliases.rs): config show output format diverged from raw config
  - `fork_history_network_filter` (cli_compatibility_aliases.rs): `--network` filter was removed from fork history
- `cli_version` dirty-state test (`version_json_uses_git_describe_output_in_untagged_checkouts`) was re-enabled in Phase 8 — it passes reliably now, no Phase 9 work needed for it
- Pre-existing test failures in `fork_integration.rs` (7 failures) are separate from the 3 ignored `integration_fork.rs` golden tests — different test files with different issues

**Learnings from implementation:**
- `UptimeNormalizer` (in `framework/normalizer.rs`) handles both legacy (`3d 1h`) and Go-parity (`2h15m`, `0s`) formats — use `.extra_normalizer(Box::new(UptimeNormalizer))` for any golden test with human-readable uptime/elapsed strings. `TimestampNormalizer` (default chain) only handles `\d+ \w+ ago` patterns, NOT compact forms
- `SpinnerNormalizer` (default chain) was generalized to match ALL braille spinner texts via `[⣾⣽⣻⢿⡿⣟⣯⣷][^\n]*` — no need to update the regex when adding new spinner messages
- Fork revert/restart tests need a `ForkRunSnapshot` pushed onto the store (not just active fork entries) — use the new `seed_fork_with_run_snapshot` helper in `integration_fork.rs` which seeds both active fork entry AND a run snapshot with a valid registry snapshot directory
- `run_restart()` returns `Ok(())` even on Anvil spawn failure — errors are collected and printed to stderr only, so don't use `expect_err(true)` for restart error paths
- Golden framework captures stdout first, then stderr — `eprintln!` warnings appear after `println!` output in golden files
- **Bug fix:** `fork history --json` `--network` filter was missing from the JSON path in `run_history()` — now filters `data.history` entries. `ForkHistoryEntry.network` can be comma-separated for multi-network actions (e.g., "mainnet, sepolia"); filter with `.split(", ").any(|n| n == filter)`
- `fork history --json` output shape is `{active, enteredAt, runSnapshots, history}` — history entries are in the `history` field, not at root level
- `treb config` is normalized to `treb config show` via argv normalization in `main.rs` — always byte-identical output
- `config show` human output format: `Namespace: <val>`, `Network: <val>`, `Source: <val>` — no "Current config" header

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

**Notes from Phase 6:**
- `AnvilRpc` in `fork_routing.rs` now has a `set_code` method (for deploying CreateCall at `0xCCCC...01`) — must be preserved or migrated to alloy provider's `anvil_setCode` equivalent

**Learnings from implementation:**
- Provider construction is centralized in `crates/treb-forge/src/provider.rs`: `build_http_provider(url)` for read-only, `build_wallet_provider(url, wallet)` for signing — all RPC-touching code should use these helpers
- `connect_http()` takes `url::Url`, not a string — URL parsing must happen before the call; `url` crate added as workspace dep (replacing `reqwest::Url`)
- `reqwest` fully removed from `treb-forge/Cargo.toml` — zero production `reqwest` usage in treb-forge. `reqwest` remains in `treb-cli` (run.rs, prune.rs, dev.rs, networks.rs, register.rs) and `treb-safe` (REST API)
- `ureq` preserved in `anvil.rs` for synchronous blocking health checks — not part of the alloy migration
- `AnvilRpc` struct (170 lines) replaced with standalone `pub(crate)` helpers: `anvil_impersonate`, `anvil_stop_impersonating`, `anvil_set_balance`, `anvil_set_code`, `anvil_mine`, `evm_increase_time`, `eth_call_bytes`, `impersonate_send_tx`, `estimate_gas_for_call`, `block_gas_limit` — all take `&impl Provider`
- Anvil-specific RPC: use `provider.raw_request::<_, serde_json::Value>("method_name".into(), (param1, param2))` — `.into()` needed for `Cow<'static, str>`, tuple params for args
- `provider.call(TransactionRequest)` returns `Bytes` directly for `eth_call` — no hex decoding needed
- `provider.get_transaction_receipt()` returns `Ok(Some(receipt))` for confirmed, `Ok(None)` for pending — much simpler than manual JSON-RPC null checks
- `provider.get_chain_id()` returns `u64` directly — no hex parsing needed; wrap in `tokio::time::timeout` for connection timeouts
- `provider.send_transaction(tx).await?.get_receipt().await?` works on Anvil with impersonated accounts (no wallet needed)
- `TransactionRequest.gas` is a field assignment (`tx.gas = Some(30_000_000)`), not a builder method
- Revert detection with alloy: `err.to_string().to_ascii_lowercase().contains("revert")` on `TransportError` — alloy surfaces JSON-RPC error messages in Display impl
- Alloy parses JSON-RPC error responses even from non-200 HTTP responses — it surfaces the JSON-RPC error, not the HTTP status
- Mock JSON-RPC tests with alloy require: `Connection: close` header to prevent keep-alive issues, complete receipt JSON (transactionIndex, from, to, cumulativeGasUsed, effectiveGasPrice, logs, logsBloom, type required for deserialization)
- `receipt_to_broadcast_receipt()` in fork_routing.rs converts alloy `TransactionReceipt` to `BroadcastReceipt` — shared by both fork and live broadcast paths
- Net code reduction: ~430 lines removed across all stories; `parse_hex_u64` and all reqwest-based RPC helper functions eliminated from treb-forge
- `query_proposal_state` no longer takes a `reqwest::Client` — callers just pass `rpc_url` and the function builds the provider internally (signature change affects Phase 1 callers)

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
| 5 | Compose Test Fixes | 4 | 1 | 18 (+ 2 deleted; 7 remain for Phase 1/3) |
| 6 | Deploy Real Safe | 6 | 2 | 0 (new tests) |
| 7 | Deploy Real Governor | 7 | 6 | 0 (new tests) |
| 8 | Mixed Sender & Edge Cases | 5 | 7 | 2 (+ 5 new tests) |
| 9 | Fork Infrastructure Fixes | 5 | -- | 5 |
| 10 | Full Provider Migration | 6 | 1 | 0 |
| **Total** | | **52** | | **~64 + new** |

---

## Ignored Test Inventory

For tracking which tests get re-enabled in which phase:

| Test Binary | Count | Phase |
|------------|------:|-------|
| integration_run | 13 | 1 |
| integration_compose | 15 → 7 | 5 (done: 6 re-enabled, 2 deleted; 7 remain for 1/3) |
| integration_non_interactive | 1 | 1 |
| integration_fork | 3 → 0 | 9 (done: all 3 re-enabled) |
| cli_compatibility_aliases | 2 → 0 | 9 (done: all 2 re-enabled) |
| cli_compose | 12 → 0 | 5 (done: all 12 re-enabled) |
| cli_version | 1 → 0 | 8 (done: re-enabled) |
| fixture_compile | 1 → 0 | 8 (done: re-enabled) |
| e2e_deployment_workflow | 3 | 4 |
| e2e_fork_workflow | 3 | 2 |
| e2e_prune_reset_workflow | 1 | 4 |
| e2e_registry_consistency | 4 | 4 |
| e2e_workflow | 6 | 4 |
| **Total** | **64 → 37** | (25 re-enabled, 2 deleted in Phases 5+8+9) |
