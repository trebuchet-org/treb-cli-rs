# PRD: Phase 12 - gen-deploy and migrate Command Output

## Introduction

Phase 12 ports the human-readable output of the `gen-deploy` and `migrate` commands from the Go CLI to the Rust CLI, achieving exact output parity where the underlying functionality is shared. The `gen-deploy` command has minimal output (a success line plus deployment instructions) that must match Go exactly. The `migrate` command in Rust has different subcommands (`config`, `registry`) than the Go version (single command), so parity focuses on matching the output FORMAT and STYLE for shared patterns: success messages, preview labels, warning format, cancellation text, cleanup messages, and next-steps sections.

This phase depends on Phase 1 (shared color palette, emoji constants, format helpers) which is already complete.

## Goals

1. **gen-deploy success message matches Go exactly**: `\n✅ Generated deployment script: PATH\n` followed by type-specific instruction lines and a "To deploy, run:" command.
2. **migrate config success/completion output matches Go style**: Green bold `✓` success messages instead of `✅` stage emoji, preview header, cancellation text, cleanup messages, and numbered next-steps section.
3. **migrate config warning output matches Go format**: Yellow warning for existing treb.toml, matching Go's `Warning: treb.toml already exists.` text and color.
4. **All golden test expected files updated** to reflect the new output format.
5. **All existing unit and integration tests pass** with updated assertions.

## User Stories

### P12-US-001: Port gen-deploy Human Output to Match Go Format

**Description:** Update the gen-deploy success message and add instruction lines after script generation to match Go's `render/generate.go` output exactly.

**Changes:**
- In `crates/treb-cli/src/commands/gen_deploy.rs`:
  - Change success message from `"Generated deploy script: {path}"` to `"Generated deployment script: {path}"` (line ~885)
  - Add a leading newline before the success message (Go uses `\n✅`)
  - After the success message, print instruction lines based on script type:
    - Library: `"This library will be deployed with CREATE2 for deterministic addresses."`
    - Proxy: `"This script will deploy both the implementation and proxy contracts."` + `"Make sure to update the initializer parameters if needed."`
    - Then for all types: empty line, `"To deploy, run:"`, `"  treb run {script_path} --network <network>"`

**Go reference:** `internal/cli/render/generate.go:16-23` (Render function), `internal/usecase/generate_deployment_script.go:193-212` (buildInstructions)

**Acceptance criteria:**
- Success message reads `✅ Generated deployment script: PATH` (not "deploy script")
- Leading newline before the success emoji
- Type-specific instruction lines printed after success (library, proxy, or none for plain contract)
- "To deploy, run:" line with `treb run PATH --network <network>` printed for all types
- `--json` output unchanged (instructions are human-only)
- Typecheck passes (`cargo check -p treb-cli`)

### P12-US-002: Port migrate config Success, Cleanup, and Next Steps Output to Go Format

**Description:** Update the migrate config command's success messages, cleanup output, and add a next-steps section to match Go's `migrate.go` output style.

**Changes:**
- In `crates/treb-cli/src/commands/migrate.rs`:
  - Change success message from `print_stage("✅", "Migration complete — treb.toml updated to v2 format.")` to green bold `✓ treb.toml written successfully\n` via `println!` with `styled()` (matching Go line 158: `green.Printf("✓ treb.toml written successfully\n")`)
  - After foundry cleanup succeeds, print green bold `✓ foundry.toml cleaned up\n` (matching Go line 168)
  - When `cleanup_foundry` is requested but user declines (interactive path, if applicable) or no sections found, consider Go's skip message: `"Skipped foundry.toml cleanup — you can remove [profile.*.treb.*] sections manually."`
  - Add "Next steps:" section after success (matching Go lines 176-184):
    - `"Next steps:"`
    - `"  1. Review the generated treb.toml"`
    - If foundry not cleaned up: `"  2. Remove [profile.*.treb.*] sections from foundry.toml"` + `"  3. Run \`treb config show\` to verify your config is loaded correctly"`
    - If foundry cleaned up: `"  2. Run \`treb config show\` to verify your config is loaded correctly"`
  - Change interactive preview header to print `"Generated treb.toml:"` before showing content (matching Go line 142)
  - Change cancellation message from `"Cancelled."` to `"Migration cancelled."` (matching Go line 147)

**Go reference:** `internal/cli/migrate.go:157-184` (success, cleanup, next steps)

**Acceptance criteria:**
- Success message is green bold `✓ treb.toml written successfully` (not stage-format `✅ Migration complete...`)
- Foundry cleanup success prints green bold `✓ foundry.toml cleaned up`
- Next-steps section printed with correct numbering (2 items if cleaned up, 3 if not)
- Preview shows `"Generated treb.toml:"` header before TOML content
- Cancellation text is `"Migration cancelled."`
- `--json` output unchanged
- Typecheck passes (`cargo check -p treb-cli`)

