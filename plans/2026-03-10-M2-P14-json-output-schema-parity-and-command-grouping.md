# PRD: Phase 14 - JSON Output Schema Parity and Command Grouping

## Introduction

Phase 14 is the final phase of the Exact 1:1 Output Parity project. Phases 1-13 ported all human-readable output from the Go CLI to the Rust CLI. This phase audits every command's `--json` output to ensure schema-identical JSON with the Go CLI, ports command grouping from Go's `root.go` (Main Commands vs Management Commands), and aligns `--help` descriptions.

The Go CLI only supports `--json` on four commands (list, show, register, fork diff). Of those, list and show were already ported in Phases 4 and 5. This phase fixes the remaining two (register, fork diff) and audits all Rust-only JSON outputs for camelCase/null-vs-omit consistency. The command grouping and help text alignment ensures the CLI help output matches Go.

## Goals

1. **Register JSON schema matches Go exactly**: Output `{"deployments":[{"deploymentId":"...","address":"...","contractName":"...","label":"..."}]}` — matching Go's 4-field-per-entry wrapper structure.
2. **Fork diff JSON schema matches Go exactly**: Output `{"network":"...","newDeployments":[...],"modifiedDeployments":[...],"newTransactionCount":N,"hasChanges":bool}` with per-entry `{id, contractName, address, type, changeType}`.
3. **All JSON outputs use consistent conventions**: Every `--json` command uses `print_json()` for sorted keys, `camelCase` field names, and correct null-vs-omit behavior via `skip_serializing_if`.
4. **Command grouping matches Go**: `--help` shows "Main Commands" (init, list, show, gen-deploy, run, verify, compose, fork) and "Management Commands" (sync, tag, register, dev, networks, prune, reset, config, migrate).
5. **Help descriptions match Go**: Short and long descriptions for all commands align with Go CLI text.

## User Stories

### P14-US-001: Fix register Command JSON Schema to Match Go

**Description:** The Go CLI's register `--json` outputs `{"deployments":[{"deploymentId":"...","address":"...","contractName":"...","label":"..."}]}` — a simple wrapper with exactly 4 fields per deployment entry. The Rust CLI currently outputs a `RegisterOutputJson` struct with 6 top-level fields (`success`, `txHash`, `chainId`, `mode`, `deployments`, `transactionId`) and 5 fields per deployment (`id`, `contractName`, `address`, `namespace`, `chainId`). This must be changed to match Go's schema.

**Changes:**
- In `crates/treb-cli/src/commands/register.rs`:
  - Replace `RegisterOutputJson` with a Go-matching wrapper containing only `deployments` array
  - Rename `RegisteredDeploymentJson.id` to `deployment_id` with `#[serde(rename = "deploymentId")]`
  - Add `label` field to `RegisteredDeploymentJson`
  - Remove `namespace` and `chain_id` from `RegisteredDeploymentJson`
  - Update the JSON output call site to construct the new wrapper
- Update any tests asserting on register JSON output fields

