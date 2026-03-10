# PRD: Phase 8 - compose Command Output

## Introduction

Phase 8 ports the compose command's human-readable output from the Go CLI (`render/compose.go`) to the Rust CLI (`commands/compose.rs`) to achieve exact 1:1 output parity. The compose command orchestrates multi-component deployments in dependency order — its output covers the execution plan, per-step results, and a final summary. This phase builds on Phase 6 (run command output), which established the script execution display patterns that compose wraps.

The Rust compose command already has a working implementation with dry-run plan display, per-step progress, and result summary. However, the styling and formatting diverge from Go: the plan uses different colors (LABEL/MUTED instead of Cyan/Green), the execution-time output uses `print_stage` instead of Go's step-result pattern, the summary uses a flat totals line instead of Go's `═══` separator with structured stats, and env vars are not displayed.

## Goals

1. **Execution plan matches Go exactly**: Plan header uses `🎯` + `📋` with bold cyan styling; numbered steps show component name in cyan, script in green, dependencies in gray; env vars shown in yellow — matching `render/compose.go` lines 37-68.
2. **Per-step result matches Go exactly**: Success shows `✓ Step completed successfully` in green with deployment count; failure shows `❌ Failed: {error}` in red — matching `render/compose.go` lines 71-87.
3. **Summary matches Go exactly**: 70-char `═` separator, `🎉` success or `❌` failure with bold styling, `📊 Summary:` section with bullet-point stats — matching `render/compose.go` lines 90-116.
4. **Golden test files updated**: All compose golden files reflect the new Go-matching output format.
5. **No behavioral changes**: JSON output, validation, topological sort, resume, dump-command — all remain unchanged.

## User Stories

### P8-US-001: Port Execution Plan Header and Component List Styling

**Description:** Update `print_dry_run_plan()` to match Go's `RenderExecutionPlan()` format exactly. The Go version uses `🎯 Orchestrating {group}` on line 1, `📋 Execution plan: N components` on line 2, then a bold `📋 Execution Plan:` header followed by a 50-char `─` separator. Each step shows the component name in cyan, script in green, and dependencies in gray (using Go's `color.FgHiBlack`). Env vars are displayed in yellow on a continuation line.

**Changes:**
- `crates/treb-cli/src/commands/compose.rs` — Update `print_dry_run_plan()`:
  - Change plan header line 1 to: `\n🎯 Orchestrating {group}`
  - Change plan header line 2 to: `📋 Execution plan: N components\n`
  - Add bold `📋 Execution Plan:` label before separator (currently missing the `📋` emoji and bold)
  - Change component name color from `color::LABEL` (bold) to cyan (`color::STAGE` without bold, or `Style::new().cyan()`)
  - Change script color from `color::MUTED` (dimmed) to green (`Style::new().green()`)
  - Change dependency text color to gray (`color::GRAY` / bright black)
  - Add env vars display: after the `→ script (depends on: [...])` line, if the component has env vars, print `   Env: {map}` in yellow on the next line
  - Remove step number styling (Go uses plain `N.`, not bold)

**Acceptance Criteria:**
- Plan header shows `🎯 Orchestrating {group}` and `📋 Execution plan: N components` as separate lines
- Bold `📋 Execution Plan:` label appears before the 50-char separator
- Component names render in cyan
- Script paths render in green
- Dependencies render in gray/bright-black
- Step numbers are plain (not bold)
- Env vars display in yellow on a continuation line with `   Env: {map}` format
- Skipped steps still render with warning styling
- `cargo clippy --workspace --all-targets` passes
- `cargo test -p treb-cli -- compose` passes (some golden tests may now fail — that is expected until P8-US-005)

---

### P8-US-002: Port Execution-Time Progress Output to Match Go Step Results

**Description:** Update the execution loop's human-readable output to match Go's `RenderStepResult()` and `RenderExecutionPlan()` patterns. Currently, the Rust code uses `output::print_stage()` for orchestration header, skip messages, execution start, and completion — producing `🚀 Orchestrating...`, `⏭️ [N/N] Skipping...`, `🔨 [N/N] Executing...`, `✅ [N/N] ... completed` format. The Go version renders the execution plan at the start (same as dry-run), then shows per-step results: `✓ Step completed successfully` (green) with `  Created N deployment(s)` count, or `❌ Failed: {error}` (red).

**Changes:**
- `crates/treb-cli/src/commands/compose.rs` — Update the `run()` function's execution loop:
  - Before the execution loop, call the plan display function (same as dry-run) to show the execution plan header + numbered component list
  - Replace `print_stage("🚀", "Orchestrating...")` with the plan display call
  - Replace per-step `print_stage("🔨", "[N/N] Executing...")` with a component execution header (e.g., step number + name, matching what Go shows via real-time rendering)
  - On success: replace `print_stage("✅", "[N/N] ... completed")` with `✓ Step completed successfully` in green + optional `  Created N deployment(s)` line (matching `RenderStepResult` lines 76-86)
  - On failure: replace `eprintln!("Component '{}' failed...")` with `❌ Failed: {error}` in red (matching `RenderStepResult` line 74)
  - Replace skip messages `print_stage("⏭️", "[N/N] Skipping...")` — Go doesn't show skip messages during execution (skipped components are visible in the plan)
  - Replace final `print_stage("✅", "Orchestration complete.")` / `print_stage("❌", "Orchestration failed.")` — Go delegates this to `renderSummary()`

