# PRD: Phase 4 - E2E Broadcast Tests (Wallet Path)

## Introduction

Phase 4 re-enables the 14 ignored end-to-end tests that exercise the full wallet broadcast pipeline: `treb init` → `treb run --broadcast` → registry verification. These tests were disabled before Phase 1 introduced live signing via `alloy-provider`. With live signing, checkpoint saves (Phase 3), and non-interactive fork simulation (Phase 2) now in place, these tests should pass against a plain Anvil node — after fixing CLI command signature drift accumulated over Phases 1–3.

This phase does not add new production code. It is a test-only phase that validates the cumulative work of Phases 1–3 by re-enabling the e2e test suite that covers the core wallet deployment workflow.

## Goals

1. **Re-enable all 14 ignored e2e tests** across `e2e_workflow.rs` (6), `e2e_deployment_workflow.rs` (3), `e2e_registry_consistency.rs` (4), and `e2e_prune_reset_workflow.rs` (1) — all passing in CI.
2. **Fix CLI command signature drift** — update tests that reference removed flags (`--dry-run`) or changed fork CLI interfaces (`fork exit --network` → holistic `fork exit`).
3. **Verify broadcast artifact format** — confirm `run-latest.json` is written with per-transaction hashes and receipts in Foundry-compatible `ScriptSequence` format after a successful broadcast.
4. **Zero production code changes** — all changes confined to test files under `crates/treb-cli/tests/`.

## User Stories

### P4-US-001: Re-enable core e2e_workflow tests (6) and e2e_prune_reset_workflow (1)

**Description:** Remove `#[ignore]` from 7 tests that exercise the core wallet broadcast pipeline: init → run → list/show/tag/prune/reset. These tests use `run_deployment()` → `treb run ... --broadcast --non-interactive` which should work with Phase 1's live signing against Anvil.

**Tests to re-enable:**
- `e2e_workflow::e2e_init_run_list`
- `e2e_workflow::e2e_run_show`
- `e2e_workflow::e2e_run_tag_list_with_filter`
- `e2e_workflow::e2e_run_prune_dry_run_clean`
- `e2e_workflow::e2e_run_reset_list`
- `e2e_workflow::e2e_list_no_color_has_no_ansi`
- `e2e_prune_reset_workflow::e2e_deploy_reset_redeploy`

**Files:** `crates/treb-cli/tests/e2e_workflow.rs`, `crates/treb-cli/tests/e2e_prune_reset_workflow.rs`

**Acceptance Criteria:**
- All 7 `#[ignore]` attributes removed
- All 7 tests pass with `cargo test -p treb-cli --test e2e_workflow --test e2e_prune_reset_workflow`
- Fix any assertion failures caused by output format drift (e.g., JSON field renames, registry command renames like `treb reset` → `treb registry drop`)
- `cargo clippy --workspace --all-targets` passes

### P4-US-002: Re-enable and fix e2e_deployment_workflow tests (3)

**Description:** Remove `#[ignore]` from 3 deployment lifecycle tests. The `e2e_dry_run_no_registry_mutation` test references `--dry-run` which was removed in Phase 1 — rewrite it to use simulation-only mode (omit `--broadcast`) instead. Validate the `RunOutputJson` schema emitted by `treb run --json --broadcast`.

**Tests to re-enable:**
- `e2e_deployment_workflow::e2e_full_deployment_lifecycle`
- `e2e_deployment_workflow::e2e_run_json_output_fields`
- `e2e_deployment_workflow::e2e_dry_run_no_registry_mutation`

**Files:** `crates/treb-cli/tests/e2e_deployment_workflow.rs`

**Required fixes:**
- `e2e_dry_run_no_registry_mutation`: Remove `--dry-run` and `--broadcast` flags from the command invocation; simulation-only mode = omit `--broadcast` (per Phase 1 learnings). The test assertion (deployments.json empty or absent) remains the same since simulation-only mode should not persist registry entries.
- `e2e_run_json_output_fields`: Validate `RunOutputJson` schema matches current output — check for any new fields added by Phase 3 (checkpoint-related) or field renames. In particular, `dryRun` may have been renamed or removed since `--dry-run` was deleted.

