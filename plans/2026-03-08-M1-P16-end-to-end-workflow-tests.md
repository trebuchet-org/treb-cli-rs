# PRD: Phase 16 - End-to-End Workflow Tests

## Introduction

Phase 16 creates comprehensive end-to-end workflow tests that exercise multi-command sequences matching real user scenarios. While previous phases (4-15) each tested individual commands in isolation with golden files and unit tests, this phase validates that commands **compose correctly** — that registry state flows between commands, that fork mode properly isolates and restores state, and that deployment lifecycles (deploy → tag → prune → reset) work end-to-end.

The existing `e2e_workflow.rs` file already contains 6 basic E2E tests (init→run→list, run→show, run→tag→list, run→prune, run→reset→list, list--no-color). This phase expands that foundation with deeper assertions, new workflows (fork mode, register, compose), and structured registry consistency verification.

All E2E tests run against in-process Anvil nodes using the established `AnvilConfig::new().port(0).spawn()` pattern. Tests use `--json` output for structured assertions and `--non-interactive` to prevent prompt blocking.

## Goals

1. **Workflow coverage**: Every major multi-command user journey has at least one E2E test exercising the full sequence with live Anvil execution.
2. **Registry consistency**: After every multi-command sequence, verify that registry files (deployments.json, transactions.json, lookup.json) are internally consistent and cross-referenced correctly.
3. **Fork isolation verification**: Prove that fork enter/exit properly snapshots and restores registry state, and that fork diff accurately reports changes made during fork mode.
4. **Reusable infrastructure**: Extract shared E2E test helpers (project setup, deployment execution, JSON assertion utilities, registry file readers) so future phases can add workflow tests without duplicating boilerplate.
5. **No flaky tests**: All tests use OS-assigned ports, deterministic assertions (no timing-sensitive checks), and graceful skip via `spawn_anvil_or_skip()` in restricted environments.

## User Stories

### P16-US-001: E2E Test Infrastructure and Shared Helpers

**Description**: Extract the common setup patterns from `e2e_workflow.rs` into a reusable module and add new assertion helpers for JSON output parsing and registry file validation. This shared module will be consumed by all subsequent stories.

**Current state**: `e2e_workflow.rs` contains inline helpers (`setup_project()`, `run_deployment()`, `spawn_anvil_or_skip()`, `copy_dir_recursive()`, `TREB_TOML`, `TREB_DEPLOY_SCRIPT`) that cannot be imported by other test files. No registry file assertion helpers exist.

**Changes**:
- Create `crates/treb-cli/tests/e2e/mod.rs` as a shared E2E helper module with:
  - `setup_project()` — copies gen-deploy-project fixture, writes treb.toml + deploy script, runs `treb init`
  - `run_deployment(path, rpc_url)` — executes `treb run` with broadcast against given RPC
  - `spawn_anvil_or_skip()` — spawns Anvil or returns None in restricted envs
  - `treb()` — returns `assert_cmd::Command` for the treb-cli binary
  - `copy_dir_recursive()` — recursive directory copy
  - `TREB_TOML` and `TREB_DEPLOY_SCRIPT` constants
- Add JSON assertion helpers:
  - `run_json(path, args) -> serde_json::Value` — run command with `--json`, assert success, parse stdout
  - `assert_deployment_count(json, expected)` — assert array length
  - `get_deployment_id(json, index) -> String` — extract deployment ID from list output
- Add registry file readers:
  - `read_registry_file<T: DeserializeOwned>(treb_dir, filename) -> T` — generic JSON file reader
  - `read_deployments(treb_dir) -> serde_json::Value` — read deployments.json
  - `read_transactions(treb_dir) -> serde_json::Value` — read transactions.json
  - `deployment_count(treb_dir) -> usize` — count entries in deployments.json
- Refactor existing `e2e_workflow.rs` to import from `e2e/mod.rs` instead of defining helpers inline

**Acceptance criteria**:
- [ ] `e2e/mod.rs` module exists and exports all listed helpers
- [ ] Existing 6 tests in `e2e_workflow.rs` still pass after refactor (use `mod e2e;` import)
- [ ] `cargo test -p treb-cli -- e2e_init_run_list` passes (confirms no regression)
- [ ] Typecheck passes: `cargo check -p treb-cli --tests`

