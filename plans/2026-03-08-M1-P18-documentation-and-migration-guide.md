# PRD: Phase 18 - Documentation and Migration Guide

## Introduction

Phase 18 is the final phase of the treb-cli-rs project, creating user-facing documentation for the Rust CLI and a migration guide for users switching from the Go `treb-cli`. With all 17 prior phases complete — covering the full command set, output formatting, JSON audit, E2E tests, and cross-platform release pipeline — this phase documents the finished product.

The current README.md is a 47-line developer scaffold with no user-facing content. No MIGRATION.md, CHANGELOG.md, or architectural documentation exists. The project root CLAUDE.md needs updates to reflect the final crate structure, testing patterns, and architectural decisions accumulated across 18 phases. CLI help text exists on all commands but needs an audit to ensure descriptions, flag names, and examples match the Go CLI.

## Goals

1. **User-ready README**: A comprehensive README.md that a new user can follow to install, configure, and use treb from scratch — including installation via `trebup`, quick start with `treb init` and `treb run`, and a command reference table covering all 22 commands/subcommands.

2. **Migration guide**: A MIGRATION.md that enables Go CLI users to switch to the Rust CLI with confidence — documenting breaking changes (treb.toml v1 dropped, foundry.toml sender format dropped), behavioral differences (in-process forge, JSON key sorting), and improvements (speed, fork mode, governor/Safe support).

3. **Structured changelog**: A CHANGELOG.md generated from the 18-phase git history, organized by feature area, giving users and contributors a clear record of what was built and when.

4. **Help text parity**: All `--help` output across 22 commands matches Go CLI descriptions in tone and content, with consistent formatting and no stale or missing descriptions.

## User Stories

### P18-US-001: README.md — Feature Overview, Installation, and Command Reference

**Description:** Replace the minimal developer-scaffold README with a comprehensive user-facing document covering feature overview, installation via `trebup` and from source, quick start walkthrough, full command reference table, configuration guide, and links to MIGRATION.md and CHANGELOG.md.

**Acceptance Criteria:**
- README.md includes a one-paragraph project description explaining treb as a deployment orchestration CLI for Foundry with in-process forge integration
- "Installation" section documents `trebup` one-liner (`curl -sL https://raw.githubusercontent.com/trebuchet-org/treb-cli-rs/main/scripts/trebup | bash`), environment variables (`TREB_VERSION`, `TREB_INSTALL_DIR`), and shell completion auto-installation (bash, zsh, fish locations)
- "Building from Source" section documents `git clone --recurse-submodules`, Rust nightly requirement, `cargo build --release`, and that the binary is `treb-cli` (renamed to `treb` in release packaging)
- "Quick Start" section walks through `treb init`, writing a deploy script, `treb run --dry-run`, and `treb run --broadcast` with example output snippets
- "Command Reference" table lists all 22 commands/subcommands with one-line descriptions and key flags
- "Configuration" section documents treb.toml v2 format: `[accounts]` (private_key, keystore types), `[namespace]` (profile, senders), with a minimal example
- "Environment Variables" section documents `NO_COLOR`, `TREB_NON_INTERACTIVE`, `CI`, and their effect on output
- "JSON Output" section documents `--json` flag behavior: alphabetically sorted keys, `treb run --json` may include forge compilation output before JSON (search for first `{`), `--json --broadcast` requires `--non-interactive`
- Links to MIGRATION.md and CHANGELOG.md at the top
- `cargo test --workspace --all-targets` passes (no broken doc links or test references)

**Notes:**
- Reference `scripts/trebup` for accurate installation instructions
- Reference `crates/treb-cli/src/main.rs` for the authoritative command list
- Reference `crates/treb-cli/tests/fixtures/project/treb.toml` for config example
- Keep the README under 300 lines — it should be scannable, not exhaustive
- GitHub org is `trebuchet-org`, repo is `trebuchet-org/treb-cli-rs`

---

### P18-US-002: MIGRATION.md — Go to Rust CLI Migration Guide

**Description:** Create a migration guide for users switching from the Go `treb-cli` to the Rust `treb-cli-rs`. Document breaking changes, dropped features, behavioral differences, and improvements. Organized as a checklist users can follow.

**Acceptance Criteria:**
- "Breaking Changes" section documents:
  - treb.toml v1 format is no longer supported at runtime (use `treb migrate config` to convert)
  - foundry.toml `[profile.*.treb.*]` sender configuration is no longer supported (use `treb migrate config`)
  - Binary name changed from `treb` (Go) to `treb` (Rust) — same name, different install path