**Acceptance Criteria:**
- All 3 `#[ignore]` attributes removed
- `e2e_dry_run_no_registry_mutation` rewritten to omit `--broadcast` instead of using `--dry-run`
- All 3 tests pass with `cargo test -p treb-cli --test e2e_deployment_workflow`
- `RunOutputJson` field assertions in `e2e_run_json_output_fields` match current schema
- `cargo clippy --workspace --all-targets` passes

### P4-US-003: Re-enable and fix e2e_registry_consistency tests (4)

**Description:** Remove `#[ignore]` from 4 registry consistency tests. The `e2e_registry_consistency_after_fork_cycle` test references the old per-network fork CLI interface (`fork exit --network anvil-31337`) — update it for the current holistic fork model where `fork exit` takes no arguments and snapshots live at `priv/snapshots/holistic`.

**Tests to re-enable:**
- `e2e_registry_consistency::e2e_registry_consistency_after_deployment`
- `e2e_registry_consistency::e2e_registry_consistency_after_tag`
- `e2e_registry_consistency::e2e_registry_consistency_after_reset`
- `e2e_registry_consistency::e2e_registry_consistency_after_fork_cycle`

**Files:** `crates/treb-cli/tests/e2e_registry_consistency.rs`

**Required fixes for `e2e_registry_consistency_after_fork_cycle`:**
- Line 231: `fork enter --network anvil-31337 --rpc-url` — keep `--network` (enter still accepts it), but verify the network name matches what `foundry.toml` defines for the Anvil RPC endpoint
- Line 255: `fork exit --network anvil-31337` → `fork exit` (holistic, no args)
- Line 283: Snapshot cleanup assertion path `priv/snapshots/anvil-31337` → `priv/snapshots/holistic`
- Line 268: Fork state JSON shape — verify `forks` is checked correctly for the holistic model (Phase 2 learnings: `fork status --json` returns `{"active": bool, "enteredAt": ..., "forks": [...]}`)

**Acceptance Criteria:**
- All 4 `#[ignore]` attributes removed
- Fork cycle test updated for holistic fork CLI interface
- All 4 tests pass with `cargo test -p treb-cli --test e2e_registry_consistency`
- `assert_registry_consistent()` validates all cross-references after each operation
- `cargo clippy --workspace --all-targets` passes

### P4-US-004: Add broadcast artifact assertions to e2e tests

**Description:** Extend one or two existing e2e tests to verify that broadcast artifacts (`run-latest.json`) are written correctly after a successful `treb run --broadcast`. Phase 3 introduced checkpoint saves that write per-transaction hashes and receipts — e2e tests should confirm these artifacts exist and contain the expected structure.

**Files:** `crates/treb-cli/tests/e2e_workflow.rs` or `crates/treb-cli/tests/e2e_deployment_workflow.rs`, `crates/treb-cli/tests/e2e/mod.rs`

**Assertions to add (extend `e2e_init_run_list` or `e2e_full_deployment_lifecycle`):**
- `broadcast/` directory exists under the project root after a successful broadcast
- `run-latest.json` exists within the broadcast directory structure
- `run-latest.json` parses as valid JSON containing a `transactions` array
- Each transaction entry has a non-null `hash` field
- `receipts` array is non-empty and each receipt has `transactionHash` and `status`

**Acceptance Criteria:**
- At least one e2e test asserts broadcast artifact existence and basic structure
- Helper added to `e2e/mod.rs` if reusable (e.g., `assert_broadcast_artifacts_exist(project_root)`)
- All tests pass with `cargo test -p treb-cli --test e2e_workflow --test e2e_deployment_workflow`
- `cargo clippy --workspace --all-targets` passes

## Functional Requirements

