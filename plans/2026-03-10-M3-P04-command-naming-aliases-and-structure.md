# PRD: Phase 4 - Command Naming, Aliases, and Structure

## Introduction

Phase 4 aligns the Rust CLI's command names, subcommand structure, and aliases with the Go CLI so that existing scripts, CI pipelines, and documentation targeting the Go CLI work unmodified with the Rust CLI. This is a pure CLI surface change — no registry, config, or domain logic modifications are needed.

The Go CLI uses `treb gen deploy`, `treb completion`, bare `treb config` (defaults to show), and `treb ls`. The Rust CLI currently uses `treb gen-deploy`, `treb completions`, requires `treb config show`, and already has the `ls` alias. This phase closes those gaps while preserving backward compatibility for existing Rust CLI users.

**Key reference files:**
- Go: `../treb-cli/internal/cli/generate.go`, `root.go`, `config.go`, `list.go`
- Rust: `crates/treb-cli/src/main.rs`, `crates/treb-cli/src/commands/gen_deploy.rs`, `crates/treb-cli/src/commands/config.rs`

## Goals

1. **Go CLI command parity**: `treb gen deploy`, `treb generate deploy`, `treb completion`, `treb config` (no subcommand), and `treb ls` all work identically to Go.
2. **Backward compatibility**: `treb gen-deploy` and `treb completions` continue to work (hidden aliases) so existing Rust CLI users are not broken.
3. **Help text accuracy**: Help output reflects the new command structure with `gen deploy` in Main Commands and `completion` in Additional Commands.
4. **Golden test coverage**: All golden snapshots updated to reflect renamed/restructured commands.
5. **Integration test coverage**: Explicit tests verify every alias and backward-compatible form.

## User Stories

---

### P4-US-001: Restructure gen-deploy as gen deploy Nested Subcommand

**Description:** Replace the flat `gen-deploy` command with a `gen` parent command containing a `deploy` subcommand. Add `generate` as a visible alias on the `gen` parent. Add a hidden `gen-deploy` alias at the top level for backward compatibility.

**Why:** The Go CLI uses `treb gen deploy <artifact>` and `treb generate deploy <artifact>`. The Rust CLI currently uses `treb gen-deploy <artifact>`, which breaks scripts targeting the Go CLI.

**Acceptance Criteria:**
1. `treb gen deploy Counter` produces the same output as `treb gen-deploy Counter` does today
2. `treb generate deploy Counter` works identically (visible `generate` alias on `gen` parent)
3. `treb gen-deploy Counter` still works (hidden backward-compat alias)
4. `treb gen deploy Counter --json` produces valid JSON output
5. `treb gen deploy Counter --strategy create2` works with all existing flags
6. `Commands::json_flag()` in `main.rs` returns `true` for `Gen { Deploy { json: true } }`
7. Help grouping shows `gen` (not `gen-deploy`) under Main Commands
8. `cargo check -p treb-cli` passes with no errors

**Files to modify:**
- `crates/treb-cli/src/main.rs` — Replace `GenDeploy` enum variant with `Gen` parent containing `GenSubcommand::Deploy`; add `#[command(visible_alias = "generate")]` on `Gen`; add hidden `gen-deploy` alias; update `json_flag()`, help grouping (`write_group`), and dispatch (`run_command`)
- `crates/treb-cli/src/commands/mod.rs` — Add `gen` module if needed for subcommand enum, or keep dispatch in `gen_deploy.rs`
- `crates/treb-cli/src/commands/gen_deploy.rs` — No functional changes; the `run()` function signature stays the same, called from the new dispatch path

**Notes:**
- The `gen-deploy` backward compat can be achieved via a hidden top-level alias or a hidden command variant. Clap supports `#[command(alias = "gen-deploy")]` on the `Gen` parent which won't work for the nested form, so consider a separate hidden `GenDeploy` variant that delegates to the same `gen_deploy::run()`.
- From Phase 1 learnings: when restructuring clap commands, update both the command enum definition and all match arms in `main.rs` (JSON-mode detection, non-interactive detection, etc.) — partial updates cause compile failures.

---

### P4-US-002: Update gen-deploy Golden Tests and Test File for New Syntax

**Description:** Update all gen-deploy golden tests and the `cli_gen_deploy.rs` test file to use the new `gen deploy` command syntax.

**Why:** After US-001 restructures the command, golden tests that invoke `gen-deploy` in their `commands.golden` files need to use `gen deploy` (space-separated) to match the primary command form. The test file also needs its `args` arrays updated.

**Acceptance Criteria:**
1. All `commands.golden` files under `crates/treb-cli/tests/golden/gen_deploy_*` use `gen deploy` syntax
2. All test invocations in `crates/treb-cli/tests/cli_gen_deploy.rs` use `["gen", "deploy", ...]` args
3. `cargo test -p treb-cli` passes — all gen-deploy golden tests pass
4. Golden `.expected` files are updated (via `UPDATE_GOLDEN=1`) to reflect any help text changes in error output

