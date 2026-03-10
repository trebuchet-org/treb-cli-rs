# PRD: Phase 5 - Global Non-Interactive Flag and Short Flags

## Introduction

Phase 5 aligns the Rust CLI's non-interactive flag handling and short flag conventions with the Go CLI to ensure drop-in compatibility. Currently, `--non-interactive` is defined per-command on only `run` and `compose`, while the Go CLI defines it as a global persistent flag inherited by every command. Additionally, the Go CLI provides `-s`/`-n` short flags for `--namespace`/`--network` on `list` and `tag` commands, which the Rust CLI lacks.

This phase promotes `--non-interactive` to a global flag on the `Cli` struct, removes the per-command duplicates from `run`/`compose`, updates all command handlers to read the global flag, adds the missing `-s`/`-n` short flags, and also fixes the `TREB_NON_INTERACTIVE` env var to accept both `"true"` and `"1"` (the master plan specifies `=1`, Go uses `"true"`, and the Rust doc comments reference `=1` but code only checks `"true"`).

## Goals

1. **Global `--non-interactive` flag**: Every command accepts `--non-interactive` via a single global definition on `Cli`, matching Go's `rootCmd.PersistentFlags().Bool("non-interactive", ...)` pattern.
2. **Consistent non-interactive detection**: All commands that use `is_non_interactive(false)` today (prune, reset, verify, show, tag via selector) receive the global flag value instead of hardcoded `false`.
3. **Short flag parity**: `list` and `tag` commands accept `-s` for `--namespace` and `-n` for `--network`, matching Go CLI.
4. **Env var compatibility**: `TREB_NON_INTERACTIVE` accepts both `"true"` and `"1"` (case-insensitive) to align with the documented convention and common CI usage.
5. **No short flag conflicts**: Verify and document that `-s`/`-n` additions don't clash with existing short flags on any command.

## User Stories

### P5-US-001: Add Global `--non-interactive` Flag to Cli Struct

**Description:** Move `--non-interactive` from per-command definition on `Run` and `Compose` to a global flag on the `Cli` struct, alongside the existing `--no-color` global flag.

**Changes:**
- `crates/treb-cli/src/main.rs`: Add `#[arg(long, global = true)] non_interactive: bool` to the `Cli` struct (next to the existing `no_color` field)
- Remove `non_interactive: bool` field from the `Run` variant in `Commands` enum (lines 103-106)
- Remove `non_interactive: bool` field from the `Compose` variant in `Commands` enum (lines 401-404)
- Update the doc comment on the new global flag: `"Skip interactive prompts (also enabled via TREB_NON_INTERACTIVE=1/true, CI=true, or non-TTY stdin/stdout)"`

**Acceptance Criteria:**
- `treb --non-interactive run script/Deploy.s.sol` is accepted by the parser
- `treb run --non-interactive script/Deploy.s.sol` is accepted by the parser (clap global flags work in both positions)
- `treb --non-interactive list` is accepted (previously would fail)
- `treb --non-interactive show` is accepted
- `treb --non-interactive tag` is accepted
- The `Run` and `Compose` variants no longer define their own `non_interactive` field
- `cargo check -p treb-cli` passes (will require US-002 changes to compile)

---

### P5-US-002: Wire Global Non-Interactive Flag Through All Command Handlers

**Description:** Update the `run()` match arms in `main.rs` to pass `cli.non_interactive` to command handlers. Update commands that currently call `is_non_interactive(false)` to receive and use the global flag value.

