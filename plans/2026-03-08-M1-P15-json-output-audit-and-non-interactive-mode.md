# PRD: Phase 15 - JSON Output Audit and Non-Interactive Mode

## Introduction

Phase 15 is a cross-cutting audit and hardening pass across all 22 CLI commands to ensure JSON output schema correctness and reliable non-interactive operation. Previous phases (4-14) implemented command functionality with human-readable and JSON output, but several commands have inconsistent JSON field naming (snake_case instead of the required camelCase), missing `--json` support, and no structured error output in JSON mode. Additionally, non-interactive mode detection relies solely on TTY checks and per-command `--non-interactive` flags — there is no centralized environment variable detection for CI/automation contexts.

This phase ensures `treb-cli-rs` is a reliable drop-in replacement for the Go CLI in automated pipelines, CI systems, and tooling that parses JSON output.

## Goals

1. **All JSON output uses consistent camelCase field naming** — every `Serialize` struct used for `--json` output has `#[serde(rename_all = "camelCase")]` and produces field names matching the Go CLI schema.
2. **Non-interactive mode is centrally detected** — a single utility checks `TREB_NON_INTERACTIVE=true`, `CI=true`, TTY state, and the `--non-interactive` CLI flag, replacing scattered TTY checks.
3. **Errors in JSON mode are structured** — when `--json` is set, errors output as `{"error": "message"}` to stderr with exit code 1, not raw text.
4. **All commands with meaningful output support `--json`** — fork subcommands (enter, exit, revert, restart, diff) gain JSON output.
5. **Golden file coverage exists for every JSON code path** — each command's `--json` output has a golden file test ensuring schema stability.

## User Stories

### P15-US-001: Non-Interactive Mode Detection Infrastructure

**Description:** Create a centralized `is_non_interactive()` utility that consolidates all non-interactive detection logic into one place, replacing the scattered TTY checks and per-command `--non-interactive` flag handling across `run.rs`, `compose.rs`, `prompt.rs`, and `selector.rs`.

**Current state:**
- `run.rs` line 177: `!non_interactive && Term::stdout().is_term()` for network selection
- `run.rs` line 313: `!non_interactive && io::stdin().is_terminal()` for broadcast confirmation
- `compose.rs` line 622: `io::stdin().is_terminal()` for execution confirmation
- `prompt.rs`: `Confirm` checks `Term::stdout().is_term()` internally
- `selector.rs`: Returns error if not in TTY
- No `TREB_NON_INTERACTIVE` or `CI` env var checks anywhere

**Changes:**
- Add `is_non_interactive()` function to `ui/` module (or a new `ui/interactive.rs`)
- Check: `TREB_NON_INTERACTIVE=true` OR `CI=true` OR `!stdin().is_terminal()`
- Accept optional `cli_flag: bool` parameter for the `--non-interactive` CLI flag override
- Update `run.rs` prompt guards to use `is_non_interactive(non_interactive)`
- Update `compose.rs` prompt guards to use `is_non_interactive(non_interactive)`
- Update `prompt.rs` `confirm()` to use `is_non_interactive(false)`
- Update `selector.rs` fuzzy select functions to use `is_non_interactive(false)`

**Acceptance Criteria:**
- [ ] `is_non_interactive()` function exists and checks all three sources (env vars + TTY)
- [ ] `TREB_NON_INTERACTIVE=true` suppresses all interactive prompts
- [ ] `CI=true` suppresses all interactive prompts
- [ ] Existing `--non-interactive` flag behavior unchanged
- [ ] `selector.rs` returns clear error message when non-interactive and no default available
- [ ] `prompt.rs` `confirm()` returns default value when non-interactive
- [ ] Typecheck passes (`cargo check -p treb-cli`)
- [ ] Existing tests pass (`cargo test -p treb-cli`)

---

### P15-US-002: JSON Error Output Wrapper

**Description:** When any command is invoked with `--json`, errors should be output as structured JSON (`{"error": "message"}`) to stderr instead of raw anyhow error text. This makes error handling reliable for tooling that parses CLI output.

**Current state:**
- `main()` returns `anyhow::Result<()>` — errors print as plain text via anyhow's Display impl
- No error formatting logic aware of the `--json` flag
- Exit code is already 1 on error (anyhow default)