**Acceptance Criteria:**
- `treb register --json` outputs `{"deployments":[{"address":"0x...","contractName":"Foo","deploymentId":"default/31337/Foo","label":"v1"}]}` with exactly 4 fields per entry
- Keys are sorted alphabetically (via `print_json()`)
- `label` field is always present (empty string when no label, matching Go's non-omitempty behavior)
- `cargo clippy --workspace --all-targets` passes
- `cargo test -p treb-cli` passes (with `UPDATE_GOLDEN=1` if needed)

---

### P14-US-002: Fix fork diff Command JSON Schema to Match Go

**Description:** The Go CLI's `fork diff --json` outputs a `ForkDiffResult` struct: `{"network":"...","newDeployments":[...],"modifiedDeployments":[...],"newTransactionCount":N,"hasChanges":bool}` where each deployment entry has `{id, contractName, address, type, changeType}` with `contractName` and `address` having `omitempty`. The Rust CLI currently outputs `{"network":"...","changes":[{"change":"...","file":"...","key":"..."}],"clean":bool}` — a completely different structure.

**Changes:**
- In `crates/treb-cli/src/commands/fork.rs`:
  - Create a `ForkDiffResultJson` struct matching Go's `ForkDiffResult`: `network`, `new_deployments`, `modified_deployments`, `new_transaction_count`, `has_changes`
  - Create a `ForkDiffEntryJson` struct matching Go's `ForkDiffEntry`: `id`, `contract_name` (skip_serializing_if empty), `address` (skip_serializing_if empty), `type` (renamed from `deployment_type`), `change_type`
  - Populate `new_deployments` and `modified_deployments` from the existing `new_deployments` and `modified_deployments` vectors already collected for human output
  - Replace the `serde_json::json!` inline construction with the typed struct
- Update fork diff JSON golden files and test assertions

**Acceptance Criteria:**
- `treb fork diff --json` outputs `{"hasChanges":true,"modifiedDeployments":[...],"network":"mainnet","newDeployments":[...],"newTransactionCount":0}` matching Go schema
- Each deployment entry contains `{address, changeType, contractName, id, type}` (sorted keys)
- `contractName` and `address` are omitted when empty (matching Go `omitempty`)
- `cargo clippy --workspace --all-targets` passes
- `cargo test -p treb-cli` passes (with `UPDATE_GOLDEN=1` if needed)

---

### P14-US-003: Audit All JSON Schemas for Consistency

**Description:** With register and fork diff fixed, audit every remaining command's `--json` output for consistency: camelCase field names, null-vs-omit behavior, and usage of `print_json()`. Commands with `--json` only in Rust (not in Go) are acceptable as additive features but must follow consistent conventions.

**Changes:**
- In each command file under `crates/treb-cli/src/commands/`:
  - Verify all JSON output structs use `#[serde(rename_all = "camelCase")]`
  - Verify all optional fields use `#[serde(skip_serializing_if = "Option::is_none")]` (matching Go `omitempty`)
  - Verify all JSON output goes through `output::print_json()` for sorted keys
  - Verify empty Vec fields use `#[serde(skip_serializing_if = "Vec::is_empty")]` where Go uses `omitempty` on slices
  - Fix any inconsistencies found
- Commands to audit: run, verify, compose, fork (enter/exit/revert/restart/status/history), sync, tag, prune, reset, dev anvil, networks, config, gen-deploy, migrate

**Acceptance Criteria:**
- All JSON output structs use `#[serde(rename_all = "camelCase")]`
- All optional fields use appropriate `skip_serializing_if` attributes
- All JSON output calls go through `output::print_json()` (not raw `println!` with `serde_json::to_string_pretty`)
- No `null` values appear in JSON where Go would omit the field entirely
- `cargo clippy --workspace --all-targets` passes
- `cargo test -p treb-cli` passes

---

### P14-US-004: Add Command Grouping to Match Go Help Output

**Description:** The Go CLI (`root.go`) organizes commands into two groups: "Main Commands" (init, list, show, gen-deploy, run, verify, compose, fork) and "Management Commands" (sync, tag, register, dev, networks, prune, reset, config, migrate). The Rust CLI currently has a flat command list. Add clap `help_heading` attributes to group commands identically.

**Changes:**
- In `crates/treb-cli/src/main.rs`:
  - Add `#[command(help_heading = "Main Commands")]` to: `Init`, `List`, `Show`, `GenDeploy`, `Run`, `Verify`, `Compose`, `Fork`
  - Add `#[command(help_heading = "Management Commands")]` to: `Sync`, `Tag`, `Register`, `Dev`, `Networks`, `Prune`, `Reset`, `Config`, `Migrate`
  - Keep `Version` and `Completions` without a heading (they appear under default "Commands" or at top level)
  - Verify ordering within groups matches Go (init → list → show → gen → run → verify → compose → fork for Main; sync → tag → register → dev → networks → prune → reset → config → migrate for Management)

**Acceptance Criteria:**
- `treb --help` shows commands grouped under "Main Commands" and "Management Commands" headings
- Command ordering within each group matches Go CLI
- `Version` and `Completions` appear outside the two main groups
- `cargo clippy --workspace --all-targets` passes
- `cargo test -p treb-cli` passes

---

### P14-US-005: Align --help Descriptions with Go CLI

**Description:** Update the short (`about`) and long (`long_about`) descriptions for all commands and the root CLI to match Go's cobra command definitions from `root.go` and individual command files.

**Changes:**
- In `crates/treb-cli/src/main.rs`:
  - Update root CLI `about` to: "Smart contract deployment orchestrator for Foundry"
  - Update root CLI `long_about` to: "Trebuchet (treb) orchestrates Foundry script execution for deterministic smart contract deployments using CreateX factory contracts."
  - Update each command's doc-comment (`///`) to match Go's `Short` description
  - Update `long_about` or extended doc-comments to match Go's `Long` description where they differ
- Key Go descriptions to match:
  - `init`: "Initialize treb in a Foundry project"
  - `list`: "List deployments from registry"
  - `show`: "Show detailed deployment information from registry"
  - `gen-deploy` (Go: `gen`/`generate`): "Generate deployment scripts"
  - `run`: "Run a Foundry script with treb infrastructure"
  - `verify`: "Verify contracts on block explorers"
  - `compose`: "Execute orchestrated deployments from a YAML configuration"
  - `fork`: "Manage network fork mode"
  - `sync`: "Sync registry with on-chain state"
  - `tag`: "Manage deployment tags"
  - `register`: "Register an existing contract deployment in the registry"
  - `dev`: "Development utilities"
  - `networks`: "List available networks from foundry.toml"
  - `prune`: "Prune registry entries that no longer exist on-chain"
  - `reset`: "Reset registry entries for the current namespace and network"
  - `config`: "Manage treb local config"
  - `migrate`: "Migrate config to new treb.toml accounts/namespace format"
- Also update fork subcommand descriptions and dev subcommand descriptions to match Go

**Acceptance Criteria:**
- `treb --help` root description matches Go CLI
- Each command's `--help` short description matches Go's `Short` field
- Each command's `--help` long description matches Go's `Long` field
- `cargo clippy --workspace --all-targets` passes
- `cargo test -p treb-cli` passes (with `UPDATE_GOLDEN=1` if needed for help text golden files)

---

### P14-US-006: Update Golden Files and Verify All Tests Pass

**Description:** After all schema, grouping, and help text changes, regenerate affected golden files and run the full test suite to ensure nothing is broken. This is a verification and cleanup story.

**Changes:**
- Run `UPDATE_GOLDEN=1 cargo test -p treb-cli` to regenerate all affected golden files
- Review the golden file diffs to verify they reflect only the intended changes
- Run `cargo test --workspace --all-targets` to verify all tests pass
- Run `cargo clippy --workspace --all-targets` for lint clean
- Fix any broken E2E test assertions (especially in `e2e/mod.rs` helpers that may parse JSON output)

**Acceptance Criteria:**
- All golden files reflect the updated JSON schemas and help text
- `cargo test --workspace --all-targets` passes with zero failures
- `cargo clippy --workspace --all-targets` passes with zero warnings
- No regressions in E2E workflow tests
- Golden file diffs reviewed and contain only expected changes (no accidental drift)

## Functional Requirements

- **FR-1:** Register `--json` output MUST produce `{"deployments":[{"address":"...","contractName":"...","deploymentId":"...","label":"..."}]}` with exactly these 4 fields per entry and no extra top-level fields.
- **FR-2:** Fork diff `--json` output MUST produce `{"hasChanges":bool,"modifiedDeployments":[...],"network":"...","newDeployments":[...],"newTransactionCount":N}` matching Go's `ForkDiffResult` struct.
- **FR-3:** Fork diff entries MUST include `{address, changeType, contractName, id, type}` with `address` and `contractName` omitted when empty (Go `omitempty`).
- **FR-4:** All JSON output MUST use `print_json()` for deterministic key sorting.
- **FR-5:** All JSON structs MUST use `#[serde(rename_all = "camelCase")]`.
- **FR-6:** Optional fields in JSON MUST use `skip_serializing_if` to omit rather than emit `null`, matching Go `omitempty` behavior.
- **FR-7:** `treb --help` MUST show "Main Commands" and "Management Commands" headings with commands grouped as in Go.
- **FR-8:** Short descriptions for all commands MUST match Go CLI's cobra `Short` field text.
- **FR-9:** Long descriptions for all commands MUST match Go CLI's cobra `Long` field text.
- **FR-10:** All golden files MUST be updated to reflect schema and help text changes.

## Non-Goals

- **No new JSON fields**: This phase does not add new fields to JSON output that Go doesn't have. Rust-only JSON features (commands that Go doesn't support with `--json`) are kept as-is.
- **No human output changes**: All human-readable output was finalized in Phases 1-13. This phase only touches JSON schemas, help text, and command grouping.
- **No removing Rust-only --json support**: Commands like `run --json`, `verify --json`, `sync --json` etc. that exist only in Rust are kept — they are additive features beyond Go parity.
- **No functional behavior changes**: Only output formatting, JSON schemas, and help text are modified. No command logic or business rules change.
- **No subcommand help grouping**: Only top-level commands are grouped. Subcommands (fork enter/exit/..., dev anvil/..., config show/set/...) retain their existing help structure.