**Changes:**
- `crates/treb-cli/src/main.rs`: In the `run()` function's match on `cli.command`:
  - `Commands::Run { .. }` arm: pass `cli.non_interactive` instead of the removed per-command field to `commands::run::run()`
  - `Commands::Compose { .. }` arm: pass `cli.non_interactive` to `commands::compose::run()`
  - `Commands::Show { .. }` arm: pass `cli.non_interactive` to `commands::show::run()`
  - `Commands::Tag { .. }` arm: pass `cli.non_interactive` to `commands::tag::run()`
  - `Commands::Verify { .. }` arm: pass `cli.non_interactive` to `commands::verify::run()`
  - `Commands::Prune { .. }` arm: pass `cli.non_interactive` to `commands::prune::run()`
  - `Commands::Reset { .. }` arm: pass `cli.non_interactive` to `commands::reset::run()`
- `crates/treb-cli/src/commands/show.rs`: Add `non_interactive: bool` parameter, pass to `is_non_interactive()` calls (currently called via `fuzzy_select_deployment_id` which calls `is_non_interactive(false)`)
- `crates/treb-cli/src/commands/tag.rs`: Add `non_interactive: bool` parameter, pass to `is_non_interactive()` calls (same selector pattern)
- `crates/treb-cli/src/commands/verify.rs`: Change `is_non_interactive(false)` call at line 570 to `is_non_interactive(non_interactive)`, add parameter
- `crates/treb-cli/src/commands/prune.rs`: Change `is_non_interactive(false)` at line 429 to `is_non_interactive(non_interactive)`, add parameter
- `crates/treb-cli/src/commands/reset.rs`: Change `is_non_interactive(false)` at line 240 to `is_non_interactive(non_interactive)`, add parameter
- `crates/treb-cli/src/commands/run.rs`: No signature change needed (already accepts `non_interactive: bool`), just wire from new source
- `crates/treb-cli/src/commands/compose.rs`: No signature change needed, just wire from new source

**Note on show/tag:** These commands use `fuzzy_select_deployment_id()` from `ui/selector.rs`, which internally calls `is_non_interactive(false)`. The simplest approach is to either: (a) pass the flag to `show::run()`/`tag::run()` and have them call `is_non_interactive(non_interactive)` before invoking the selector, or (b) thread the flag through the selector functions. Choose whichever is simpler — option (a) is likely cleaner since the selector already returns an error in non-interactive mode when no query is given.

**Acceptance Criteria:**
- `cargo check -p treb-cli` passes
- `cargo test -p treb-cli` passes (existing tests still work)
- `treb --non-interactive prune` skips confirmation prompt (previously only worked via env var or non-TTY)
- `treb --non-interactive reset` skips confirmation prompt
- `treb --non-interactive verify` selects all candidates without prompting
- No command handler calls `is_non_interactive(false)` anymore — all pass the global flag value

---

### P5-US-003: Accept `TREB_NON_INTERACTIVE=1` in Addition to `"true"`

**Description:** The master plan specifies `TREB_NON_INTERACTIVE=1` as a detection criterion, and the Rust doc comments reference `=1`, but the implementation only checks for `"true"`. Go's implementation also only checks `"true"`. Update the Rust code to accept both `"1"` and `"true"` for maximum compatibility.

**Changes:**
- `crates/treb-cli/src/ui/interactive.rs`: Update `env_requests_non_interactive()` to accept `"1"` in addition to `"true"` (case-insensitive) for `TREB_NON_INTERACTIVE`
- Keep `CI` env var checking `"true"` only (matches convention — CI systems set `CI=true`)
- Update module doc comment to reflect both accepted values

**Acceptance Criteria:**
- `TREB_NON_INTERACTIVE=1` triggers non-interactive mode
- `TREB_NON_INTERACTIVE=true` still triggers non-interactive mode (backward compat)
- `TREB_NON_INTERACTIVE=TRUE` still triggers non-interactive mode (case-insensitive)
- `TREB_NON_INTERACTIVE=0` does NOT trigger non-interactive mode
- `TREB_NON_INTERACTIVE=false` does NOT trigger non-interactive mode
- Unit tests in `interactive.rs` updated to cover `"1"` value
- `cargo test -p treb-cli` passes

---

### P5-US-004: Add `-s` and `-n` Short Flags to `list` and `tag` Commands