**Changes:**
- In `main.rs`, wrap the command dispatch in a custom error handler
- Parse the `--json` flag from CLI args before dispatching (or capture it from the parsed `Cli` struct)
- On error, if `--json` was set, output `{"error": "<message>"}` to stderr via `eprintln!`
- Suppress anyhow's default error Display when JSON error is printed
- Use `std::process::exit(1)` after printing JSON error to avoid anyhow's own error output
- Keep exit code 1 for errors (matching Go CLI)

**Acceptance Criteria:**
- [ ] `treb run --json` with an invalid config outputs `{"error": "..."}` to stderr
- [ ] `treb list --json` with no registry outputs `{"error": "..."}` to stderr
- [ ] Error JSON has exactly one field: `error` (string)
- [ ] Exit code is 1 on error regardless of `--json`
- [ ] Exit code is 0 on success
- [ ] Normal (non-JSON) error output is unchanged when `--json` is not set
- [ ] Typecheck passes
- [ ] Golden file test for at least one command's JSON error output

---

### P15-US-003: Fix camelCase Annotations on JSON Output Structs

**Description:** Several JSON output structs are missing `#[serde(rename_all = "camelCase")]`, causing fields to serialize as snake_case instead of camelCase. This breaks schema compatibility with the Go CLI.

**Structs missing the annotation (confirmed by code audit):**
- `RunOutputJson` in `run.rs` — fields `dry_run`, `gas_used`, `console_logs` serialize as snake_case; `governor_proposals` has a manual `#[serde(rename)]` band-aid
- `DeploymentJson` in `run.rs` — fields `contract_name`, `chain_id`, `deployment_type` serialize as snake_case
- `SkippedJson` in `run.rs` — field `contract_name` serializes as snake_case
- `ConfigShowOutput` in `config.rs` — fields `config_source`, `project_root` serialize as snake_case
- `ComposeOutputJson` in `compose.rs` — no multi-word fields affected, but missing for consistency

**Changes:**
- Add `#[serde(rename_all = "camelCase")]` to all five structs listed above
- Remove the manual `#[serde(rename = "governorProposals")]` on `RunOutputJson.governor_proposals` (now redundant)
- Audit all other `Serialize` structs in `commands/` for missing annotations (verify the ones that already have it)
- Regenerate ALL affected golden files with `UPDATE_GOLDEN=1 cargo test -p treb-cli`

**Acceptance Criteria:**
- [ ] `RunOutputJson` serializes fields as `dryRun`, `gasUsed`, `consoleLogs`, `governorProposals`
- [ ] `DeploymentJson` serializes fields as `contractName`, `chainId`, `deploymentType`
- [ ] `SkippedJson` serializes field as `contractName`
- [ ] `ConfigShowOutput` serializes fields as `configSource`, `projectRoot`
- [ ] `ComposeOutputJson` has `#[serde(rename_all = "camelCase")]`
- [ ] No `Serialize` struct in `commands/` is missing `rename_all = "camelCase"` (except enum variants that intentionally use `snake_case`)
- [ ] All golden file tests pass after regeneration
- [ ] Typecheck passes

---

### P15-US-004: Add --json Support to Fork Subcommands

**Description:** Several fork subcommands lack `--json` output support. Fork status, history, and diff already have `--json`; but enter, exit, revert, and restart do not. These subcommands produce meaningful output that automation tooling needs to parse.

**Current state (from main.rs fork subcommands):**
- `fork status` — has `--json` ✓
- `fork history` — has `--json` ✓
- `fork diff` — need to verify; may need `--json`
- `fork enter` — no `--json`
- `fork exit` — no `--json`
- `fork revert` — no `--json`
- `fork restart` — no `--json`

**Changes:**
- Add `#[arg(long)] json: bool` to fork enter, exit, revert, and restart subcommand structs
- Create JSON output structs with `#[serde(rename_all = "camelCase")]` for each:
  - `ForkEnterJson`: `network`, `chainId`, `port`, `rpcUrl`, `snapshotId`, `pid`
  - `ForkExitJson`: `network`, `restoredEntries`, `cleanedUp`
  - `ForkRevertJson`: `network`, `snapshotId`, `newSnapshotId`
  - `ForkRestartJson`: `network`, `chainId`, `port`, `rpcUrl`, `snapshotId`
- Wire JSON output through `output::print_json()` in each handler
- Add `--json` to fork diff if not already present
- Add golden file tests for each fork subcommand's JSON output

