# PRD: Phase 9 — Fork Infrastructure Test Fixes

## Introduction

Phase 9 fixes the 5 remaining ignored tests in the treb-cli-rs test suite: 3 fork golden tests in `integration_fork.rs` and 2 compatibility alias tests in `cli_compatibility_aliases.rs`. These tests were disabled as the fork subsystem and config commands evolved through Phases 1–8, and their golden files or assertions no longer match current behavior. This phase has no code dependencies on other phases and can run in parallel with any remaining work.

After Phase 9, the ignored test count drops from 42 to 37 (the remaining 37 are broadcast-dependent tests awaiting Phases 1/3/4).

## Goals

1. **Re-enable all 5 ignored tests** — remove every `#[ignore]` annotation added with a "Phase 9" comment.
2. **Zero golden file drift** — every re-enabled golden test passes with both `cargo test` and `UPDATE_GOLDEN=1` without manual patching.
3. **No production code behavior changes** — only test files, golden files, and (if needed) dead CLI flag removal are touched; fork/config command logic stays unchanged.
4. **Existing test suite stays green** — `cargo test -p treb-cli` and `cargo clippy --workspace --all-targets` pass with no regressions.

## User Stories

### P9-US-001 — Fix `fork_status_with_active_fork` golden (uptime drift)

**Problem:** The test seeds a `ForkEntry` with a fixed `started_at` timestamp (2026-01-15T10:30:00 UTC). The human output includes an `Uptime: <value>` line computed from `Utc::now() - started_at`, so the golden file drifts every time the test runs.

**Solution:** Add the existing `UptimeNormalizer` (from `framework/normalizer.rs`) as an `extra_normalizer` on this test's golden runner. The normalizer replaces all uptime patterns (`\d+h\d+m`, `\d+d \d+h`, `< 1m`, etc.) with the placeholder `<UPTIME>`. Refresh the golden file with `UPDATE_GOLDEN=1`.

**Acceptance Criteria:**
- `#[ignore]` removed from `fork_status_with_active_fork` in `integration_fork.rs`
- Test uses `UptimeNormalizer` via `extra_normalizer()`
- Golden file `golden/fork_status_with_active_fork/commands.golden` updated with `<UPTIME>` placeholder
- `cargo test -p treb-cli --test integration_fork -- fork_status_with_active_fork` passes
- `cargo clippy --workspace --all-targets` passes

---

### P9-US-002 — Fix `fork_revert_port_unreachable` golden

**Problem:** The holistic fork model changed how `fork revert` behaves when a fork's Anvil port is unreachable. The command still succeeds (prints a warning and restores the registry), but the exact output text in the golden file no longer matches what the command produces.

**Solution:** Remove `#[ignore]`, run the test with `UPDATE_GOLDEN=1` to capture current output, verify the refreshed golden file matches expected behavior (warning about unreachable port + registry restoration message), and commit the updated golden.

**Acceptance Criteria:**
- `#[ignore]` removed from `fork_revert_port_unreachable` in `integration_fork.rs`
- Golden file `golden/fork_revert_port_unreachable/commands.golden` refreshed to match current output
- Output still communicates: (a) registry was restored, (b) Anvil was unreachable so EVM revert was skipped
- `cargo test -p treb-cli --test integration_fork -- fork_revert_port_unreachable` passes
- `cargo clippy --workspace --all-targets` passes

---

### P9-US-003 — Fix `fork_restart_port_unreachable` golden and error expectation

**Problem:** `fork restart` behavior changed — it now succeeds (silently ignores stop errors, spawns fresh Anvil) instead of erroring when the existing port is unreachable. The test still has `.expect_err(true)`, which causes the golden runner to panic with "Unexpected success" before the golden comparison even runs. This means `UPDATE_GOLDEN=1` cannot refresh the golden file until `expect_err` is removed first.

**Solution:** Remove `.expect_err(true)` from the test, remove `#[ignore]`, run with `UPDATE_GOLDEN=1` to capture the new success output. Since the test seeds fork state but has no actual Anvil running, the restart will attempt to spawn Anvil (which may fail in CI). If Anvil spawning is unavailable, the test should handle this gracefully — either by expecting the specific "anvil not ready" error as a known failure, or by mocking the Anvil spawn path. Investigate the actual behavior when running without Anvil and update the golden accordingly.

**Acceptance Criteria:**
- `#[ignore]` removed from `fork_restart_port_unreachable` in `integration_fork.rs`
- `.expect_err(true)` removed or changed to match actual behavior
- Golden file `golden/fork_restart_port_unreachable/commands.golden` refreshed
- Test passes in the standard test environment (`cargo test -p treb-cli --test integration_fork -- fork_restart_port_unreachable`)
- If the test still expects an error (different error than before), the golden and `expect_err` are aligned
- `cargo clippy --workspace --all-targets` passes

---

### P9-US-004 — Fix `bare_config_matches_config_show` compatibility test

**Problem:** The test asserts that `treb config` and `treb config show` produce byte-identical stdout and stderr. The output format has diverged — bare `treb config` no longer prints the "Current config" header that `treb config show` does, or the output structure changed in some other way.

**Solution:** Investigate what `treb config` vs `treb config show` actually output now. The `main.rs` argv normalization routes bare `treb config` to `ConfigSubcommand::Show`, so they should be equivalent. If there's a real divergence, determine whether it's intentional and update the test accordingly:
- If outputs are identical again after recent changes: just remove `#[ignore]` and verify it passes.
- If outputs intentionally differ: update the test to assert the actual current relationship (e.g., both succeed, specific expected differences) rather than byte-identity.