**Description:** The Go CLI defines `-s` as the short flag for `--namespace` and `-n` for `--network` on `list` and `tag` commands. Add these short flags to the Rust CLI for parity.

**Pre-check:** Verify no existing short flag conflicts:
- `-s` is not used on any command currently (verified: no `short = 's'` in any command file)
- `-n` is not used on `list` or `tag` currently (verified: no `short = 'n'` in any command file)
- `-s` IS used on `verify` for `--sourcify` in Go (Phase 8 scope) — but since Rust's `verify` command has no short flags today, there's no conflict. The `-s` short flag can safely be added to `list` and `tag` now; Phase 8 will handle `verify` separately.

**Changes:**
- `crates/treb-cli/src/main.rs`:
  - `List` variant: Change `#[arg(long)]` to `#[arg(long, short = 'n')]` on `network` field
  - `List` variant: Change `#[arg(long)]` to `#[arg(long, short = 's')]` on `namespace` field
  - `Tag` variant: Add `network: Option<String>` with `#[arg(long, short = 'n')]` and `namespace: Option<String>` with `#[arg(long, short = 's')]` — **Note:** If `tag` doesn't have `network`/`namespace` fields yet (it currently doesn't), this story adds them to the clap definition but they won't be wired to filtering logic (that's Phase 6). The fields are accepted by the parser but may be unused pending Phase 6.

**Important consideration for `tag`:** The Go CLI's `tag` command accepts `--network`/`-n` and `--namespace`/`-s` flags (see `tag.go` lines 40-41). The Rust `Tag` variant currently has no `network` or `namespace` fields at all. This story adds the clap definitions so the flags are accepted by the parser (matching Go), but the tag command's runtime behavior won't use them for filtering until Phase 6. Accepting-but-ignoring is preferable to rejecting flags that Go scripts pass.

**Acceptance Criteria:**
- `treb list -n mainnet` works (equivalent to `treb list --network mainnet`)
- `treb list -s production` works (equivalent to `treb list --namespace production`)
- `treb ls -n mainnet -s production` works (with alias)
- `treb tag Counter -n mainnet` is accepted by parser (no "unknown flag" error)
- `treb tag Counter -s production` is accepted by parser
- `treb list --help` shows `-n` next to `--network` and `-s` next to `--namespace`
- `treb tag --help` shows `-n` next to `--network` and `-s` next to `--namespace`
- No short flag conflicts with other commands
- `cargo check -p treb-cli` passes
- `cargo test -p treb-cli` passes

---

### P5-US-005: Update Golden Tests and Add Integration Tests

**Description:** Update help text golden files to reflect the new global `--non-interactive` flag and new short flags. Add integration tests verifying the global flag works across commands.