- "Dropped Features" section documents any features present in Go but not in Rust (if none, state "full feature parity achieved")
- "Behavioral Differences" section documents:
  - In-process forge execution (no subprocess `forge` calls) — faster, no `forge` binary needed on PATH
  - JSON output keys are alphabetically sorted (Go may have had insertion-order)
  - `treb run --json` stdout may include forge compilation output before the JSON object
  - `--json --broadcast` requires `--non-interactive` flag (explicit safety constraint)
  - Non-interactive mode detection: `--non-interactive` flag, `TREB_NON_INTERACTIVE=true`, `CI=true`, plus stdin TTY detection
  - JSON error format: `{"error":"msg"}` on stderr with exit code 1
  - Deployment ID format: `namespace/chainId/contractName:label`
  - `treb init` does not create `deployments.json` until the first deployment
  - `reset --namespace` scopes deployments only; transactions scoped by `--network` only
  - Fork snapshot only copies files that exist at snapshot time
- "Improvements" section highlights:
  - In-process foundry: no external `forge` binary dependency, faster compilation and execution
  - Governor sender support: create governance proposals via `treb run` with oz_governor account type
  - Safe multisig integration: propose transactions to Safe Transaction Service
  - Shell completion auto-installation via `trebup`
  - `treb version --json` includes foundryVersion, trebSolCommit, rustVersion for reproducibility
  - Cross-platform releases (linux-amd64, linux-arm64, darwin-amd64, darwin-arm64)
- "Registry Compatibility" section confirms Rust reads Go registry files and is forward-compatible (may add new fields)
- "Migration Checklist" with numbered steps: (1) install Rust CLI, (2) run `treb migrate config` if using v1 config, (3) verify with `treb list`, (4) test with `treb run --dry-run`
- `cargo test --workspace --all-targets` passes

**Notes:**
- Reference master plan key decisions for compatibility guarantees
- Reference Phase 11 for migrate command details
- Reference Phase 15 for JSON/non-interactive behavior
- Reference Phase 13/12 for governor/Safe as new features

---

### P18-US-003: CHANGELOG.md and CLAUDE.md Architecture Documentation

**Description:** Create a structured CHANGELOG.md documenting all 18 phases of work organized by feature area. Update the project root CLAUDE.md with architectural decisions, crate responsibilities, testing patterns, and development workflow documentation accumulated across all phases.

**Acceptance Criteria:**
- CHANGELOG.md uses Keep a Changelog format with `## [0.1.0]` as the single release section
- Changelog entries organized by category: Added, Changed, Fixed (not per-phase — group by feature)
- "Added" covers: all 22 commands, in-process forge integration, treb-sol bindings, output formatting framework (tree rendering, color palette, badges), verification system (Etherscan, Sourcify, Blockscout), compose orchestration, fork mode, Safe multisig, Governor sender, dev anvil management, shell completions, cross-platform releases, E2E test suite, JSON output mode for all commands, non-interactive mode
- "Changed" covers: treb.toml v1 → v2 migration path (v1 no longer supported at runtime)
- CLAUDE.md at project root is updated (or created if it only exists at `.ralph/CLAUDE.md`) with:
  - Workspace crate map: 8 crates with one-line responsibility description each
  - Key file paths: main.rs (CLI entry), commands/ (command handlers), ui/ (output formatting), store/ (registry modules)
  - Testing patterns: golden file framework (`UPDATE_GOLDEN=1`), integration test anvil spawning (`AnvilConfig::new().port(0).spawn()`), E2E test helpers (`tests/e2e/mod.rs`), `#[tokio::test(flavor = "multi_thread")]` for async tests
  - Build metadata: `build.rs` sets `TREB_FOUNDRY_VERSION`, `TREB_SOL_COMMIT`, etc.
  - Alloy/foundry version pinning: all alloy 1.x crates pinned to v1.1.1 via `[patch.crates-io]`
  - Color/output conventions: `is_color_enabled()` check, `styled()` helper pattern, `print_json` with `sort_json_keys()`
  - Non-interactive mode: `ui::interactive::is_non_interactive()` centralized check
- `cargo test --workspace --all-targets` passes

**Notes:**
- Derive CHANGELOG entries from git log and phase plan files in `plans/`
- CLAUDE.md should be concise (under 150 lines) — it is loaded into every Claude Code session
- Do NOT duplicate content already in the auto-memory MEMORY.md — CLAUDE.md should document stable project conventions, not session-specific learnings
- If a root CLAUDE.md already exists, update it; if only `.ralph/CLAUDE.md` exists, create a new root CLAUDE.md (the `.ralph/` one is Ralph agent instructions, not project docs)