**Acceptance Criteria:**
- `#[ignore]` removed from `bare_config_matches_config_show` in `cli_compatibility_aliases.rs`
- Test assertions updated to match actual current behavior of both commands
- Test passes: `cargo test -p treb-cli --test cli_compatibility_aliases -- bare_config_matches_config_show`
- `cargo clippy --workspace --all-targets` passes

---

### P9-US-005 — Fix `fork_history_network_filter` compatibility test

**Problem:** The test asserts that `treb fork history --network mainnet --json` returns only 2 of 3 seeded history entries (filtering out the sepolia entry). The `--network` flag still exists in the clap definition but the implementation does NOT actually filter history entries by network — it only affects the header display.

**Solution:** Investigate whether network filtering should be implemented or the flag should be removed:
- **Option A (implement filtering):** Add network filtering to `fork history` so `--network mainnet` only returns entries matching that network. This is the minimal fix if the flag is intentionally exposed.
- **Option B (remove dead flag):** If `--network` on `fork history` is vestigial and not intended to filter, remove the flag from the clap definition and update the test to no longer test filtering behavior (e.g., test that all 3 entries appear regardless, or delete the test if the alias parity it tested is no longer relevant).

The preferred approach is **Option A** (implement filtering) since the flag is already exposed to users and filtering is the expected behavior.

**Acceptance Criteria:**
- `#[ignore]` removed from `fork_history_network_filter` in `cli_compatibility_aliases.rs`
- `treb fork history --network mainnet --json` returns only history entries for the "mainnet" network
- Test seeds 3 entries (2 mainnet, 1 sepolia) and asserts filtered result has exactly 2
- Test passes: `cargo test -p treb-cli --test cli_compatibility_aliases -- fork_history_network_filter`
- If Option A: `fork history` implementation filters entries by `--network` value when provided
- If Option B: `--network` flag removed from `fork history` clap args, test rewritten or deleted
- `cargo clippy --workspace --all-targets` passes

## Functional Requirements

- **FR-1:** All 5 `#[ignore]` annotations with "Phase 9" comments are removed.
- **FR-2:** Golden files for the 3 fork tests are refreshed to match current command output.
- **FR-3:** The `UptimeNormalizer` is applied to `fork_status_with_active_fork` so the golden file is stable across runs.
- **FR-4:** The `fork_restart_port_unreachable` test's `expect_err` setting matches the command's actual exit behavior.
- **FR-5:** The `bare_config_matches_config_show` test's assertions match the actual relationship between `treb config` and `treb config show` output.
- **FR-6:** The `fork_history_network_filter` test exercises a working `--network` filter (or is updated to reflect the flag's removal).
- **FR-7:** `cargo test -p treb-cli` passes with no new test failures.
- **FR-8:** `cargo clippy --workspace --all-targets` passes with no new warnings.

## Non-Goals

- **No fork command behavior changes** — the fork enter/exit/status/revert/restart commands are not being modified (except possibly adding network filtering to `fork history` in US-005).
- **No new test coverage** — this phase only re-enables existing ignored tests, it does not add new test cases.
- **No golden file format changes** — the golden test framework and normalizer infrastructure stay as-is; we only add existing normalizers to tests that need them.
- **No broadcast-dependent test fixes** — the 37 remaining ignored tests (integration_run, integration_compose broadcast subset, e2e_* suites) depend on Phases 1/3/4 and are out of scope.
- **No config command behavior changes** — only the test assertion is updated, not the config show implementation.

## Technical Considerations

**Dependencies:** None. All 5 tests are self-contained subprocess tests using `TestContext` and seeded fixture data. No Anvil, RPC, or broadcast infrastructure is required (fork tests seed state directly via `ForkStateStore`).

**Existing infrastructure to reuse:**
- `UptimeNormalizer` in `crates/treb-cli/tests/framework/normalizer.rs` — already handles all uptime format variants
- `seed_fork_status()` and `seed_fork_history()` helpers in `integration_fork.rs` — create deterministic fork state
- `setup_config_project()` in `cli_compatibility_aliases.rs` — creates minimal foundry project with init
- `extra_normalizer()` on `GoldenTest` — plugs in additional normalizers per-test

**Risk: fork_restart_port_unreachable Anvil dependency:**
The restart command attempts to spawn a new Anvil process. In the test environment (no running Anvil on the seeded port), the command may fail with a different error than the original golden captured. The test must be aligned to whatever the command actually does: if it errors with "anvil not ready", capture that; if it succeeds with a warning, capture that. Run the command manually first to determine the actual behavior before committing the golden.

**Risk: fork history --network flag decision:**
US-005 requires a judgment call: implement filtering or remove the dead flag. If implementing filtering, it's a small change in `fork.rs` (filter `history` entries by `entry.network == network` before display). If removing the flag, update `build.rs` completions too. Either path is small, but the choice should be made before starting implementation.

**Golden refresh workflow:**
For tests where the command output changed, the standard workflow is:
1. Remove `#[ignore]` (and fix `expect_err` if needed)
2. Run with `UPDATE_GOLDEN=1 cargo test -p treb-cli --test <test_binary> -- <test_name>`
3. Review the refreshed golden file to verify it matches expected behavior
4. Commit the updated golden file
