# PRD: Phase 11 - Error Messages, Version Format, and Help Text

## Introduction

Phase 11 polishes the remaining surface-level differences between the Rust `treb-cli` and the Go `treb` CLI to complete drop-in parity. While previous phases aligned command structure, flags, registry format, and subcommand behavior, this phase targets the three remaining visible gaps: version string format, error message style, and help text consistency. These differences are the first thing users encounter when switching from Go to Rust, so fixing them is essential for a seamless transition.

This phase has no dependencies on other phases and can be implemented independently.

## Goals

1. **Version string parity**: `treb version` and `treb --version` output a `git describe`-style version string (e.g., `treb nightly-41-gc72d1b1`) instead of the static Cargo package version (`treb 0.1.0`).
2. **Error message alignment**: Unknown command and unknown flag errors match the Go CLI's format (e.g., `Error: unknown command "foo" for "treb"` instead of clap's `error: unrecognized subcommand 'foo'`).
3. **Help footer consistency**: All help output (root and subcommand) includes the Go-style footer `Use "treb [command] --help" for more information about a command.` where appropriate.
4. **Document `-h` vs `--help` behavior**: Explicitly decide and document whether the clap convention (summary on `-h`, full on `--help`) is kept as an intentional improvement over Go's Cobra convention (full help on both `-h` and `--help`).

## User Stories

### P11-US-001: Git-Describe Version String in build.rs