- **FR-1:** All 14 previously-ignored e2e tests compile and pass against a local Anvil node.
- **FR-2:** Tests that referenced `--dry-run` are rewritten to use simulation-only mode (omit `--broadcast`).
- **FR-3:** Tests that referenced per-network fork exit (`fork exit --network X`) are updated for holistic fork exit (no args).
- **FR-4:** Fork cycle test validates holistic snapshot directory (`priv/snapshots/holistic`) cleanup, not per-network paths.
- **FR-5:** At least one e2e test asserts `run-latest.json` broadcast artifact structure after a successful broadcast.
- **FR-6:** `cargo test --workspace --all-targets` passes with no new test failures.
- **FR-7:** `cargo clippy --workspace --all-targets` passes with no new warnings.

## Non-Goals

- **No production code changes.** All work is in test files under `crates/treb-cli/tests/`.
- **No new CLI flags or features.** Tests are adapted to the current CLI surface, not the other way around.
- **No Safe or Governor e2e tests.** This phase covers only the wallet broadcast path. Safe/Governor e2e coverage is Phase 6–8.
- **No compose test fixes.** Compose tests are Phase 5.
- **No fork infrastructure fixes.** The 3 ignored fork tests and 2 compatibility alias tests are Phase 9.
- **No golden file updates.** These e2e tests use programmatic assertions, not golden file snapshots. If golden file drift is discovered tangentially, note it but do not fix it here.

## Technical Considerations

### Dependencies

- **Phase 1 (complete):** Live signing via `alloy-provider` — `broadcast_wallet_run_live` and `broadcast_routable_txs_live` must be working for `treb run --broadcast` to succeed against Anvil.
- **Phase 3 (complete):** Checkpoint saves — `run-latest.json` is written per-transaction during broadcast. P4-US-004 validates this.
- **Phase 2 (complete):** Holistic fork mode — `fork exit` takes no arguments. Required for the fork cycle consistency test.

### Key Integration Points

- **`e2e/mod.rs` shared helpers:** `spawn_anvil_or_skip()`, `setup_project()`, `run_deployment()`, `run_json()`, `assert_deployment_count()`, `get_deployment_id()`, `assert_registry_consistent()` — all should work as-is since they don't reference removed flags.
- **`run_deployment()` (e2e/mod.rs:188):** Calls `treb run script/TrebDeploySimple.s.sol --rpc-url <url> --broadcast --non-interactive` — this is the core path that exercises Phase 1's live signing.
- **Registry store format:** Tests read `.treb/deployments.json`, `.treb/transactions.json`, `.treb/lookup.json`, `.treb/fork.json` directly. The bare-map format (no `_format`/`entries` wrapper) is current; `read_registry_file()` in `e2e/mod.rs` already handles both formats via the legacy compat check.

### Known Drift Catalog

| Test | Issue | Fix |
|------|-------|-----|
| `e2e_dry_run_no_registry_mutation` | Uses `--dry-run` flag (removed in P1) | Remove `--broadcast` and `--dry-run`; simulation-only = omit `--broadcast` |
| `e2e_registry_consistency_after_fork_cycle` | Uses `fork exit --network anvil-31337` | Change to `fork exit` (holistic, no args) |
| `e2e_registry_consistency_after_fork_cycle` | Checks `priv/snapshots/anvil-31337` | Change to `priv/snapshots/holistic` |
| `e2e_registry_consistency_after_fork_cycle` | `forks` field shape may differ | Verify against holistic model: `forks` may be object or array depending on fork state store |
| `e2e_run_json_output_fields` | `dryRun` field may be renamed/removed | Verify current `RunOutputJson` schema |
| `e2e_run_reset_list` | May use `treb reset` (old command) | Verify uses `treb registry drop` (current) |

### Risk: Anvil Availability

All 14 tests use `spawn_anvil_or_skip()` which returns `None` in restricted environments. Tests skip cleanly when Anvil is unavailable. This is existing behavior and does not change.

### Broadcast Artifact Path

Foundry writes broadcast artifacts to `broadcast/<script_name>/<chain_id>/`. The exact path for the Anvil chain ID 31337 would be `broadcast/TrebDeploySimple.s.sol/31337/run-latest.json`. P4-US-004 assertions should discover this path dynamically rather than hardcoding it.
