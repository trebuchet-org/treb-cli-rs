# PRD: Phase 5 - Compose Test Fixes

## Introduction

Phase 5 re-enables the 20 ignored compose tests (12 in `cli_compose.rs`, 8 in `integration_compose.rs`) that broke when `--dry-run` and `--dump-command` CLI flags were removed in Phase 1. The core fix is making compose's simulation mode (no `--broadcast`) behave like the old `--dry-run`: parse the YAML, display the execution plan, and exit. This restores plan-only inspection without requiring project initialization or script execution.

This phase depends on Phase 1 (live signing) and builds on Phase 3's re-enabling of 7 integration_compose error-path tests.

## Goals

1. **Re-enable all 20 ignored compose tests** — 12 in `cli_compose.rs` and 8 in `integration_compose.rs` — with zero new test ignores.
2. **Make simulation mode (no `--broadcast`) equivalent to old `--dry-run`** — plan display and exit, no project init required.
3. **Remove dead `--dump-command` code path** from `compose.rs` and delete the 2 golden tests that depended on it.
4. **All workspace tests pass** (`cargo test --workspace --all-targets`) and lint is clean (`cargo clippy --workspace --all-targets`).

## User Stories

### P5-US-001: Make simulation mode (no --broadcast) show execution plan and exit

**Description:** Change `compose::run()` so that omitting `--broadcast` triggers the plan-only display and exits, replacing the old `--dry-run` behavior. Remove the dead `dump_command` code path entirely. Clean up the `dry_run` and `dump_command` parameters from `compose::run()` and the call site in `main.rs`.

**Implementation details:**
- In `compose::run()`, replace `if dry_run { ... }` (line ~850) with `if !broadcast { ... }`. This makes simulation mode (no `--broadcast`) show the plan and exit, exactly as `--dry-run` did.
- Update the `[DRY RUN]` banner text to something appropriate for simulation mode (e.g., `[SIMULATION]` or just the plan without the dry-run label), since there is no `--dry-run` flag anymore.
- Remove the `dump_command` parameter and its entire code block (lines ~873–920) — the feature is hardcoded to `false` with no CLI surface.
- Remove the `dry_run` parameter — it is also hardcoded to `false`.
- Update the `display_session_banner()` signature and `should_prompt_for_broadcast_confirmation()` / `should_reject_interactive_json_broadcast()` to remove the `dry_run` parameter, since `!broadcast` now implies plan-only mode and these functions are only reached when `broadcast` is true.
- Update the call site in `main.rs` (line ~1233) to remove the `false, // dry_run` and `false, // dump_command` arguments.

**Acceptance criteria:**
- `treb compose <file>` (no `--broadcast`) displays the execution plan on stderr and exits 0 without requiring project init.
- `treb compose <file> --json` (no `--broadcast`) outputs the JSON plan on stdout and exits 0.
- `treb compose <file> --broadcast ...` proceeds to execution as before.
- `cargo clippy --workspace --all-targets` passes with no new warnings.
- Typecheck passes (`cargo check --workspace --all-targets`).

---

### P5-US-002: Re-enable plan-display cli_compose tests (9 tests)

**Description:** Remove `#[ignore]` from the 9 `cli_compose.rs` tests that verify plan display and flag parsing behavior. These tests run `compose <file>` without `--broadcast`, which now triggers simulation mode (US-001).

**Tests to re-enable:**
1. `compose_dry_run_shows_plan` (line 150) — plan display with `simple.yaml`
2. `compose_dry_run_chain_shows_correct_order` (line 163) — topological ordering in chain
3. `compose_dry_run_diamond_shows_correct_order` (line 190) — diamond topology ordering
4. `compose_dry_run_json_is_valid` (line 217) — JSON plan structure validation
5. `compose_dry_run_json_chain_has_correct_structure` (line 244) — JSON chain topology
6. `compose_dry_run_json_does_not_include_component_env` (line 276) — env not in JSON output
7. `compose_dry_run_human_formats_component_env_deterministically` (line 308) — sorted env display
8. `compose_all_flags_accepted` (line 342) — all flags parse without error
9. `compose_resume_flag_accepted` (line 380) — `--resume` flag parsing