---

### P16-US-002: Basic Deployment Workflow with Full Assertions

**Description**: Expand the existing init→run→list and run→show tests with deeper JSON field validation, and add a new comprehensive workflow test that exercises init → run → list → show → tag → list-with-filter in a single test function with structured assertions at each step.

**Current state**: Existing tests verify basic success/failure and minimal field presence (e.g., "address exists", "array length is 1"). They don't validate deployment type, contract name, chain ID value, namespace, transaction count, or cross-reference between list and show output.

**Changes**:
- Add `e2e_full_deployment_lifecycle` test in a new `crates/treb-cli/tests/e2e_deployment.rs`:
  - Step 1: `treb init` — verify .treb/ directory created
  - Step 2: `treb run script/TrebDeploySimple.s.sol --rpc-url <anvil> --broadcast --non-interactive --json` — parse output, verify `gasUsed > 0`, `deployments` array non-empty, `transactions` array non-empty
  - Step 3: `treb list --json` — verify exactly 1 deployment, extract ID, verify `contractName == "SimpleContract"`, `namespace == "default"`, `chainId` matches Anvil's chain ID (31337)
  - Step 4: `treb show <id> --json` — verify `address` matches list output, `contractName` matches, `chainId` matches, `deploymentType` is present
  - Step 5: `treb tag <id> --add v1.0.0 --add latest` — assert success
  - Step 6: `treb list --tag v1.0.0 --json` — verify 1 result, same ID
  - Step 7: `treb list --tag nonexistent --json` — verify 0 results
  - Step 8: `treb tag <id> --remove latest` — assert success
  - Step 9: `treb show <id> --json` — verify tags array contains "v1.0.0" but not "latest"
- Add `e2e_run_json_output_fields` test verifying all expected `RunOutputJson` fields are present and well-formed (deployments, transactions, gasUsed, skipped, consoleLogs)
- Add `e2e_dry_run_no_registry_mutation` test: run with `--dry-run`, then verify deployments.json is empty (no state written)

**Acceptance criteria**:
- [ ] `e2e_full_deployment_lifecycle` passes with all 9 steps validated
- [ ] `e2e_run_json_output_fields` validates all RunOutputJson fields
- [ ] `e2e_dry_run_no_registry_mutation` confirms dry-run doesn't write state
- [ ] All tests use `spawn_anvil_or_skip()` and skip cleanly in restricted envs
- [ ] Typecheck passes: `cargo check -p treb-cli --tests`

---

### P16-US-003: Fork Mode E2E Workflow

**Description**: Create an end-to-end test that exercises the full fork lifecycle: enter fork mode, deploy a contract (modifying registry), verify the diff shows the addition, revert to restore pre-fork state, and exit fork mode — verifying registry state at each transition.

**Current state**: `integration_fork.rs` tests fork subcommands with pre-seeded data (no live Anvil). The fork enter/exit commands have never been tested with actual deployment execution between them.

**Changes**:
- Add `crates/treb-cli/tests/e2e_fork.rs` with async tests:
  - `e2e_fork_enter_deploy_diff_revert_exit`:
    1. Setup project with `setup_project()`, spawn Anvil
    2. `treb fork enter --network anvil-31337 --rpc-url <anvil_url>` — assert success
    3. Verify fork.json has an active fork entry via registry file reader
    4. `treb run script/TrebDeploySimple.s.sol --rpc-url <anvil_url> --broadcast --non-interactive` — assert success
    5. `treb list --json` — verify 1 deployment exists
    6. `treb fork diff --json` — verify diff shows 1 added deployment
    7. `treb fork revert --network anvil-31337` — assert success
    8. `treb list --json` — verify 0 deployments (state reverted to pre-fork snapshot)
    9. `treb fork exit --network anvil-31337` — assert success
    10. Verify fork.json has no active forks
    11. `treb list --json` — verify still 0 deployments
  - `e2e_fork_enter_deploy_exit_restores_state`:
    1. Same setup, enter fork, deploy
    2. Exit fork directly (without revert)
    3. Verify registry restored to pre-fork state (0 deployments)
    4. Verify fork.json cleaned up
  - `e2e_fork_status_shows_active_fork`:
    1. Enter fork mode
    2. `treb fork status --json` — verify active fork entry with correct network, chain ID, RPC URL