**Description:** Update `build.rs` to embed a `git describe --tags --always --dirty` version string (matching Go's Makefile `VERSION` variable) so that both `treb --version` and `treb version` display the same format as the Go CLI.

**Changes:**
- `crates/treb-cli/build.rs` — add a `git describe --tags --always --dirty` command to produce a `TREB_VERSION` env var at compile time; fall back to `CARGO_PKG_VERSION` if `git describe` fails (e.g., shallow clone, no tags)
- `crates/treb-cli/src/main.rs` — change the clap `version` attribute from `version` (which uses `CARGO_PKG_VERSION`) to `version = env!("TREB_VERSION")` so `treb --version` prints the git-describe string
- `crates/treb-cli/src/commands/version.rs` — change `info.version` from `env!("CARGO_PKG_VERSION")` to `env!("TREB_VERSION")` so `treb version` (both human and JSON) prints the git-describe string

**Acceptance Criteria:**
- `treb --version` outputs `treb <git-describe-string>` (e.g., `treb v0.1.0-3-gabcdef1` or `treb nightly-41-gc72d1b1` depending on tags)
- `treb version` human output first line is `treb <git-describe-string>`
- `treb version --json` includes `"version": "<git-describe-string>"` field
- When no git tags exist, falls back to `CARGO_PKG_VERSION` so the build never fails
- `cargo check -p treb-cli` passes (typecheck)
- `build.rs` emits `cargo:rerun-if-changed=../../.git/HEAD` and `cargo:rerun-if-changed=../../.git/refs/` to rebuild when the version changes

### P11-US-002: Custom Clap Error Formatter for Go-Style Error Messages

**Description:** Intercept clap parsing errors in `main()` and reformat them to match Go's Cobra error style before printing. The Go CLI outputs `Error: unknown command "foo" for "treb"` while clap outputs `error: unrecognized subcommand 'foo'`.

**Changes:**
- `crates/treb-cli/src/main.rs` — replace the `err.print()` call in the parse error branch with a custom `format_clap_error()` function that rewrites known clap error kinds into Go-compatible format

**Error mapping (clap → Go-style):**
- `ErrorKind::InvalidSubcommand` / `ErrorKind::UnrecognizedSubcommand` → `Error: unknown command "<cmd>" for "treb"\nRun 'treb --help' for usage.`
- `ErrorKind::UnknownArgument` → `Error: unknown flag: <flag>\nRun 'treb <command> --help' for usage.`
- All other error kinds → pass through to `err.print()` unchanged (preserving clap's formatting for typo suggestions, value validation, missing args, etc.)

**Acceptance Criteria:**
- `treb nonexistent` prints `Error: unknown command "nonexistent" for "treb"` to stderr followed by `Run 'treb --help' for usage.`
- `treb list --nonexistent-flag` prints `Error: unknown flag: --nonexistent-flag` to stderr followed by `Run 'treb list --help' for usage.`
- Clap typo suggestions (e.g., `Did you mean 'list'?`) are preserved for other error kinds
- `--json` error output still produces `{"error":"..."}` with the reformatted message
- Exit code remains non-zero for all error cases
- `cargo check -p treb-cli` passes (typecheck)
- Unit tests cover the mapping for `InvalidSubcommand`, `UnknownArgument`, and passthrough cases

### P11-US-003: Help Footer and -h vs --help Behavior Audit

**Description:** Verify and align help text footers across root and subcommand help, and document the `-h` vs `--help` behavior decision.

**Changes:**
- `crates/treb-cli/src/main.rs` — verify `build_grouped_command()` footer text; add `after_help` or `after_long_help` to subcommands if their help output is missing the `Use "treb [command] --help"` guidance footer (clap subcommands may not inherit the root template)
- Add a comment in `main.rs` near the clap configuration documenting the intentional `-h` (summary) vs `--help` (full) divergence from Go/Cobra as a Rust CLI convention worth keeping

**Acceptance Criteria:**
- Root `treb --help` includes `Use "treb [command] --help" for more information about a command.` footer (already present — verify no regression)
- Subcommand `treb <command> --help` includes a contextual footer when clap does not already provide one (e.g., `Use "treb <command> <subcommand> --help"` for commands with subcommands like `gen`, `fork`, `config`, `addressbook`)
- A code comment in `main.rs` documents that `-h` shows summary help and `--help` shows full help, as an intentional Rust CLI convention diverging from Go/Cobra's identical behavior on both
- `cargo check -p treb-cli` passes (typecheck)

### P11-US-004: Refresh All Help Golden Snapshots and Add Error Format Tests

**Description:** Update all golden snapshots that are affected by the version string change and help footer changes. Add integration tests for the new error message format.

**Changes:**
- `crates/treb-cli/tests/integration_help.rs` — refresh all `help_*` goldens (root, gen, completion, config, list, addressbook, show, sync, register, verify, fork_enter) after US-001/US-003 changes
- `crates/treb-cli/tests/` — add focused integration tests for error message formatting: unknown command, unknown flag, and passthrough cases; use direct `assert_cmd` assertions (not goldens) since error output contains dynamic content

**Acceptance Criteria:**
- All `help_*` golden tests pass with `UPDATE_GOLDEN=1` and reflect the updated footer/version
- New error format tests verify:
  - `treb nonexistent` → stderr contains `Error: unknown command "nonexistent" for "treb"`
  - `treb list --fake-flag` → stderr contains `Error: unknown flag: --fake-flag`
  - Exit codes are non-zero in all error cases
- `cargo test -p treb-cli` passes (full CLI test suite)
- `cargo clippy --workspace --all-targets` passes (lint)
- Golden tests that early-return (gated fixtures) have their `commands.golden` headers manually verified if argv changed

## Functional Requirements

- **FR-1**: The version string embedded at compile time must come from `git describe --tags --always --dirty`, falling back to `CARGO_PKG_VERSION` when git metadata is unavailable.
- **FR-2**: Both `treb --version` and `treb version` must display the same version string.
- **FR-3**: Unknown command errors must be formatted as `Error: unknown command "<cmd>" for "treb"` followed by `Run 'treb --help' for usage.` on stderr.
- **FR-4**: Unknown flag errors must be formatted as `Error: unknown flag: <flag>` followed by `Run 'treb <command> --help' for usage.` on stderr.
- **FR-5**: Clap error kinds not explicitly remapped (typo suggestions, missing required args, value validation) must pass through unchanged.
- **FR-6**: JSON error output (`--json`) must use the reformatted error message.
- **FR-7**: The root help footer `Use "treb [command] --help" for more information about a command.` must be present.
- **FR-8**: Commands with subcommands (`gen`, `fork`, `config`, `addressbook`, `migrate`) should include a contextual help footer guiding users to subcommand help.

## Non-Goals

- **Not changing `-h` to show full help**: The summary-on-`-h` / full-on-`--help` split is a clap convention that many Rust CLI users expect. Documenting this as intentional is sufficient.
- **Not matching every Cobra error message verbatim**: Only `unknown command` and `unknown flag` errors are remapped. Clap's superior typo suggestions (`Did you mean...?`) are kept as-is.
- **Not changing `treb version --json` field names**: The JSON output schema stays the same; only the `version` field value changes to git-describe format.
- **Not adding a Rust-side Makefile or build wrapper**: The `git describe` call happens in `build.rs`, not an external build script.
- **Not removing clap's default error formatting infrastructure**: The custom formatter intercepts specific error kinds and passes everything else through.

## Technical Considerations

### Version String Construction
- Go uses `git describe --tags --always --dirty` via Makefile, injected through `-ldflags`. Rust must replicate this in `build.rs` using `std::process::Command`.
- The `--always` flag ensures output even without tags (falls back to abbreviated commit hash). The `--dirty` flag appends `-dirty` for uncommitted changes.
- `build.rs` must trigger rebuilds on `.git/HEAD` and `.git/refs/` changes so the version string updates without full recompilation.

### Clap Error Interception
- Clap's `Error` type exposes `.kind()` for error classification and `.to_string()` for the formatted message. The custom formatter inspects `.kind()` and either reformats or passes through.
- The error rendering path in `main()` already branches on `--json` for JSON error output — the reformatter integrates before that branch point.
- `clap::error::ErrorKind::InvalidSubcommand` and `clap::error::ErrorKind::UnrecognizedSubcommand` may both fire depending on clap version and subcommand configuration — handle both.

### Golden Snapshot Impact
- Version string changes propagate to any golden that captures `treb --version` output or includes version in headers.
- Help footer changes affect all 11 existing `help_*` goldens in `integration_help.rs`.
- Gated goldens that early-return in restricted environments cannot be refreshed with `UPDATE_GOLDEN=1` alone — their `commands.golden` headers may need manual patching per CLAUDE.md guidance.

### Files Modified
| File | Change |
|------|--------|
| `crates/treb-cli/build.rs` | Add `git describe` version embedding, git-aware rerun triggers |
| `crates/treb-cli/src/main.rs` | Custom `version` attribute, `format_clap_error()` function, help footer/comment |
| `crates/treb-cli/src/commands/version.rs` | Use `TREB_VERSION` env var |
| `crates/treb-cli/tests/integration_help.rs` | Refresh all help goldens |
| `crates/treb-cli/tests/golden/help_*/` | Updated snapshot files |
| `crates/treb-cli/tests/integration_*.rs` or new test file | Error format integration tests |