### P12-US-003: Port migrate config Warning Output to Go Format

**Description:** Update the warning output for existing treb.toml to match Go's yellow warning format, and align the foundry-only migration warning.

**Changes:**
- In `crates/treb-cli/src/commands/migrate.rs`:
  - For the existing treb.toml warning in non-interactive mode (Go line 129): print `"Warning: treb.toml already exists and will be overwritten."` to stderr
  - For the existing treb.toml warning in interactive mode (Go lines 131-132): print yellow `"Warning: treb.toml already exists."` to stderr, followed by confirmation prompt `"Overwrite existing treb.toml?"`
  - Note: The Rust `migrate config` command currently does not check for existing treb.toml before overwrite in the v1→v2 path (it creates a backup instead). Add an explicit warning message before backup creation when treb.toml already exists, matching Go behavior.
  - The foundry-only migration warning (`"No treb.toml found — migrating senders from foundry.toml (deprecated location)."`) already exists in Rust but uses `format_warning_banner()` — verify it matches Go's format.

**Go reference:** `internal/cli/migrate.go:127-137` (existing treb.toml warning)

**Acceptance criteria:**
- When treb.toml exists and non-interactive: `"Warning: treb.toml already exists and will be overwritten."` printed to stderr
- When treb.toml exists and interactive: yellow `"Warning: treb.toml already exists."` to stderr + overwrite confirmation prompt
- Existing foundry-only warning message format verified against Go
- `--json` output unchanged
- Typecheck passes (`cargo check -p treb-cli`)

### P12-US-004: Update Unit and Integration Tests for gen-deploy and migrate Output Changes

**Description:** Update test assertions across all test files that check gen-deploy and migrate human output to match the new format.

**Changes:**
- In `crates/treb-cli/tests/cli_gen_deploy.rs`: Update any `stderr` or `stdout` predicate assertions that match on "Generated deploy script" to "Generated deployment script". Update assertions to account for instruction lines in output.
- In `crates/treb-cli/tests/integration_gen_deploy.rs`: No code changes needed (golden-file-based tests updated in US-005).
- In `crates/treb-cli/tests/integration_migrate.rs`: No code changes needed (golden-file-based tests updated in US-005).
- In `crates/treb-cli/tests/cli_prune_reset_migrate.rs`: Update any assertions that match on migrate output text (e.g., "Migration complete", "Cancelled").
- In `crates/treb-cli/src/commands/migrate.rs` (unit tests): Update assertions in `write_or_print_v2_cancelled_prompt_does_not_write` and other unit tests that check output text.
- In `crates/treb-cli/tests/e2e_workflow.rs` and `crates/treb-cli/tests/e2e/mod.rs`: Check for any migrate/gen-deploy assertions that need updating.

**Acceptance criteria:**
- All unit tests pass: `cargo test -p treb-cli --lib`
- All integration tests (non-golden) pass: `cargo test -p treb-cli --test cli_gen_deploy --test cli_prune_reset_migrate`
- No test assertions reference old output text ("Generated deploy script", "Migration complete", "Cancelled.")
- Typecheck passes (`cargo check -p treb-cli --all-targets`)

### P12-US-005: Update All Golden Test Expected Files

**Description:** Regenerate golden test expected files for gen-deploy and migrate commands, then verify all tests pass.

**Changes:**
- Run `UPDATE_GOLDEN=1 cargo test -p treb-cli --test integration_gen_deploy` to regenerate gen-deploy golden files (17 variants)
- Run `UPDATE_GOLDEN=1 cargo test -p treb-cli --test integration_migrate` to regenerate migrate golden files (8 variants)
- Run full test suite `cargo test -p treb-cli` to verify no cross-command golden drift
- Inspect diffs to confirm changes match expected output format (success message text, instruction lines, next-steps section, etc.)

**Acceptance criteria:**
- All 17 gen-deploy golden files updated with "Generated deployment script" text and instruction lines
- All 8 migrate golden files updated with new success/warning/next-steps format
- Full `cargo test -p treb-cli` passes with zero failures
- `cargo clippy --workspace --all-targets` passes with no warnings
- Git diff of golden files reviewed — changes are limited to expected output format changes

## Functional Requirements

**FR-1:** gen-deploy success message must read `\n✅ Generated deployment script: PATH\n` with a leading newline, matching Go `render/generate.go:18`.

**FR-2:** gen-deploy must print type-specific instruction lines after the success message: library instructions for libraries, proxy instructions for proxy deployments, and a "To deploy, run:" command line for all types, matching Go `usecase/generate_deployment_script.go:193-212`.