**Note**: The foundry.toml in the test fixture must have an `[rpc_endpoints]` section with `anvil-31337 = "http://localhost:8545"` — port rewriting will update the port to match the spawned Anvil. The fork commands use `--network anvil-31337` to reference this endpoint.

**Acceptance criteria**:
- [ ] `e2e_fork_enter_deploy_diff_revert_exit` passes all 11 verification steps
- [ ] `e2e_fork_enter_deploy_exit_restores_state` confirms exit restores registry
- [ ] `e2e_fork_status_shows_active_fork` validates fork status JSON fields
- [ ] Fork.json state transitions verified at each step (active → reverted → exited)
- [ ] All tests skip cleanly via `spawn_anvil_or_skip()` in restricted envs
- [ ] Typecheck passes: `cargo check -p treb-cli --tests`

---

### P16-US-004: Register and Tag E2E Workflow

**Description**: Test the register command's ability to import an existing deployment by transaction hash, then verify the registered deployment can be tagged and shown with correct metadata.

**Current state**: No E2E test exercises the register → tag → show workflow. The register command uses `trace_transaction` RPC which works against Anvil's `debug_traceTransaction` support.

**Changes**:
- Add `crates/treb-cli/tests/e2e_register.rs` with async tests:
  - `e2e_register_from_tx_hash`:
    1. Setup project, spawn Anvil, run deployment (broadcast)
    2. `treb list --json` — get the deployment's transaction hash from list or show output
    3. `treb reset --yes` — clear registry to start fresh for register test
    4. `treb register --tx-hash <hash> --rpc-url <anvil_url> --namespace default --non-interactive --json` — assert success
    5. `treb list --json` — verify 1 deployment, verify it has the correct contract address (matching original deployment)
    6. `treb tag <id> --add v2.0.0` — assert success
    7. `treb show <id> --json` — verify tags contains "v2.0.0", verify contractName and address fields present
  - `e2e_register_tag_show_roundtrip`:
    1. Register a deployment (same approach)
    2. Add multiple tags (`v1.0.0`, `stable`, `production`)
    3. `treb show <id> --json` — verify all 3 tags present
    4. `treb list --tag stable --json` — verify 1 result
    5. Remove one tag (`--remove production`)
    6. `treb show <id> --json` — verify 2 tags remain

**Note**: If `trace_transaction` is not supported by the Anvil version (some stripped configs), the test should detect this and skip with a clear message. Use `--skip-verify` flag on register to avoid block explorer calls. If the register command needs additional flags to work non-interactively, use `--non-interactive` and provide all required fields via flags.

**Acceptance criteria**:
- [ ] `e2e_register_from_tx_hash` registers from a real transaction and verifies the deployment
- [ ] `e2e_register_tag_show_roundtrip` exercises full tag lifecycle on a registered deployment
- [ ] Registered deployment address matches the originally deployed contract address
- [ ] All tests skip cleanly in restricted environments
- [ ] Typecheck passes: `cargo check -p treb-cli --tests`

---

### P16-US-005: Deployment Lifecycle with Prune and Reset

**Description**: Test the full deployment lifecycle including prune (with on-chain bytecode verification) and scoped reset operations.

**Current state**: `e2e_run_prune_dry_run_clean` exists but only tests prune dry-run on a clean registry (expects "Nothing to prune"). `e2e_run_reset_list` tests full reset. No test exercises prune with `--check-onchain` or scoped reset with `--namespace`/`--network` filters.

