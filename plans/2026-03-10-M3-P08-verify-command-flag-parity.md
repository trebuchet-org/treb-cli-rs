# PRD: Phase 8 - Verify Command Flag Parity

## Introduction

Phase 8 brings the Rust `treb verify` command to full flag parity with the Go CLI. The verify command is already functional for basic verification workflows, but is missing several flags needed for production usage: namespace/network scoping, debug output, manual contract path specification, short flags for verifier selection, and the `--blockscout-verifier-url` convenience alias. This phase adds these flags without changing the core verification orchestration in `treb-verify`.

This is an independent phase in the CLI Compatibility and Drop-in Parity masterplan (M3). It builds on patterns established in Phase 5 (global flags, short flags, `build.rs` mirroring) and Phase 6 (scoped deployment resolution with `filter_deployments()` / `resolve_deployment_in_scope()`).

## Goals

1. **Verifier short flags**: `-e`, `-b`, `-s` short flags for `--etherscan`, `--blockscout`, `--sourcify` match Go CLI usage.
2. **Scoped resolution**: `--namespace` and `--network` flags filter verification scope using the same `DeploymentFilters` + `resolve_deployment_in_scope()` pattern from Phase 6.
3. **Contract path override**: `--contract-path` allows manual source path specification (e.g., `./src/Counter.sol:Counter`), bypassing artifact-based path resolution.
4. **Debug output**: `--debug` flag prints the forge verify command before execution, matching Go CLI diagnostics.
5. **Blockscout URL alias**: `--blockscout-verifier-url` accepted as an alias for `--verifier-url`, matching Go flag naming.

## User Stories

### P8-US-001: Add Short Flags for Verifier Selection and Blockscout URL Alias

**Description:** Add `-e` (etherscan), `-b` (blockscout), `-s` (sourcify) short flags to the verify command's clap definition and `build.rs` completion builder. Also add `--blockscout-verifier-url` as a visible alias for `--verifier-url`.

**Files to modify:**
- `crates/treb-cli/src/main.rs` — Add `short = 'e'` to `etherscan`, `short = 'b'` to `blockscout`, `short = 's'` to `sourcify` in the `Verify` variant. Add `visible_alias = "blockscout-verifier-url"` to `verifier_url`.
- `crates/treb-cli/build.rs` — Mirror `-e`, `-b`, `-s` short flags and `--blockscout-verifier-url` alias on the `build_verify()` function. Add the missing `--etherscan`, `--blockscout`, `--sourcify` boolean flags (currently absent from `build_verify()`).

**Acceptance criteria:**
- `treb verify Counter -e` parses and selects etherscan verifier
- `treb verify Counter -b` parses and selects blockscout verifier
- `treb verify Counter -s` parses and selects sourcify verifier
- `treb verify Counter -ebs` parses and selects all three verifiers
- `treb verify Counter --blockscout-verifier-url https://example.com/api` parses and populates `verifier_url`
- `parse_cli_from(...)` unit tests in `main.rs` pin all short flag and alias parsing
- `cargo check --workspace` passes
- `cargo clippy --workspace --all-targets` passes

---

### P8-US-002: Add --namespace and --network Scoping Flags

**Description:** Add `--namespace` and `--network` / `-n` flags to the verify command's clap definition, `build.rs` completion builder, and wire them into the `verify::run()` function signature. Apply the Phase 6 scoped resolution pattern: build `DeploymentFilters`, call `filter_deployments()`, use `resolve_deployment_in_scope()` for single-deployment resolution, and pass filtered deployments to interactive selection and batch verification.

**Files to modify:**
- `crates/treb-cli/src/main.rs` — Add `namespace: Option<String>` and `network: Option<String>` (with `short = 'n'`) to `Verify` variant. Update `Commands::Verify` dispatch to pass them to `verify::run()`.
- `crates/treb-cli/build.rs` — Add `--namespace` and `--network` / `-n` to `build_verify()`.
- `crates/treb-cli/src/commands/verify.rs` — Update `run()` signature to accept `namespace: Option<String>` and `network: Option<String>`. Add a `VerifyScope` struct (following `ShowScope`/`TagScope` pattern) with `as_deployment_filters()`, `context_suffix()`, `no_deployments_message()`, and `decorate_resolution_error()`. In single-deployment path: filter `list_deployments()`, use `resolve_deployment_in_scope()`, pass filtered slice to `fuzzy_select_deployment_id()`. In batch path (`run_batch()`): accept and apply filters to candidate deployments.