## Technical Considerations

### Dependencies
- Phases 1-13 must be complete (all merged into the working branch). This is already the case.
- No new crate dependencies required. All changes use existing `serde`, `clap`, and `serde_json` features.

### Key Patterns
- `output::print_json()` handles recursive key sorting — all JSON output must go through this function for deterministic output.
- `#[serde(skip_serializing_if = "Option::is_none")]` is the canonical pattern for Go `omitempty` on pointer types.
- `#[serde(skip_serializing_if = "String::is_empty")]` matches Go `omitempty` on string fields.
- `#[serde(skip_serializing_if = "Vec::is_empty")]` matches Go `omitempty` on slice fields.
- `#[command(help_heading = "...")]` is the clap 4.x attribute for grouping commands under headings in `--help` output.

### Testing
- Golden files affected: register JSON golden files, fork diff JSON golden files, any help text golden files.
- E2E helpers in `crates/treb-cli/tests/e2e/mod.rs` may parse register or fork diff JSON — check for `as_array()` or field-name-specific assertions.
- Run targeted golden updates with `UPDATE_GOLDEN=1 cargo test -p treb-cli --test <test_binary>` to avoid unnecessary regeneration.

### Risks
- Changing register JSON schema is a breaking change for any consumers relying on the current Rust-only fields (`success`, `txHash`, `mode`, `transactionId`, `namespace`, `chainId`). This is acceptable since the goal is Go parity.
- Fork diff JSON structure change (`changes` array → `newDeployments`/`modifiedDeployments` split) is also breaking. Same rationale applies.
- Help text changes may affect CLI tests that assert on help output strings.