---

### P18-US-004: Inline Help Text Audit for All Commands

**Description:** Audit all 22 command and subcommand `--help` descriptions in `main.rs` to ensure they match Go CLI tone, include consistent formatting, and have accurate flag descriptions. Fix any stale, missing, or inconsistent help text.

**Acceptance Criteria:**
- Every `Commands` enum variant has a `///` doc comment with:
  - First line: imperative verb + concise description (e.g., "Execute a deployment script")
  - Additional lines: brief explanation of behavior and key flags
- Every `#[arg(...)]` has a `help` or doc comment describing the flag
- Flag names are consistent across commands: `--json`, `--verbose`, `--broadcast`, `--dry-run`, `--network`, `--namespace`, `--rpc-url`, `--non-interactive` use identical descriptions where they appear on multiple commands
- Subcommands (`fork`, `dev anvil`, `config`, `migrate`) have consistent formatting with their parent command descriptions
- `--json` flag help text on every command that supports it mentions "Output as JSON"
- `--non-interactive` flag help text mentions all detection methods (flag, env vars, TTY)
- `--broadcast` flag help text mentions the `--non-interactive` requirement when combined with `--json`
- All existing golden file tests pass (`cargo test -p treb-cli`) — help text changes should not affect golden files since they test command output, not `--help` output
- `cargo clippy --workspace --all-targets` passes with no new warnings
- Run `cargo run -p treb-cli -- --help` and `cargo run -p treb-cli -- <cmd> --help` for each command to verify output renders correctly

**Notes:**
- Focus on `crates/treb-cli/src/main.rs` where all clap structs are defined
- Do NOT change command names, flag names, or flag behavior — this is a text-only audit
- If a command's help text already matches Go style, leave it unchanged
- The Go CLI used cobra (Go CLI framework) which has similar imperative-verb conventions to clap

## Functional Requirements

- **FR-1:** README.md must be a self-contained getting-started guide — a user with no prior treb knowledge should be able to install and run their first deployment by following it
- **FR-2:** MIGRATION.md must enumerate every behavioral difference between Go and Rust CLIs that could affect user workflows or scripts
- **FR-3:** CHANGELOG.md must cover all features added across 18 phases, not just the final state
- **FR-4:** CLAUDE.md must document crate responsibilities and testing patterns so new contributors (or Claude Code agents) can navigate the codebase
- **FR-5:** Help text must use imperative verb style consistently ("Execute a deployment script", not "Executes a deployment script" or "A command to execute deployment scripts")
- **FR-6:** All documentation must reference the correct GitHub org (`trebuchet-org`) and repo (`treb-cli-rs`)
- **FR-7:** Installation instructions must document both `trebup` (recommended) and building from source

## Non-Goals

- **API documentation**: No rustdoc generation or library API docs — this project is a CLI binary, not a library
- **Website or hosted docs**: All documentation lives in the repository as markdown files
- **Tutorial content**: The README quick start is sufficient; no separate tutorial or cookbook
- **Localization**: English only
- **Man pages**: Shell completions are provided but man page generation is out of scope
- **Go CLI deprecation notice**: Updating the Go CLI repo is out of scope for this project
- **Video or visual content**: Text-only documentation

## Technical Considerations

- **Dependencies:** All 17 prior phases must be merged to the integration branch before this phase begins, as documentation must reflect the final command set, flags, and behavior
- **Golden file stability:** Help text changes (P18-US-004) modify clap `///` doc comments, which affect `--help` output but should NOT affect golden file tests (which test command execution output, not help text). However, if any golden tests capture `--help` output, they will need regeneration with `UPDATE_GOLDEN=1`
- **CLAUDE.md scope:** The root CLAUDE.md is loaded into every Claude Code conversation — keep it under 150 lines to avoid consuming context window. Detailed patterns belong in the auto-memory files, not CLAUDE.md
- **README command table:** Derive from the `Commands` enum in `main.rs` to ensure completeness — currently 22 commands counting fork/dev/config subcommands
- **Registry file format:** Document the registry files (`deployments.json`, `transactions.json`, `safe-txs.json`, `governor-proposals.json`, `lookup.json`) in MIGRATION.md since Go users will encounter new files
- **Build from source quirks:** Document that `git clone --recurse-submodules` is required (treb-sol nested submodules), Rust nightly is required (edition 2024), and the binary is named `treb-cli` not `treb` when built locally