**Acceptance criteria:**
- `treb verify Counter --namespace staging` resolves only within `staging` namespace
- `treb verify Counter -n mainnet` resolves only within `mainnet` network
- `treb verify --all --namespace staging` only verifies deployments in `staging` namespace
- Interactive selection (no deployment arg) shows only filtered deployments when scope flags are provided
- Scoped resolution error messages include context suffix (e.g., "in namespace 'staging'")
- `parse_cli_from(...)` unit tests pin `--namespace` and `-n` parsing
- Typecheck passes (`cargo check --workspace`)

---

### P8-US-003: Add --contract-path Flag and Thread Through Verification

**Description:** Add `--contract-path <PATH>` flag for manually specifying the contract source path (e.g., `./src/Counter.sol:Counter`). When provided, this overrides the automatic `parse_contract_info()` path derivation from the deployment artifact. Thread the value through `VerifyOpts` to `build_verify_args()`.

**Files to modify:**
- `crates/treb-cli/src/main.rs` — Add `contract_path: Option<String>` to `Verify` variant. Pass to `verify::run()`.
- `crates/treb-cli/build.rs` — Add `--contract-path` to `build_verify()`.
- `crates/treb-verify/src/lib.rs` — Add `contract_path: Option<String>` to `VerifyOpts`. In `build_verify_args()`, when `opts.contract_path` is `Some`, parse it as a `ContractInfo` directly (split on `:` for `path:name`) instead of using `parse_contract_info()` from the artifact.
- `crates/treb-cli/src/commands/verify.rs` — Update `run()` signature to accept `contract_path: Option<String>`. Pass through to `VerifyOpts` construction in both single and batch paths.

**Acceptance criteria:**
- `treb verify CounterProxy --contract-path "./src/Counter.sol:Counter"` uses the provided path instead of artifact-derived path
- `--contract-path` value is split on `:` — left side becomes `ContractInfo.path`, right side becomes `ContractInfo.name`
- If `--contract-path` contains no `:`, the entire value is used as `path` and the deployment's `contract_name` is used as `name`
- Unit tests in `treb-verify/src/lib.rs` cover both `--contract-path` override and fallback to artifact path
- `VerifyOpts` construction sites in `verify.rs` (single + batch) both pass `contract_path`
- Typecheck passes (`cargo check --workspace`)

---

### P8-US-004: Add --debug Flag for Forge Verify Command Output

**Description:** Add `--debug` flag that prints the forge verify command arguments before execution, matching Go CLI diagnostic behavior. Thread through `VerifyOpts` to the verification execution path.

**Files to modify:**
- `crates/treb-cli/src/main.rs` — Add `debug: bool` to `Verify` variant. Pass to `verify::run()`.
- `crates/treb-cli/build.rs` — Add `--debug` to `build_verify()`.
- `crates/treb-verify/src/lib.rs` — Add `debug: bool` to `VerifyOpts`.
- `crates/treb-cli/src/commands/verify.rs` — Update `run()` signature to accept `debug: bool`. Pass through to `VerifyOpts`. Before calling `verify_args.run()`, if `debug` is true, print the `VerifyArgs` to stderr (use the `Debug` or display format of the forge args to show what would be executed).

**Acceptance criteria:**
- `treb verify Counter --debug` prints forge verify arguments to stderr before executing
- Debug output appears for each verifier in both single and batch modes
- Debug output does not appear when `--debug` is not specified
- `parse_cli_from(...)` unit test pins `--debug` parsing
- Typecheck passes (`cargo check --workspace`)

---

### P8-US-005: Update Golden Tests, Help Snapshots, and Add Integration Tests

**Description:** Refresh help golden snapshots to reflect all new flags, and add integration test coverage for the new flag parsing and scoped resolution behavior.

**Files to modify:**
- `crates/treb-cli/tests/integration_help.rs` — Add a `help_verify` golden test that snapshots `treb verify --help` output.
- `crates/treb-cli/tests/integration_verify.rs` — Add subprocess tests for: (1) short flag parsing (`-e`, `-b`, `-s`), (2) scoped resolution with `--namespace`/`--network` using seeded registry data, (3) `--blockscout-verifier-url` alias acceptance. Use `NO_COLOR=1` for stable output assertions.
- `crates/treb-cli/tests/golden/help_root/` — Refresh root help snapshot if new flags appear in root help.
- `crates/treb-cli/tests/golden/help_verify/` — New golden directory for verify help snapshot.