**Acceptance Criteria:**
- Execution plan is displayed before the execution loop starts (same format as dry-run plan)
- Per-step success shows `✓ Step completed successfully` in green
- Deployment count shows `  Created N deployment(s)` when deployments > 0
- Per-step failure shows `❌ Failed: {error}` in red
- No `[N/N]` progress counters in step results (Go doesn't use them)
- Skip messages removed from execution loop (plan shows skipped status)
- `cargo clippy --workspace --all-targets` passes
- `cargo test -p treb-cli -- compose` passes (golden files may still be outdated)

---

### P8-US-003: Port Summary Section with Separator and Stats

**Description:** Update `display_compose_human()` to match Go's `renderSummary()` format exactly. Go uses a 70-char `═` separator, then `🎉 Successfully orchestrated {group} deployment` (green bold) on success or `❌ Orchestration failed` (red bold) on failure, followed by a `📊 Summary:` section with bullet-point stats.

**Changes:**
- `crates/treb-cli/src/commands/compose.rs` — Rewrite `display_compose_human()`:
  - Replace the current "Compose results: {group}" header + per-component detail lines + flat totals line with Go's summary format
  - Print 70-char `═` separator line: `"═".repeat(70)`
  - On success: print `🎉 Successfully orchestrated {group} deployment` in green bold
  - On success: print `\n📊 Summary:` header
  - On success: print `  • Steps executed: N/M` (executed vs total components)
  - On success: print `  • Total deployments: N`
  - On failure: print `❌ Orchestration failed` in red bold
  - On failure: print `\n📊 Summary:` header
  - On failure: print `  • Failed at step: {component_name}`
  - On failure: print `  • Steps completed: N/M` (completed count excludes the failed step)
  - On failure: print `  • Error: {error_message}` if error is available
  - The function signature may need to change to accept total component count and failed-step info (the Go `ComposeResult` has `ExecutedSteps`, `Plan.Components`, `FailedStep`)

**Acceptance Criteria:**
- 70-char `═` separator line appears before summary
- Success shows `🎉 Successfully orchestrated {group} deployment` in green bold
- Success shows `📊 Summary:` with steps executed (N/M) and total deployments
- Failure shows `❌ Orchestration failed` in red bold
- Failure shows `📊 Summary:` with failed step name, steps completed (N/M), and error message
- Per-component detail lines removed from summary (Go doesn't show them here — they appear in real-time step results)
- `cargo clippy --workspace --all-targets` passes
- `cargo test -p treb-cli -- compose` passes (golden files may still be outdated)

---

### P8-US-004: Port Env Vars Display in Plan and Add Missing Emoji

**Description:** Ensure env vars from compose components are displayed in the execution plan, and verify all emoji usage matches Go. The Go version shows `   Env: {map}` in yellow for components with env vars (line 59-62 of render/compose.go). Also verify that the `🎯` (TARGET) and `📋` (CLIPBOARD) and `📊` (CHART) and `🎉` (PARTY) emoji constants are available and used correctly.

**Changes:**
- `crates/treb-cli/src/commands/compose.rs` — Verify/add env var display in `print_dry_run_plan()`:
  - After printing the step line (component → script + deps), check if the component has env vars
  - If env vars exist, print `   Env: {map}` in yellow on the next line
  - Use Go's `%v` format for the map (Rust equivalent: `{:?}` for HashMap debug format, or format as `{KEY1: VALUE1, KEY2: VALUE2}`)
  - Also display env vars during the execution plan shown before the loop (same function)
- Verify emoji constants `TARGET` (🎯), `CLIPBOARD` (📋), `CHART` (📊), `PARTY` (🎉) exist in `ui/emoji.rs` — they do per the exploration

**Acceptance Criteria:**
- Components with env vars show `   Env: {map}` in yellow on a continuation line
- Components without env vars show no env line
- Env display uses yellow color (`color::WARNING` or `Style::new().yellow()`)
- Map format matches Go's `%v` output (e.g., `map[KEY1:VALUE1 KEY2:VALUE2]` or equivalent Rust debug format)
- All emoji match Go exactly: 🎯, 📋, 📊, 🎉, ✓, ❌
- `cargo clippy --workspace --all-targets` passes

**Note:** This story may be combined with P8-US-001 during implementation if the env var display is trivial to add alongside the color changes. If P8-US-001 already handles env vars fully, this story becomes a verification-only pass.

---

### P8-US-005: Update All Golden Test Expected Files

**Description:** Regenerate all compose golden test `.expected` files to match the new Go-parity output format. Verify that all compose tests pass, and check for any ripple effects on other test suites.

**Changes:**
- Run `UPDATE_GOLDEN=1 cargo test -p treb-cli --test integration_compose` to regenerate compose golden files
- Verify the diff: every golden file should show the format changes from P8-US-001 through P8-US-004 (new emoji, new colors, new summary format)
- Run `cargo test --workspace --all-targets` to verify no other tests are broken
- Manually verify golden file content matches Go output format

**Acceptance Criteria:**
- All compose golden tests pass: `cargo test -p treb-cli --test integration_compose`
- All CLI tests pass: `cargo test -p treb-cli`
- Full workspace tests pass: `cargo test --workspace --all-targets`
- `cargo clippy --workspace --all-targets` passes
- Golden file diffs show only intentional format changes (no unrelated drift)
- Dry-run golden files show: `🎯 Orchestrating`, `📋 Execution plan:`, bold `📋 Execution Plan:`, cyan component names, green scripts, gray deps
- Error golden files are unchanged (error output format is not being ported in this phase)

## Functional Requirements

- **FR-1:** Plan header format: `\n🎯 Orchestrating {group}\n📋 Execution plan: N components\n\n📋 Execution Plan:\n{50 dashes}\n`
- **FR-2:** Plan step format: `N. {CYAN component} → {GREEN script}` with optional ` {GRAY (depends on: [deps])}` and optional `\n   {YELLOW Env: {map}}`
- **FR-3:** Step success result: `{GREEN ✓ Step completed successfully}\n` with optional `  Created N deployment(s)\n`
- **FR-4:** Step failure result: `{RED ❌ Failed: {error}}\n`
- **FR-5:** Success summary: `{70 ═}\n{GREEN BOLD 🎉 Successfully orchestrated {group} deployment}\n\n📊 Summary:\n  • Steps executed: N/M\n  • Total deployments: N\n`
- **FR-6:** Failure summary: `{70 ═}\n{RED BOLD ❌ Orchestration failed}\n\n📊 Summary:\n  • Failed at step: {name}\n  • Steps completed: N/M\n  • Error: {msg}\n`
- **FR-7:** JSON output format unchanged — `ComposeOutputJson` struct and `PlanEntry` serialization remain identical
- **FR-8:** Dry-run banner unchanged: `🚧 [DRY RUN] Showing execution plan only — no changes will be made.`
- **FR-9:** Dump-command output unchanged
- **FR-10:** Resume state handling unchanged

## Non-Goals

- **No JSON schema changes.** The `ComposeOutputJson`, `PlanEntry`, `ComponentResultEntry`, and `ComposeTotals` structs are not modified.
- **No behavioral changes.** Topological sort, validation, resume state, broadcast confirmation, dump-command — all unchanged.
- **No verbose mode changes.** The verbose per-component context (`eprint_kv` with Script/Namespace/RPC/etc.) is not being ported to Go format in this phase.
- **No error message format changes.** Validation errors, file-not-found errors, and other error paths retain their current format.
- **No debug log format changes.** The per-component `.log` files written in debug mode are not changed.

## Technical Considerations

- **Dependency on Phase 6:** The compose command delegates per-component execution to the same pipeline as the `run` command. Phase 6 already ported `run`'s output format, so the per-component execution output (transactions, deployments) rendered by the pipeline is already Go-matching. Phase 8 only changes the compose-level wrapper output (plan, step results, summary).
- **Existing helpers:** Use `emoji::TARGET`, `emoji::CLIPBOARD`, `emoji::CHART`, `emoji::PARTY` from `ui/emoji.rs`. Use `color::STAGE` (cyan bold) for headers, `color::SUCCESS` (green bold) for success banners, `color::ERROR` (red bold) for failure banners, `color::GRAY` (bright black) for dependency text, `color::WARNING` (yellow) for env vars.
- **Output destination:** Plan and progress go to stderr (`eprintln!`); JSON goes to stdout. This matches the existing pattern established in Phases 6 and 7.
- **Golden test targeting:** Use `--test integration_compose` to regenerate only compose golden files without touching other test binaries.
- **Summary function signature:** The current `display_compose_human()` takes `(group, results, totals)` but Go's `renderSummary` needs `ExecutedSteps` count, total component count, and failed step info. The function signature will need to be extended or a new summary function created.
- **Env var map formatting:** Go's `%v` for `map[string]string` produces `map[KEY1:VALUE1 KEY2:VALUE2]`. Rust's `{:?}` for `HashMap` produces `{"KEY1": "VALUE1", "KEY2": "VALUE2"}`. The exact format should match what Go produces, but since this is a display detail, using Rust's debug format is acceptable if Go parity is not achievable without a custom formatter.