**Changes:**
- Update golden help snapshots:
  - Root `treb --help` output (built by `build_grouped_help()` in `main.rs`) — now shows `--non-interactive` as a global option
  - `treb run --help` — no longer shows per-command `--non-interactive` (it's global now)
  - `treb compose --help` — same
  - `treb list --help` — shows `-n`/`-s` short flags
  - `treb tag --help` — shows `-n`/`-s` short flags and new `--network`/`--namespace` fields
  - Any other help golden files affected by the global flag addition
- Update `crates/treb-cli/build.rs` if shell completions need adjustments for the global flag
- `crates/treb-cli/tests/integration_non_interactive.rs`: Add test verifying `--non-interactive` works on a command that previously didn't accept it (e.g., `treb --non-interactive list --json` completes without error)
- `crates/treb-cli/tests/integration_non_interactive.rs`: Add test verifying `TREB_NON_INTERACTIVE=1` works (not just `"true"`)
- `crates/treb-cli/tests/cli_compatibility_aliases.rs`: Add test confirming `-n` and `-s` short flags produce identical output to their long forms on `list`
- Run with `UPDATE_GOLDEN=1 cargo test -p treb-cli` to regenerate affected golden files, then review diffs

**Acceptance Criteria:**
- All golden files updated to reflect new global `--non-interactive` and short flags
- `cargo test -p treb-cli` passes with all golden tests
- `cargo test --workspace --all-targets` passes
- `cargo clippy --workspace --all-targets` passes
- New integration test: `--non-interactive` accepted on `list` command
- New integration test: `TREB_NON_INTERACTIVE=1` triggers non-interactive mode
- New compatibility test: `-n`/`-s` short flags produce same output as long forms on `list`

## Functional Requirements

- **FR-1:** `--non-interactive` is a global flag defined once on the `Cli` struct with `global = true`, inherited by all subcommands.
- **FR-2:** The global `--non-interactive` flag value is threaded to every command handler that uses `is_non_interactive()`.
- **FR-3:** No command handler calls `is_non_interactive(false)` — all pass the actual global flag value.
- **FR-4:** `TREB_NON_INTERACTIVE` env var accepts both `"1"` and `"true"` (case-insensitive).
- **FR-5:** `list` command accepts `-n` for `--network` and `-s` for `--namespace`.
- **FR-6:** `tag` command accepts `-n` for `--network` and `-s` for `--namespace` (parser-level; runtime filtering deferred to Phase 6).
- **FR-7:** All existing non-interactive detection criteria remain functional: env var, `CI=true`, non-TTY stdin, non-TTY stdout.
- **FR-8:** Shell completions (`build.rs`) are updated if the global flag or new short flags require it.
- **FR-9:** Help text for all affected commands reflects the changes.

## Non-Goals

- **Adding `--namespace`/`--network` filtering logic to `show` or `tag` commands** — that is Phase 6 scope. This phase only ensures the flags are accepted by the parser where Go accepts them.
- **Adding short flags to `verify` command** (`-e`, `-b`, `-s`, `-n`) — that is Phase 8 scope.
- **Changing the behavior of `is_non_interactive()` beyond accepting `"1"** — the detection criteria (env vars, TTY checks) are already correct.
- **Adding `--non-interactive` to commands that don't exist yet** (e.g., `addressbook` from Phase 9) — the global flag will automatically be inherited when those commands are added.

## Technical Considerations

1. **Clap `global = true` behavior:** A `global = true` arg on the top-level `Cli` struct is inherited by all subcommands. It can appear before or after the subcommand name on the command line. This matches Go's `PersistentFlags()` pattern.

2. **Per-command removal:** Removing `non_interactive` from `Run` and `Compose` enum variants means updating the destructuring patterns in `main.rs::run()` and passing `cli.non_interactive` instead. Since `cli` is consumed by `cli.command`, the flag must be extracted before the match (e.g., `let non_interactive = cli.non_interactive;`).

3. **Selector threading:** `show` and `tag` use `fuzzy_select_deployment_id()` from `ui/selector.rs`, which calls `is_non_interactive(false)` internally. Two options:
   - Thread `non_interactive` through the selector function signature
   - Check `is_non_interactive(non_interactive)` in the command before calling the selector and pass a pre-resolved value

   Either works; choose based on code simplicity.

4. **Tag command `network`/`namespace` fields:** Adding these fields to the `Tag` clap variant means they'll be extracted in the match arm but not yet used in `commands::tag::run()`. This is intentional — Phase 6 wires them to filtering. The fields should be passed to the function signature to avoid unused-variable warnings, or prefixed with `_`.

5. **Golden test volume:** The global flag appears in root help output. Every subcommand's `--help` will also show it (clap includes global flags in subcommand help). This may affect many golden files. Use `UPDATE_GOLDEN=1` to regenerate and review carefully.

6. **Build.rs completions:** The `build.rs` file generates shell completions. Adding a global flag to `Cli` should automatically be picked up by clap's completions derive, but verify the generated completions include `--non-interactive` for all commands.