**Acceptance criteria:**
- `help_verify` golden test passes and snapshots all new flags (`-e`, `-b`, `-s`, `-n`, `--namespace`, `--network`, `--contract-path`, `--debug`, `--blockscout-verifier-url`)
- Integration tests verify scoped resolution filters deployments correctly
- Integration tests verify short flags are accepted without error
- `UPDATE_GOLDEN=1 cargo test -p treb-cli --test integration_help` refreshes all help snapshots cleanly
- All existing golden tests pass (no unexpected regressions)
- `cargo test --workspace --all-targets` passes
- `cargo clippy --workspace --all-targets` passes

## Functional Requirements

- **FR-1:** `-e` is the short flag for `--etherscan`, `-b` for `--blockscout`, `-s` for `--sourcify` on the verify command.
- **FR-2:** `--namespace <NS>` filters verification scope to deployments in the given namespace.
- **FR-3:** `--network <NET>` / `-n <NET>` filters verification scope to deployments on the given network/chain.
- **FR-4:** Scoping flags apply to both single-deployment and `--all` batch verification.
- **FR-5:** Interactive deployment selection (no positional arg) respects scoping filters.
- **FR-6:** `--contract-path <PATH>` overrides artifact-derived contract source path for forge verify.
- **FR-7:** `--debug` prints forge verify arguments to stderr before execution.
- **FR-8:** `--blockscout-verifier-url` is a visible alias for `--verifier-url`.
- **FR-9:** All new flags are mirrored in `build.rs` for shell completion generation.
- **FR-10:** Scoped resolution error messages include namespace/network context (following `ShowScope`/`TagScope` pattern).

## Non-Goals

- **No changes to verification orchestration logic** in `treb-verify`. The forge verify execution, retry logic, and result handling remain unchanged.
- **No `--no-fork` flag on verify.** Go does not have this, and the existing fork-mode blocking check is sufficient.
- **No changes to JSON output schema.** Existing `--json` output structure stays the same.
- **No `--watch` or `--retries` behavior changes.** These flags already exist and work correctly.
- **No changes to the `--verifier` flag behavior.** The shorthand boolean flags (`-e`, `-b`, `-s`) already override `--verifier` in `main.rs` dispatch logic; this mechanism stays unchanged.

## Technical Considerations

### Patterns to Follow

- **Scoped resolution (Phase 6 pattern):** `VerifyScope` struct with `as_deployment_filters()`, `context_suffix()`, `no_deployments_message()`, `decorate_resolution_error()` — identical to `ShowScope` and `TagScope`.
- **Short flags (Phase 5 pattern):** Define in derive parser + mirror in `build.rs`. Pin with `parse_cli_from(...)` unit tests.
- **Flag aliases (Phase 7 pattern):** Use `visible_alias` on clap args. Set `value_name` explicitly to avoid leaking internal field names.

### Key Integration Points

- `crates/treb-cli/src/commands/verify.rs` — Main file receiving most changes (scoped resolution, new parameters).
- `crates/treb-verify/src/lib.rs` — `VerifyOpts` gains `contract_path` and `debug` fields; `build_verify_args()` gains contract path override logic.
- `crates/treb-cli/src/main.rs` — Verify variant gains 5 new fields; dispatch block passes them through.
- `crates/treb-cli/build.rs` — `build_verify()` gains all new flags/aliases for shell completions.

### Short Flag Conflict Check

- `-s` on verify: used for `--sourcify` (matches Go). No conflict — `-s` is `--namespace` on `list`/`tag` but each command has its own flag namespace.
- `-n` on verify: used for `--network` (matches Go). Same short flag as `list`/`tag` for the same meaning.
- `-e` on verify: used for `--etherscan` (new, matches Go). No existing usage.
- `-b` on verify: used for `--blockscout` (new, matches Go). No existing usage.

### Dependencies

- **Phase 6 infrastructure (already complete):** `DeploymentFilters`, `filter_deployments()`, `resolve_deployment_in_scope()`, `fuzzy_select_deployment_id()` with filtered slices.
- **Phase 5 infrastructure (already complete):** Global `non_interactive` on `Cli`, mirroring pattern in `build.rs`.