**Acceptance Criteria:**
- [ ] `treb fork enter --json` outputs structured JSON to stdout
- [ ] `treb fork exit --json` outputs structured JSON to stdout
- [ ] `treb fork revert --json` outputs structured JSON to stdout
- [ ] `treb fork restart --json` outputs structured JSON to stdout
- [ ] All fork JSON structs use `#[serde(rename_all = "camelCase")]`
- [ ] Human-readable output unchanged when `--json` is not set
- [ ] Golden file tests exist for each fork subcommand's JSON mode
- [ ] Typecheck passes

---

### P15-US-005: Verbose and Stage Output Suppression in JSON Mode

**Description:** Audit all commands to ensure that verbose output, stage messages, warning banners, and any other human-readable stderr/stdout output does not leak into JSON mode. Some commands guard verbose output with `if verbose && !json`, but this pattern must be verified and enforced everywhere.

**Audit targets:**
- `run.rs`: Stage messages (`print_stage`), warning banners (`print_warning_banner`), verbose KV pairs (`eprint_kv`)
- `compose.rs`: Per-component stage messages, totals display, warning banners
- `verify.rs`: Per-verifier progress output
- `sync.rs`: Sync progress output
- `register.rs`: Registration progress
- `prune.rs`: Candidate listing, confirmation prompts
- `reset.rs`: Confirmation prompts, removal summary
- `tag.rs`: Tag operation display
- `gen_deploy.rs`: Stage messages, code preview
- `migrate.rs`: Stage messages, preview display
- `fork.rs`: Fork operation messages
- `dev.rs`: Anvil operation messages

**Changes:**
- Add `if !json` guards around all `print_stage()`, `print_warning_banner()`, `print_kv()`, `eprint_kv()` calls that are not already guarded
- Ensure `--verbose --json` produces ONLY the JSON object on stdout, with no stderr pollution
- Ensure `--debug --json` produces ONLY the JSON object on stdout (debug log file still written)
- Verify that interactive prompts are skipped in JSON mode (confirmation prompts should auto-accept or error)

**Acceptance Criteria:**
- [ ] `treb run --json` stdout contains ONLY valid JSON (no stage messages, no banners)
- [ ] `treb run --verbose --json` stdout contains ONLY valid JSON
- [ ] `treb compose --json` stdout contains ONLY valid JSON
- [ ] `treb verify --json` stdout contains ONLY valid JSON
- [ ] No command produces non-JSON text on stdout when `--json` is set
- [ ] `--debug --json` still writes debug log file but stdout is clean JSON
- [ ] Typecheck passes
- [ ] Existing golden file tests pass

---

### P15-US-006: Comprehensive JSON Golden File Test Coverage

**Description:** Add golden file tests for all JSON output paths that are not yet covered, and verify exit code behavior. This is the final validation step ensuring schema stability across all commands.

**Current golden file coverage (JSON):**
- version, config show, list, show, run (basic/governor/verbose), compose (dry run), networks, gen-deploy, verify (5 variants), tag (add/remove/show), register, sync (governor/empty), prune (dry run), reset, fork (status/history/diff), dev anvil status

**Missing or needed:**
- JSON error output (from P15-US-002)
- Fork enter/exit/revert/restart JSON (from P15-US-004)
- Non-interactive mode behavior tests (from P15-US-001)
- Run dry-run JSON output
- Compose non-dry-run JSON output
- Sync with Safe transactions JSON
- Prune non-dry-run JSON
- Exit code assertions for success and error cases

**Changes:**
- Add golden file tests for each new JSON output path
- Add integration tests asserting exit code 0 on success, 1 on error
- Add integration test for `TREB_NON_INTERACTIVE=true` suppressing prompts
- Add integration test for `CI=true` suppressing prompts
- Verify all existing JSON golden files still pass after P15-US-003 changes (regeneration)
- Use `IntegrationTest` builder with appropriate normalizers

**Acceptance Criteria:**
- [ ] Golden file test exists for every command that supports `--json`
- [ ] At least one golden file test for JSON error output format
- [ ] Integration test verifies exit code 0 on successful command
- [ ] Integration test verifies exit code 1 on failed command
- [ ] Integration test verifies `TREB_NON_INTERACTIVE=true` skips prompts
- [ ] Integration test verifies `CI=true` skips prompts
- [ ] All golden file tests pass (`cargo test -p treb-cli`)
- [ ] `UPDATE_GOLDEN=1 cargo test -p treb-cli` regenerates cleanly