**Files to modify:**
- `crates/treb-cli/tests/golden/gen_deploy_*/commands.golden` — Update command invocations from `gen-deploy` to `gen deploy` (~25 files)
- `crates/treb-cli/tests/cli_gen_deploy.rs` — Update all `.args(["gen-deploy", ...])` to `.args(["gen", "deploy", ...])`

**Notes:**
- Error message golden files (e.g., `gen_deploy_error_invalid_strategy`) may change format since the error now comes from a subcommand context. Regenerate with `UPDATE_GOLDEN=1` and review diffs.
- The `gen_deploy_json` golden test is particularly important — verify JSON output structure is unchanged.

---

### P4-US-003: Rename completions Command to completion

**Description:** Rename the `completions` command to `completion` (singular, matching Go CLI's Cobra built-in). Add `completions` as a hidden alias for backward compatibility.

**Why:** The Go CLI uses `treb completion <shell>` (Cobra's built-in). Scripts and shell setup instructions from Go documentation use the singular form.

**Acceptance Criteria:**
1. `treb completion bash` generates valid bash completions
2. `treb completion zsh`, `treb completion fish`, `treb completion elvish` all work
3. `treb completions bash` still works (hidden backward-compat alias)
4. Help text shows `completion` (not `completions`) under Additional Commands
5. All tests in `cli_completions.rs` pass with updated command name
6. `cargo check -p treb-cli` passes

**Files to modify:**
- `crates/treb-cli/src/main.rs` — Rename `Completions` variant to `Completion`; add `#[command(name = "completion", alias = "completions")]`; update help grouping and dispatch
- `crates/treb-cli/tests/cli_completions.rs` — Update all `.args(["completions", ...])` to `.args(["completion", ...])`
- `crates/treb-cli/build.rs` — Verify completion generation still works (the build-time generation uses the clap `Command` tree, so the rename may affect the generated completions for the command itself)

---

### P4-US-004: Make config Default to show When No Subcommand Given

**Description:** When `treb config` is invoked without a subcommand, default to the `show` behavior (display resolved configuration). Currently, omitting the subcommand produces a clap error.

**Why:** The Go CLI's `config` command has a `RunE` handler that displays config when no subcommand is given. The Rust CLI requires the explicit `config show` subcommand.

**Acceptance Criteria:**
1. `treb config` (no args) produces the same output as `treb config show`
2. `treb config --json` produces the same output as `treb config show --json`
3. `treb config show` still works explicitly
4. `treb config set key value` still works
5. `treb config remove key` still works
6. `Commands::json_flag()` returns `true` for bare `config --json`
7. `cargo check -p treb-cli` passes
8. Existing config golden tests still pass

**Files to modify:**
- `crates/treb-cli/src/main.rs` — Make `ConfigSubcommand` optional in the `Config` variant (e.g., `Option<ConfigSubcommand>`), defaulting to `Show` behavior in dispatch; or use clap's `#[command(subcommand_required = false)]` with a default
- `crates/treb-cli/src/commands/config.rs` — May need to adjust `ConfigSubcommand` enum or add default handling

**Notes:**
- Phase 3 already aligned `config show` sender rendering to Go format. No rendering changes needed — only the dispatch path changes.
- The `--json` flag lives on the `Show` subcommand. When defaulting to show, the `--json` flag needs to be accessible at the `config` level too. Consider adding `--json` to the `Config` parent and forwarding it to `Show`.

---

### P4-US-005: Update Help Text Golden Tests for All Command Changes

**Description:** Update all golden test files that capture help output to reflect the renamed and restructured commands from US-001 through US-004.

**Why:** Help text golden snapshots include command names and groupings. After restructuring `gen-deploy` → `gen`, renaming `completions` → `completion`, and changing config behavior, help text snapshots need updating.

**Acceptance Criteria:**
1. Help text shows `gen` (with `generate` alias noted) under Main Commands
2. Help text shows `completion` under Additional Commands
3. `config` help text reflects optional subcommand / default-to-show behavior
4. All golden tests pass: `cargo test -p treb-cli`
5. No unintended changes to non-help golden snapshots

**Files to modify:**
- `crates/treb-cli/tests/golden/*/commands.golden` — Any test that captures top-level help or command-specific help
- `crates/treb-cli/tests/golden/*/*.expected` — Regenerate affected expected files with `UPDATE_GOLDEN=1`

**Notes:**
- Run `UPDATE_GOLDEN=1 cargo test -p treb-cli` to regenerate, then `git diff` to review only the expected changes.
- Pay attention to Phase 3 golden files (`config_show_default`, `config_set_show_round_trip`, `config_remove_show_round_trip`, `config_show_resolves_dotenv_sender_address`) — these should only change in help text, not in config output content.

---

### P4-US-006: Add Integration Tests for Aliases and Backward Compatibility

**Description:** Add explicit integration tests that verify every alias form and backward-compatible invocation works correctly.

**Why:** Aliases and backward-compat hidden commands are easy to accidentally break in future refactors. Explicit tests lock in the expected behavior.

**Acceptance Criteria:**
1. Test: `treb gen deploy Counter --json` succeeds and produces valid JSON
2. Test: `treb generate deploy Counter --json` succeeds and produces identical output to `gen deploy`
3. Test: `treb gen-deploy Counter --json` succeeds and produces identical output (backward compat)
4. Test: `treb completion bash` exits 0 and produces shell completion output
5. Test: `treb completions bash` exits 0 (backward compat)
6. Test: `treb config` (no subcommand) exits 0 and produces config output
7. Test: `treb ls` exits 0 (already exists in `list_ls_alias` golden test — verify it still passes)
8. All tests pass: `cargo test -p treb-cli`

**Files to create/modify:**
- `crates/treb-cli/tests/cli_command_aliases.rs` — New test file for alias and backward-compat tests
- Or add tests to existing test files (`cli_gen_deploy.rs`, `cli_completions.rs`, `cli_config.rs`)

**Notes:**
- Use direct `assert_cmd` assertions rather than golden snapshots for alias tests (per Phase 1 learnings about removed/renamed commands).
- For `gen-deploy` backward compat, assert that the JSON output structure matches the `gen deploy` output exactly (same `contractName`, `strategy`, `code` fields).
- For `config` default, use the `project` fixture with `TrebRunner` to get a valid config context.

---

## Functional Requirements

- **FR-1:** `treb gen deploy <artifact> [flags]` executes deployment script generation as a nested subcommand of `gen`.
- **FR-2:** `treb generate deploy <artifact> [flags]` works identically via visible alias on the `gen` parent.
- **FR-3:** `treb gen-deploy <artifact> [flags]` continues to work via a hidden backward-compatible alias or command variant.
- **FR-4:** `treb completion <shell>` generates shell completions (renamed from `completions`).
- **FR-5:** `treb completions <shell>` continues to work via hidden backward-compatible alias.
- **FR-6:** `treb config` (no subcommand) displays resolved configuration, identical to `treb config show`.
- **FR-7:** `treb config --json` (no subcommand) outputs JSON configuration, identical to `treb config show --json`.
- **FR-8:** `treb config show`, `treb config set`, and `treb config remove` continue to work unchanged.
- **FR-9:** `treb ls` continues to work as alias for `treb list` (already implemented — verify no regression).
- **FR-10:** Help text reflects new command names: `gen` under Main Commands, `completion` under Additional Commands.
- **FR-11:** `--json` flag detection (`Commands::json_flag()`) works correctly for both `gen deploy --json` and `config --json`.

## Non-Goals

- **No new commands**: This phase only renames, restructures, and adds aliases to existing commands. No new functionality is added.
- **No output format changes**: Command output (human and JSON) remains identical. Only command invocation syntax changes.
- **No flag changes**: Command flags (`--strategy`, `--proxy`, `--json`, etc.) are unchanged. Short flag additions (like `-s`, `-n`) are deferred to Phase 5.
- **No gen subcommand expansion**: The `gen` parent only has `deploy` as a subcommand. Future `gen` subcommands (if any) are out of scope.
- **No deprecation warnings**: Hidden backward-compat aliases work silently without deprecation messages. Deprecation can be added in a future phase if desired.
- **No build.rs changes for completion output**: The shell completion file content generation stays the same. Only the command name in the CLI tree changes.

## Technical Considerations

### Clap Nested Subcommand Pattern

The `gen deploy` restructuring requires converting from a flat enum variant to a parent-with-subcommand pattern. The idiomatic clap approach:

```rust
#[command(visible_alias = "generate")]
Gen {
    #[command(subcommand)]
    command: GenSubcommand,
},
```

For `gen-deploy` backward compatibility, consider a separate hidden variant:
```rust
#[command(name = "gen-deploy", hide = true)]
GenDeploy { /* same fields as Gen::Deploy */ },
```

Both variants dispatch to the same `gen_deploy::run()` function.

### Config Default Subcommand

Clap does not have a built-in "default subcommand" feature. Options:
1. Make the subcommand `Option<ConfigSubcommand>` and handle `None` as `Show` in dispatch
2. Use `#[command(subcommand_required = false)]` and process args manually

Option 1 is cleaner. The `--json` flag needs special handling since it currently lives on `ConfigSubcommand::Show`. When subcommand is `None`, `--json` must still be accessible — consider hoisting it to the `Config` parent.

### Match Arm Updates

Every match arm in `main.rs` that references `Commands::GenDeploy` or `Commands::Completions` must be updated. Key locations:
- `run_command()` dispatch
- `Commands::json_flag()`
- Help text grouping (`write_group` calls)
- Any non-interactive detection

### Golden Test Update Strategy

Run `UPDATE_GOLDEN=1 cargo test -p treb-cli` after all code changes, then review diffs to ensure only expected changes (command names in help text, error messages) are present. No output content should change.

### Dependencies

- **None from previous phases.** This phase is independent.
- Phase 3's `config show` sender rendering changes are already in place — the default-to-show behavior inherits them automatically.
- The `list` / `ls` alias is already implemented and has golden test coverage (`list_ls_alias`). No changes needed, only regression verification.
