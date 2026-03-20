# PRD: Phase 2 - Non-Interactive Fork Simulation & --skip-fork-execution

## Introduction

Phase 2 changes the behavior of `handle_queued_executions()` in `crates/treb-cli/src/commands/run.rs` so that non-interactive mode auto-simulates queued Safe/Governor executions on fork instead of skipping them. This matches the Go CLI behavior where CI pipelines get full fork simulation without manual prompts. A new `--skip-fork-execution` flag is added for users who explicitly want to defer simulation to `treb fork exec`.

Additionally, `display_script_broadcast_summary()` is updated to show a pending indicator ("⏳") for governor/safe proposed results, improving visibility of queued items in the output.

This phase depends on Phase 1 (live signing) and unblocks Phase 6 (real Safe contract deployment tests). It re-enables 3 `e2e_fork_workflow` tests that use `--non-interactive` and were blocked on the broadcast signing path completed in Phase 1.

## Goals

1. **Non-interactive fork auto-simulation**: `handle_queued_executions()` auto-simulates Safe and Governor queued items when `is_fork && !prompts_enabled` (matching Go CLI behavior), instead of saving as queued with a skip message.
2. **Explicit skip mechanism**: `--skip-fork-execution` flag on `treb run` lets users opt out of fork simulation, saving queued items for later `treb fork exec`.
3. **Proposed result visibility**: `display_script_broadcast_summary()` shows "⏳" prefix for governor/safe proposed result lines, making pending items visually distinct.
4. **Re-enable 3 e2e tests**: The 3 ignored `e2e_fork_workflow` tests pass with `--non-interactive` now that live signing (Phase 1) and auto-simulation (this phase) are in place.

## User Stories

### P2-US-001: Add --skip-fork-execution flag to CLI and ExecuteScriptOpts

**Description**: Add a `--skip-fork-execution` boolean flag to the `treb run` command in `crates/treb-cli/src/main.rs` and thread it through `ExecuteScriptOpts` and into `handle_queued_executions()`.

**Changes**:
- Add `skip_fork_execution: bool` field to `Run` variant in `crates/treb-cli/src/main.rs` (with `#[arg(long)]`)
- Add `skip_fork_execution: bool` field to `ExecuteScriptOpts` in `crates/treb-cli/src/commands/run.rs`
- Pass the flag from `run()` function into `ExecuteScriptOpts` construction
- Add `skip_fork_execution: bool` parameter to `handle_queued_executions()` signature
- Pass the flag from the `run()` caller site into `handle_queued_executions()`
- Mirror the flag in `crates/treb-cli/build.rs` for shell completion generation

**Acceptance Criteria**:
- [ ] `treb run --help` shows `--skip-fork-execution` in the flag list
- [ ] `treb run script/X.s.sol --skip-fork-execution` parses without error
- [ ] `ExecuteScriptOpts` carries the `skip_fork_execution` field
- [ ] `handle_queued_executions()` receives the flag (unused in this story, wired in P2-US-002)
- [ ] `cargo clippy --workspace --all-targets` passes
- [ ] `cargo test -p treb-cli` passes (existing tests unaffected)

---

### P2-US-002: Auto-simulate fork executions in non-interactive mode

**Description**: Change `handle_queued_executions()` so that when `is_fork && !prompts_enabled && !skip_fork_execution`, the function auto-simulates (calls `exec_safe_from_registry` / `simulate_governance_on_fork`) without prompting. The current behavior skips simulation and prints "Saved as queued" — the new behavior matches Go CLI where CI gets full simulation.

When `skip_fork_execution` is true, the function should always save as queued regardless of interactive mode, printing the existing "Saved as queued — execute later via `treb fork exec`" message.

**Changes**:
- In `handle_queued_executions()` (`crates/treb-cli/src/commands/run.rs`, ~lines 932-1125):
  - For `SafeProposal` branch: change the `else if is_fork` block (line 1021-1024) to auto-simulate when `!skip_fork_execution`, only skip when `skip_fork_execution` is true
  - For `GovernanceProposal` branch: same change at the `else if is_fork` block (line 1116-1118)
  - When `is_fork && prompts_enabled`: keep existing interactive prompt behavior, but also respect `skip_fork_execution` (if true, skip without prompting)

**Logic table**:
| is_fork | prompts_enabled | skip_fork_execution | Behavior |
|---------|----------------|--------------------|----|
| true | true | false | Prompt user (existing) |
| true | true | true | Save as queued (new) |
| true | false | false | Auto-simulate (new — matches Go) |
| true | false | true | Save as queued (existing message) |
| false | * | * | No action (existing — live mode) |

**Acceptance Criteria**:
- [ ] Non-interactive fork mode auto-simulates Safe queued executions (calls `exec_safe_from_registry`)
- [ ] Non-interactive fork mode auto-simulates Governor queued executions (calls `simulate_governance_on_fork`)
- [ ] `--skip-fork-execution` in fork mode saves as queued without simulation, printing existing skip message
- [ ] `--skip-fork-execution` in interactive fork mode skips the prompt and saves as queued
- [ ] Live mode (non-fork) behavior unchanged regardless of flags
- [ ] `cargo clippy --workspace --all-targets` passes
- [ ] `cargo test -p treb-cli` passes

---

### P2-US-003: Update display_script_broadcast_summary for proposed results

**Description**: Update `display_script_broadcast_summary()` in `crates/treb-cli/src/commands/run.rs` to prefix governor/safe proposed result lines with "⏳" (the hourglass emoji already available as `emoji::HOURGLASS`), making pending items visually distinct from completed transactions.