**Implementation details:**
- Remove `#[ignore]` and the `// TODO: compose tests need updating after --dry-run removal` comments.
- Update assertion strings if the banner text changed in US-001 (e.g., if `[DRY RUN]` becomes `[SIMULATION]`).
- The `compose_dry_run_human_formats_component_env_deterministically` test checks `Env: {"ALPHA": "first", "ZETA": "last"}` in stderr — verify this still appears in plan display output.
- The `compose_all_flags_accepted` test runs with `--json` — verify JSON plan output succeeds (no clap parsing errors).
- Consider renaming test functions to drop the `dry_run_` prefix since the concept is now simulation mode (optional, only if it improves clarity).

**Acceptance criteria:**
- All 9 tests pass with `cargo test -p treb-cli --test cli_compose`.
- No `#[ignore]` annotations remain on these 9 tests.
- Typecheck passes.

---

### P5-US-003: Re-enable execution-failure cli_compose tests (3 tests)

**Description:** Remove `#[ignore]` from the 3 `cli_compose.rs` tests that verify error rendering during compose execution. These tests need `--broadcast` added to their commands because execution only proceeds when `--broadcast` is passed (US-001 makes non-broadcast = plan-only).

**Tests to re-enable:**
1. `compose_json_execution_failure_emits_only_wrapped_json_error` (line 438) — JSON error wrapping on component failure
2. `compose_setup_failure_uses_component_failed_renderer` (line 480) — component failure rendering with env error
3. `compose_resume_failure_shows_resume_banner_and_step_start` (line 544) — resume banner + step-start on failure

**Implementation details:**
- Remove `#[ignore]` and TODO comments.
- Add `--broadcast` to each test's command arguments so compose enters the execution path. For example:
  - `compose_json_execution_failure`: add `"--broadcast"` to args after `"--network", "localhost"`
  - `compose_setup_failure`: add `"--broadcast"` to args
  - `compose_resume_failure`: add `"--broadcast"` to args
- These tests run against `--network localhost` (which points to `http://localhost:8545` in the test `COMPOSE_EXEC_FOUNDRY_TOML`) — they fail at script compilation (NonExistent.s.sol), not at RPC, so no running Anvil is needed.
- Update assertion strings if any stderr output changed (e.g., mode display in banner).
- The `compose_resume_failure` test writes a `compose-state.json` with a `deployment_total` field — verify this is still needed by current resume state loading.

**Acceptance criteria:**
- All 3 tests pass with `cargo test -p treb-cli --test cli_compose`.
- Each test verifies the correct error rendering (JSON wrapped errors, component failure renderer, resume banner).
- No `#[ignore]` annotations remain on these 3 tests.
- Typecheck passes.

---

### P5-US-004: Re-enable integration_compose golden tests and clean up dump-command tests

**Description:** Re-enable the 6 dry-run golden tests by removing `--dry-run` from their commands and refreshing golden files. Delete the 2 `--dump-command` golden tests and their golden directories since that feature is fully removed.

**Golden tests to re-enable (6):**
1. `compose_dry_run_single` (line 49) — single component plan
2. `compose_dry_run_simple` (line 67) — two independent components
3. `compose_dry_run_chain` (line 84) — linear chain topology
4. `compose_dry_run_diamond` (line 102) — diamond topology
5. `compose_dry_run_json_simple` (line 122) — JSON plan for simple topology
6. `compose_dry_run_json_chain` (line 140) — JSON plan for chain topology

**Golden tests to delete (2):**
1. `compose_dump_command_simple` (line 187) — per-component forge commands (feature removed)
2. `compose_dump_command_chain` (line 205) — per-component forge commands (feature removed)

**Implementation details:**
- For the 6 dry-run tests: remove `#[ignore]`, remove `"--dry-run"` from `.test(&[...])` args so the command is `compose <file>` (or `compose <file> --json`).
- Refresh golden files with `UPDATE_GOLDEN=1 cargo test -p treb-cli --test integration_compose` after US-001 changes are in place.
- Golden file `commands.golden` headers will change (no `--dry-run` in argv), and stderr output will change if the `[DRY RUN]` banner text was updated.
- Delete the 2 dump-command test functions from `integration_compose.rs`.
- Delete the golden directories: `tests/golden/compose_dump_command_simple/` and `tests/golden/compose_dump_command_chain/`.
- Update the module-level doc comment in `integration_compose.rs` to remove references to `--dump-command`.