**Changes**:
- Add `crates/treb-cli/tests/e2e_lifecycle.rs` with async tests:
  - `e2e_prune_onchain_clean_registry`:
    1. Setup project, spawn Anvil, deploy SimpleContract
    2. `treb prune --check-onchain --rpc-url <anvil_url> --dry-run --json` — verify nothing to prune (all contracts have live bytecode)
    3. Verify deployment still in registry (prune dry-run doesn't mutate)
  - `e2e_prune_detects_selfdestructed`:
    1. Deploy a contract, verify it exists in registry
    2. Use `anvil_setCode` RPC to zero out the contract's bytecode (simulating selfdestruct)
    3. `treb prune --check-onchain --rpc-url <anvil_url> --dry-run --json` — verify 1 prune candidate
    4. `treb prune --check-onchain --rpc-url <anvil_url> --yes --json` — verify pruned
    5. `treb list --json` — verify 0 deployments
  - `e2e_reset_scoped_by_namespace`:
    1. Deploy 2 contracts (would need 2 deploy scripts with different namespaces, or manual registry seeding)
    2. `treb reset --namespace default --yes` — reset only default namespace
    3. Verify scoped reset only removes matching entries
  - `e2e_deploy_reset_redeploy`:
    1. Deploy → verify 1 deployment → reset → verify 0 → deploy again → verify 1
    2. Confirms the full create-destroy-recreate cycle works cleanly

**Note**: `anvil_setCode` is called via raw JSON-RPC: `{"method":"anvil_setCode","params":["<address>","0x"]}`. This requires extracting the deployed address from `treb list --json` output, then making a direct HTTP call to the Anvil RPC URL. Use `reqwest` or `std::process::Command` with `curl` for this.

**Acceptance criteria**:
- [ ] `e2e_prune_onchain_clean_registry` confirms no false positives on live contracts
- [ ] `e2e_prune_detects_selfdestructed` proves prune catches zeroed bytecode
- [ ] `e2e_reset_scoped_by_namespace` validates namespace-scoped reset
- [ ] `e2e_deploy_reset_redeploy` confirms clean recreate cycle
- [ ] All JSON output assertions use structured parsing (not string matching)
- [ ] Typecheck passes: `cargo check -p treb-cli --tests`

---

### P16-US-006: Cross-Command Registry Consistency Assertions

**Description**: Add a suite of tests that verify registry file invariants hold after complex multi-command sequences. These tests read the raw `.treb/*.json` files and check cross-references, index consistency, and data integrity.

**Current state**: Existing E2E tests only check command output (stdout JSON). No test reads the registry files directly to verify internal consistency (e.g., that lookup.json indexes match deployments.json entries, that transaction IDs in deployments reference valid transactions).

**Changes**:
- Add `crates/treb-cli/tests/e2e_consistency.rs` with async tests:
  - `e2e_registry_consistency_after_deployment`:
    1. Deploy a contract
    2. Read `.treb/deployments.json` — parse all deployment entries
    3. Treat `.treb/deployments.json` object keys as the canonical deployment IDs
    4. Read `.treb/lookup.json` — verify every deployment address appears in the `byAddress` index
    5. Verify every deployment contract name appears in the `byName` index
    6. Read `.treb/transactions.json` — verify transaction IDs referenced by deployments exist
  - `e2e_registry_consistency_after_tag`:
    1. Deploy, tag with multiple tags
    2. Verify lookup.json `byTag` index contains the tag → deployment mapping
    3. Verify deployment entry's tags array matches what `show --json` reports
  - `e2e_registry_consistency_after_reset`:
    1. Deploy multiple (2+ deploy runs)
    2. Reset
    3. Verify all registry files are empty/reset (deployments.json = {}, transactions.json = {}, lookup.json indexes empty)
    4. Verify registry.json still has valid metadata (not deleted)
  - `e2e_registry_consistency_after_fork_cycle`:
    1. Enter fork, deploy, exit fork
    2. Verify deployments.json matches pre-fork state (empty)
    3. Verify fork.json has no active forks
    4. Verify snapshot directory is cleaned up
  - Add a `assert_registry_consistent(treb_dir)` helper function that performs the common consistency checks (lookup index ↔ deployments cross-reference) and can be called from any E2E test as a final assertion

**Acceptance criteria**:
- [ ] `e2e_registry_consistency_after_deployment` validates deployments.json-keyed IDs and all implemented lookup.json index cross-references
- [ ] `e2e_registry_consistency_after_tag` validates tag indexes
- [ ] `e2e_registry_consistency_after_reset` validates clean reset state
- [ ] `e2e_registry_consistency_after_fork_cycle` validates fork cleanup
- [ ] `assert_registry_consistent()` helper is reusable from any test
- [ ] All tests use structured JSON parsing (serde_json) for registry file reading
- [ ] Typecheck passes: `cargo check -p treb-cli --tests`

---

## Functional Requirements

- **FR-1**: All E2E tests must use in-process Anvil nodes with OS-assigned ports (`port(0)`) — no hardcoded ports.
- **FR-2**: All E2E tests requiring Anvil must use `#[tokio::test(flavor = "multi_thread")]` and `spawn_anvil_or_skip()` for graceful degradation in restricted environments.
- **FR-3**: CLI subprocess invocations must use `tokio::task::spawn_blocking` to avoid blocking the async runtime.
- **FR-4**: All tests must use `--non-interactive` or `TREB_NON_INTERACTIVE=true` when running commands that could prompt for input (run --broadcast, reset, prune).
- **FR-5**: JSON output assertions must parse stdout with `serde_json::from_slice` — no regex or string-contains assertions for structured data.
- **FR-6**: Registry file assertions must read `.treb/*.json` files directly and parse with serde_json for cross-reference validation.
- **FR-7**: Tests must not depend on external services (block explorers, Safe Transaction Service, etc.) — all operations must complete against local Anvil only.
- **FR-8**: Shared E2E helpers must be importable from multiple test files via `mod e2e;` pattern.
- **FR-9**: Each Anvil instance must be explicitly dropped at test end to release the bound port.
- **FR-10**: Tests must pass deterministically — no timing-sensitive assertions, no flaky port conflicts.

## Non-Goals

- **Verify command E2E**: Verification requires block explorer APIs; this is tested in Phase 6 with mocked responses. E2E tests will not call external verification APIs.
- **Safe/Governor live E2E**: Full Safe multisig and Governor proposal lifecycles require deployed governance contracts and API services. These are adequately tested in Phases 12-13 with pre-seeded data and mocked interactions.
- **Compose live execution E2E**: Multi-component compose with live broadcast requires multiple deploy scripts with inter-component dependencies. This is complex to set up reliably and is better covered by compose dry-run tests (Phase 7) + individual run E2E tests.
- **Performance benchmarking**: These tests verify correctness, not speed.
- **Golden file snapshots**: E2E tests use programmatic assertions (JSON field checks, registry file parsing) rather than golden file comparison, since output includes non-deterministic values (addresses, hashes, timestamps).
- **CI parallelism optimization**: Tests will run sequentially within each test file; parallelism optimization across files is left to cargo's default test runner.

## Technical Considerations

### Dependencies
- **treb-forge**: For `AnvilConfig`, `AnvilInstance`, `anvil::spawn()` — already a dev-dependency of treb-cli
- **serde_json**: For JSON parsing of command output and registry files — already available
- **tokio**: For async test runtime — already configured
- **assert_cmd**: For CLI subprocess execution — already used extensively
- **reqwest** (or raw TCP): For `anvil_setCode` RPC call in prune tests — treb-forge already depends on reqwest

### Test File Organization
```
crates/treb-cli/tests/
  e2e/
    mod.rs              # Shared helpers (P16-US-001)
  e2e_workflow.rs       # Existing tests (refactored to use e2e/ module)
  e2e_deployment.rs     # P16-US-002
  e2e_fork.rs           # P16-US-003
  e2e_register.rs       # P16-US-004
  e2e_lifecycle.rs      # P16-US-005
  e2e_consistency.rs    # P16-US-006
```

### Anvil Port Management
Each test spawns its own Anvil with `port(0)`. Tests are independent and don't share Anvil instances. The `ContextPool` pattern from the framework is available but not required since E2E tests are inherently sequential within each test function.

### Foundry.toml Port Rewriting
Fork E2E tests need `foundry.toml` with `[rpc_endpoints]` that reference the Anvil port. Use `port_rewrite_foundry_toml_single(workdir, port)` from the existing test framework to dynamically update ports.

### RPC Calls for Prune Tests
The `anvil_setCode` RPC call to simulate self-destructed contracts can be made via a simple HTTP POST:
```rust
reqwest::Client::new()
    .post(&rpc_url)
    .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"anvil_setCode","params":[address,"0x"]}))
    .send().await?;
```

### Register Command Constraints
The `register --tx-hash` command uses `debug_traceTransaction` RPC, which Anvil supports. However, the register command may require interactive contract selection if multiple contracts are found in the trace. Use `--non-interactive` and/or `--contract-name` to pre-select.

### Test Execution Time
E2E tests with Anvil + forge compilation are slow (5-30 seconds each). The test suite should be structured so CI can run E2E tests as a separate job or with `--test-threads=1` to avoid resource contention.