**Changes**:
- In `display_script_broadcast_summary()` (~lines 1644-1677), prefix each proposed result detail line with `emoji::HOURGLASS`:
  - Safe proposed: `"⏳ {sender_role} proposed to Safe (safeTxHash=..., nonce=...)"`
  - Governor proposed: `"⏳ {sender_role} proposed to Governor ... (proposal=...)"`
- Keep the existing yellow color styling

**Acceptance Criteria**:
- [ ] Safe proposed lines in broadcast summary start with "⏳"
- [ ] Governor proposed lines in broadcast summary start with "⏳"
- [ ] Existing color behavior preserved (yellow when color enabled, plain when not)
- [ ] `cargo clippy --workspace --all-targets` passes
- [ ] `cargo test -p treb-cli` passes
- [ ] Any affected golden files updated (run `UPDATE_GOLDEN=1 cargo test -p treb-cli` if needed)

---

### P2-US-004: Re-enable e2e_fork_workflow tests

**Description**: Remove the `#[ignore]` attribute from the 3 `e2e_fork_workflow` tests in `crates/treb-cli/tests/e2e_fork_workflow.rs`. These tests use `--non-interactive` via the shared `run_deployment()` helper (which passes `--non-interactive` to `treb run`). With Phase 1's live signing and this phase's non-interactive auto-simulation, these tests should now pass.

**Tests to re-enable**:
1. `e2e_fork_enter_deploy_diff_revert_exit` (line 81)
2. `e2e_fork_enter_deploy_exit_restores_state` (line 199)
3. `e2e_fork_status_shows_active_fork` (line 279)

**Changes**:
- Remove `#[ignore] // TODO: re-enable after live broadcast signing is implemented` from all 3 tests
- Run each test individually to verify it passes
- Fix any test failures caused by output changes from P2-US-003 (proposed result display)

**Acceptance Criteria**:
- [ ] All 3 tests have `#[ignore]` removed
- [ ] `e2e_fork_enter_deploy_diff_revert_exit` passes: full lifecycle (enter → deploy → diff → revert → exit) with all 11 verification steps
- [ ] `e2e_fork_enter_deploy_exit_restores_state` passes: registry state properly restored on fork exit
- [ ] `e2e_fork_status_shows_active_fork` passes: `fork status --json` reports network, chainId, rpcUrl
- [ ] `cargo test -p treb-cli --test e2e_fork_workflow` passes (all 3 tests)
- [ ] `cargo test --workspace --all-targets` passes (no regressions)

## Functional Requirements

- **FR-1**: `handle_queued_executions()` must auto-simulate Safe and Governor queued items on fork when non-interactive, unless `--skip-fork-execution` is set.
- **FR-2**: `--skip-fork-execution` flag must be available on `treb run` and must suppress fork simulation regardless of interactive mode.
- **FR-3**: `--skip-fork-execution` must be reflected in shell completions (`build.rs`).
- **FR-4**: `display_script_broadcast_summary()` must prefix proposed result lines with "⏳" for visual distinction.
- **FR-5**: The 3 `e2e_fork_workflow` tests must pass without `#[ignore]`.
- **FR-6**: Interactive fork mode prompt behavior must remain unchanged when `--skip-fork-execution` is not set.
- **FR-7**: Live mode (non-fork) behavior must remain unchanged regardless of `--skip-fork-execution`.

## Non-Goals

- **No changes to `treb fork exec`**: The `fork exec` command itself is not modified in this phase; only the `treb run` pipeline's queued execution handling changes.
- **No new fork simulation logic**: Auto-simulation reuses the existing `exec_safe_from_registry()` and `simulate_governance_on_fork()` code paths — no new simulation implementation.
- **No compose command changes**: The `--skip-fork-execution` flag is only added to `treb run`, not `treb compose`.
- **No JSON output changes**: The `--json` output format is not modified; the "⏳" display change is human-output only.
- **No Safe/Governor contract deployment**: Real Safe/Governor test fixtures are deferred to Phases 6-7.

## Technical Considerations

### Dependencies
- **Phase 1 (completed)**: Live signing via `alloy-provider` is in place. The e2e tests depend on `--non-interactive --broadcast` working against Anvil, which Phase 1 delivered.

### Key Files
| File | Changes |
|------|---------|
| `crates/treb-cli/src/main.rs` | Add `--skip-fork-execution` to `Run` variant |
| `crates/treb-cli/build.rs` | Mirror `--skip-fork-execution` for completions |
| `crates/treb-cli/src/commands/run.rs` | `ExecuteScriptOpts` field, `handle_queued_executions()` logic, `display_script_broadcast_summary()` display |
| `crates/treb-cli/tests/e2e_fork_workflow.rs` | Remove `#[ignore]` from 3 tests |

### Integration Points
- `handle_queued_executions()` is called from `run()` in `run.rs` (line ~903) — the `skip_fork_execution` flag flows through `ExecuteScriptOpts` which is constructed in `run()`.
- `display_script_broadcast_summary()` is called from both `run.rs` and `compose.rs` — changes apply to both command paths.
- The `run_deployment()` test helper in `crates/treb-cli/tests/e2e/mod.rs` passes `--non-interactive` to `treb run` — the auto-simulation behavior is what unblocks the e2e tests.

### Conventions (from CLAUDE.md)
- Mirror new `--skip-fork-execution` flag in both `main.rs` `#[arg(long)]` and `build.rs` for shell completions
- Use `after_long_help` for contextual help footers, not `after_help`
- Refresh any affected golden files with `UPDATE_GOLDEN=1`