## Functional Requirements

- **FR-1:** All JSON output structs must have `#[serde(rename_all = "camelCase")]` and produce camelCase field names matching the Go CLI schema.
- **FR-2:** Non-interactive mode is detected via `TREB_NON_INTERACTIVE=true`, `CI=true`, or non-TTY stdin, in addition to the existing `--non-interactive` CLI flag.
- **FR-3:** When `--json` is set and a command errors, stderr must contain `{"error": "<message>"}` as valid JSON.
- **FR-4:** Exit codes: 0 for success, 1 for any error. No other exit codes.
- **FR-5:** All `print_stage()`, `print_warning_banner()`, `print_kv()`, and `eprint_kv()` calls must be suppressed when `--json` is set.
- **FR-6:** All JSON output must go through `output::print_json()` which applies `sort_json_keys()` for deterministic key ordering.
- **FR-7:** Fork enter, exit, revert, and restart subcommands must support `--json` with camelCase output structs.
- **FR-8:** Interactive prompts (dialoguer Confirm, FuzzySelect, MultiSelect) must check the centralized `is_non_interactive()` and either return defaults or return clear errors.
- **FR-9:** `--verbose --json` and `--debug --json` must produce clean JSON on stdout with no human-readable output mixed in.

## Non-Goals

- **Schema changes beyond camelCase fixes** — this phase fixes naming convention issues only, not adding/removing fields from JSON output.
- **New command functionality** — no new features, only output format corrections and non-interactive infrastructure.
- **Go CLI source code comparison** — the audit uses existing golden files and project knowledge as the reference, not a live Go CLI installation.
- **`--json` for init, completions, dev anvil start/stop/logs** — these commands produce side effects or streaming output where JSON is not meaningful.
- **Custom exit codes** — only 0 and 1; no per-error-type exit codes.
- **JSON schema validation tooling** — no JSON Schema files or automated schema diffing; golden file tests are the validation mechanism.

## Technical Considerations

### Dependencies
- All command phases (4-14) must be merged into the working branch before this phase begins, as the audit touches every command handler.
- Golden file regeneration after camelCase fixes will cascade to ALL existing JSON golden files — plan for a bulk `UPDATE_GOLDEN=1` pass.

### Key Files
- `crates/treb-cli/src/main.rs` — CLI dispatch, error handling wrapper, `--json` flag extraction
- `crates/treb-cli/src/output.rs` — `print_json()`, `sort_json_keys()`, formatting utilities
- `crates/treb-cli/src/ui/color.rs` — `should_use_color()`, `NO_COLOR` handling (pattern for env var detection)
- `crates/treb-cli/src/ui/prompt.rs` — `confirm()` with TTY check
- `crates/treb-cli/src/ui/selector.rs` — `fuzzy_select_*` functions with TTY check
- `crates/treb-cli/src/commands/*.rs` — all command handlers with JSON output structs
- `crates/treb-cli/tests/framework/` — golden file test framework (`golden.rs`, `integration_test.rs`)
- `crates/treb-cli/tests/golden/` — golden file directory (30+ test scenarios)

### Patterns to Follow
- `#[serde(rename_all = "camelCase")]` on every `Serialize` struct used for JSON output
- `output::print_json(&value)` for all JSON output (never raw `serde_json::to_string_pretty`)
- `if !json { print_stage(...); }` guard pattern for human output in JSON mode
- `color::should_use_color()` pattern in `ui/color.rs` as template for `is_non_interactive()` env var detection
- `IntegrationTest::new(...)` builder pattern for golden file tests with normalizer chains

### Risks
- Changing `RunOutputJson` field names from snake_case to camelCase is a **breaking change** for any tooling currently parsing the Rust CLI's JSON output. This is intentional — the current snake_case output is a bug, and camelCase is the documented contract.
- Bulk golden file regeneration after camelCase fixes may mask unrelated regressions — review diffs carefully, not just "do they pass."
- The `serde_json` `preserve_order` feature (enabled transitively via alloy) means `to_value()` produces IndexMap ordering — all JSON must go through `sort_json_keys()` via `print_json()` for deterministic output.