**Acceptance criteria:**
- All 6 re-enabled golden tests pass with `cargo test -p treb-cli --test integration_compose`.
- The 2 dump-command tests and their golden directories are deleted.
- Golden files accurately capture current simulation output (plan display for human, JSON array for `--json`).
- No `#[ignore]` annotations remain in `integration_compose.rs`.
- `UPDATE_GOLDEN=1` produces stable, deterministic golden files.
- Typecheck passes.

## Functional Requirements

- **FR-1:** `treb compose <file>` without `--broadcast` must display the execution plan and exit 0, without requiring `treb init` or a Foundry project.
- **FR-2:** `treb compose <file> --json` without `--broadcast` must output a JSON array of plan steps on stdout, each with `step`, `component`, `script`, `deps` fields.
- **FR-3:** `treb compose <file> --broadcast --network <net>` must proceed to execution as before (parse, resolve config per component, execute scripts, broadcast).
- **FR-4:** Human plan display must show components in topological order with step numbering, dependency annotations, and per-component env vars in sorted order.
- **FR-5:** The `dump_command` code path and parameter must be removed from `compose::run()` since it is unreachable (hardcoded to `false` in `main.rs`).
- **FR-6:** The `dry_run` parameter must be removed from `compose::run()` since simulation behavior is now determined by `!broadcast`.
- **FR-7:** All 20 previously-ignored compose tests must pass without `#[ignore]`.

## Non-Goals

- **No new compose features.** This phase only fixes tests and cleans up dead code.
- **No compose broadcast/execution testing.** Full compose execution with `--broadcast` against Anvil is out of scope (future phases cover Safe/Governor compose flows).
- **No SessionPipeline changes.** The compose execution pipeline (`SessionPipeline`) is untouched — only the pre-execution plan display and parameter cleanup are in scope.
- **No compose checkpoint saves.** Phase 3 noted that SessionPipeline doesn't use pre-routing sequence — that integration point is deferred.
- **No `--dry-run` or `--dump-command` replacement features.** These flags are permanently removed; simulation mode (`!broadcast`) replaces `--dry-run`, and `--dump-command` has no replacement.

## Technical Considerations

**Dependencies:**
- Phase 1 (live signing) must be merged — the compose execution path uses the broadcast pipeline established in P1.
- Phase 3 re-enabled 7 integration_compose tests (error paths) — those must remain passing.

**Integration points:**
- `compose::run()` signature change affects `main.rs` call site (line ~1233). Both `dry_run: false` and `dump_command: false` arguments are removed.
- `display_session_banner()` in `compose.rs` takes a `dry_run` parameter used for mode display — after removing it, the function should derive mode from `broadcast` alone.
- `should_prompt_for_broadcast_confirmation()` and `should_reject_interactive_json_broadcast()` take `dry_run` — remove the parameter since these are only reached in the `broadcast` branch now.
- `super::run::deployment_banner_mode(dry_run, broadcast, is_fork)` is called from compose — verify that passing `false` for `dry_run` (or refactoring the call) still produces correct mode labels.

**Constraints:**
- The `compose-project` test fixture is empty (`.gitkeep` only) — plan-display tests must NOT require a Foundry project, which is why simulation mode exits before `ensure_initialized()`.
- Golden files must be refreshed with `UPDATE_GOLDEN=1` after the behavioral change. The `commands.golden` headers will drop `--dry-run` from the recorded argv.
- The `[DRY RUN]` banner text in `format_warning_banner()` call (line ~858) should be updated since `--dry-run` is no longer a user-facing concept.

**Test matrix after this phase:**
| Test file | Total | Enabled | Ignored |
|-----------|------:|--------:|--------:|
| `cli_compose.rs` | 25 | 25 | 0 |
| `integration_compose.rs` | 13 (was 15, 2 deleted) | 13 | 0 |