**FR-3:** migrate config success must print green bold `✓ treb.toml written successfully` instead of the current stage-format message, matching Go `migrate.go:158`.

**FR-4:** migrate config foundry cleanup success must print green bold `✓ foundry.toml cleaned up`, matching Go `migrate.go:168`.

**FR-5:** migrate config must print a "Next steps:" section with numbered items after successful migration, with conditional numbering based on whether foundry cleanup was performed, matching Go `migrate.go:176-184`.

**FR-6:** migrate config interactive preview must show `"Generated treb.toml:"` header before TOML content display, matching Go `migrate.go:142`.

**FR-7:** migrate config cancellation must print `"Migration cancelled."` instead of `"Cancelled."`, matching Go `migrate.go:147`.

**FR-8:** migrate config must print a yellow warning when treb.toml already exists (interactive: `"Warning: treb.toml already exists."` + overwrite prompt; non-interactive: `"Warning: treb.toml already exists and will be overwritten."`), matching Go `migrate.go:127-137`.

**FR-9:** All `--json` output schemas must remain unchanged — no JSON format changes in this phase.

**FR-10:** All golden test expected files must be regenerated and pass.

## Non-Goals

- **No interactive account naming**: The Go `migrate` command has interactive account naming (`interactiveAccountNaming`). The Rust `migrate config` command uses a different approach (v1→v2 conversion without renaming prompts). Adding account naming is a feature addition, not output parity.
- **No namespace pruning**: The Go `migrate` has `pruneEmptyNamespaces` interactive flow. This is not present in Rust and would be a feature addition.
- **No JSON schema changes**: This phase only modifies human-readable output. JSON output (`--json`) remains unchanged.
- **No command restructuring**: The Rust `migrate` has `config` and `registry` subcommands while Go has a single `migrate` command. This structural difference is preserved.
- **No changes to migrate registry output**: The `migrate registry` subcommand output is Rust-specific (no Go equivalent) and is not modified in this phase.
- **No removal of Rust-specific stage messages**: The compile/generate stage messages (`🔨 Compiling project...`, `📝 Generating deploy script...`) in gen-deploy and the detection/conversion stages in migrate config are useful UX additions in the Rust CLI. They are not present in Go's render output but are kept as they provide progress feedback.

## Technical Considerations

### Dependencies
- **Phase 1 (complete):** Emoji constants (`crate::ui::emoji`), color styles (`color::SUCCESS`, `color::GREEN`, `color::WARNING`, `color::STAGE`, `color::GRAY`), format helpers (`output::format_success()`, `output::format_warning()`)

### Key Patterns from Previous Phases
- Use `styled(text, style)` helper with `color::is_color_enabled()` check for conditional coloring (from Phase 5, 7, 8, 9, 10, 11)
- Green bold (`color::SUCCESS`) for success messages like `✓ treb.toml written successfully` (from Phase 10: `emoji::CHECK_MARK` is `✓` U+2713, not `✅` U+2705)
- Yellow (`color::WARNING`) for warning messages to stderr (from Phase 3, 10)
- `println!` for human output to stdout, `eprintln!` for progress/warnings to stderr (from Phase 8, 10, 11)
- When removing `styled()` helper or `print_stage()` calls from a command, also remove unused `owo_colors` and `color` imports (from Phase 11)

### Integration Points
- `crates/treb-cli/src/commands/gen_deploy.rs` — main file for gen-deploy output changes
- `crates/treb-cli/src/commands/migrate.rs` — main file for migrate output changes
- `crates/treb-cli/src/output.rs` — shared output utilities (print_stage, format_stage, print_json)
- `crates/treb-cli/src/ui/emoji.rs` — emoji constants (CHECK_MARK ✓, CHECK ✅)
- `crates/treb-cli/src/ui/color.rs` — color styles (SUCCESS, GREEN, WARNING)

### Test Files to Update
- `crates/treb-cli/tests/cli_gen_deploy.rs` — assertion-based gen-deploy tests
- `crates/treb-cli/tests/integration_gen_deploy.rs` — golden-file gen-deploy tests (17 variants)
- `crates/treb-cli/tests/integration_migrate.rs` — golden-file migrate tests (8 variants)
- `crates/treb-cli/tests/cli_prune_reset_migrate.rs` — assertion-based migrate tests
- `crates/treb-cli/src/commands/migrate.rs` — inline unit tests

### Risks
- The gen-deploy instruction lines require knowing the script type (library, proxy, contract) at the point of output — this information is already available in the `context` struct.
- The migrate config overwrite warning requires checking whether treb.toml exists BEFORE deciding to show the warning — the current code creates a backup without warning. This needs reordering of the existing logic.
- Error golden files are stable when changing success/normal output formatting — no updates needed for error-path golden files (from Phase 11 learnings).
