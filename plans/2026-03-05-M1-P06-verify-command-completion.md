# PRD: Phase 6 - verify Command Completion

## Introduction

Phase 6 completes the `treb verify` command to reach full feature parity with the Go CLI. The core verification infrastructure already exists: forge-verify integration handles API submission/polling for all three block explorers, `--all` batch mode works, `--force`/`--watch`/`--retries`/`--delay` are wired up, and the registry stores per-verifier results. What remains is multi-verifier support (verify against multiple explorers in one invocation), per-verifier shorthand flags, environment variable API key resolution, and styled output using the Phase 3 formatting framework. This phase also updates golden files to reflect the new output format.

## Goals

1. **Multi-verifier support**: Users can verify a contract against multiple block explorers (etherscan, sourcify, blockscout) in a single invocation, with aggregate PARTIAL status when results are mixed.
2. **Per-verifier shorthand flags**: `--etherscan`, `--blockscout`, `--sourcify` boolean flags for ergonomic verifier selection, matching Go CLI behavior.
3. **Environment-based API key resolution**: Verifier API keys resolve from standard environment variables (`ETHERSCAN_API_KEY`, `BLOCKSCOUT_API_KEY`) when not provided via CLI flag.
4. **Styled output parity**: Verification output uses Phase 3 formatting (stage indicators, styled status, per-verifier tree with URLs) consistent with `show` command's verification section.
5. **Golden file coverage**: All verify output paths have golden file tests exercising single, batch, and multi-verifier scenarios.

## User Stories

### P6-US-001: Per-Verifier Shorthand Flags and Verifier List Resolution

**Description:** Add `--etherscan`, `--blockscout`, `--sourcify` boolean CLI flags as shorthand alternatives to `--verifier`. When one or more shorthand flags are set, they define the list of verifiers to use (replacing `--verifier`). When multiple shorthand flags are set, verification runs against all selected verifiers. When no shorthand flags are set, fall back to `--verifier` (default: "etherscan"). Refactor internal data flow from single verifier string to a `Vec<String>` of selected verifiers.

**Acceptance Criteria:**
- `treb verify <dep> --etherscan` verifies against etherscan only
- `treb verify <dep> --etherscan --sourcify` verifies against both etherscan and sourcify
- `treb verify <dep> --etherscan --blockscout --sourcify` verifies against all three
- `treb verify <dep>` (no flags) defaults to etherscan (via `--verifier` default)
- `treb verify <dep> --verifier sourcify` still works for single verifier selection
- Shorthand flags override `--verifier` when both are specified
- `verify.rs::run()` and `run_batch()` accept `&[String]` instead of `&str` for verifier parameter
- CLI arg definitions added to `Commands::Verify` in `main.rs`
- Typecheck passes (`cargo check -p treb-cli`)
- Existing unit tests in `cli_verify.rs` updated for new flag parsing

**Key Files:**
- `crates/treb-cli/src/main.rs` (add flags to Verify variant)
- `crates/treb-cli/src/commands/verify.rs` (refactor run/run_batch signatures)

---

### P6-US-002: Multi-Verifier Verification Loop and Aggregate Status

**Description:** Implement the loop that verifies a single deployment against multiple verifiers in sequence. After all verifiers complete, compute an aggregate `VerificationStatus`: VERIFIED if all pass, FAILED if all fail, PARTIAL if mixed results. Update the registry with all per-verifier entries and the aggregate status in one pass. This applies to both single deployment and batch verification paths.

**Acceptance Criteria:**
- Single deployment verification against N verifiers runs each verifier sequentially
- Each verifier result is recorded in `deployment.verification.verifiers` HashMap with lowercase key (etherscan, sourcify, blockscout)
- Aggregate status: all VERIFIED -> `VerificationStatus::Verified`, all FAILED -> `VerificationStatus::Failed`, mixed -> `VerificationStatus::Partial`
- `etherscan_url` field populated from first successful etherscan verification (for backward compat with show command)
- `verified_at` set to timestamp of first successful verification
- Registry `update_deployment()` called once after all verifiers complete (not per-verifier)
- Rate limiting delay (`--delay`) applied between verifier attempts
- `--force` flag applies to all selected verifiers
- Typecheck passes
- Unit tests covering aggregate status computation: all-pass, all-fail, mixed

**Key Files:**
- `crates/treb-cli/src/commands/verify.rs` (loop + aggregate logic)
- `crates/treb-core/src/types/enums.rs` (VerificationStatus::Partial already exists)

---

### P6-US-003: Per-Verifier API Key Resolution from Environment

**Description:** When `--verifier-api-key` is not provided on the CLI, resolve API keys from standard environment variables per verifier. Etherscan uses `ETHERSCAN_API_KEY`, Blockscout uses `BLOCKSCOUT_API_KEY`. Sourcify does not require an API key. This allows users to set keys once in `.env` and verify against multiple providers without repeating keys on the CLI.

**Acceptance Criteria:**
- When verifier is "etherscan" and no `--verifier-api-key`, check `ETHERSCAN_API_KEY` env var
- When verifier is "blockscout" and no `--verifier-api-key`, check `BLOCKSCOUT_API_KEY` env var
- When verifier is "sourcify", no API key lookup needed (sourcify is keyless)
- Explicit `--verifier-api-key` always takes precedence over env var for ALL verifiers
- When multiple verifiers are selected, each gets its own key from its respective env var
- Update `VerifyOpts` construction to set per-verifier `verifier_api_key` and `etherscan_api_key` fields appropriately
- Typecheck passes
- Unit tests: env var resolution for each verifier type, CLI override precedence

**Key Files:**
- `crates/treb-cli/src/commands/verify.rs` (key resolution before `VerifyOpts` construction)

---

### P6-US-004: Styled Single Deployment Verification Output

**Description:** Replace plain `eprintln!()` calls in the single deployment verification path with Phase 3 output utilities. Use `print_stage()` for verification progress, styled status strings using VERIFIED/FAILED/UNVERIFIED color styles, and per-verifier result display matching the `show` command's verification section format. The already-verified skip message should also be styled.

**Acceptance Criteria:**
- Verification start shows stage indicator: `print_stage("\u{1f50d}", "Verifying ContractName on etherscan...")` (or similar search emoji)
- Per-verifier results displayed with styled status: green for VERIFIED, red for FAILED
- Successful verification shows explorer URL indented under verifier name
- Failed verification shows reason indented under verifier name
- Multi-verifier results rendered as a list (verifier: status, URL/reason per line)
- Already-verified skip message uses `print_warning_banner()` or styled output
- All styled output gated on `color::is_color_enabled()` using `styled()` pattern
- Completion shows verification_badge summary (e.g., `e[V] s[X] b[-]`)
- Non-JSON output goes to stderr for progress, stdout for results (matching run command convention)
- Typecheck passes

**Key Files:**
- `crates/treb-cli/src/commands/verify.rs` (single path output)
- Uses: `output::print_stage`, `output::print_kv`, `ui::color::VERIFIED/FAILED/UNVERIFIED`, `ui::badge::verification_badge`

---

### P6-US-005: Styled Batch Verification Output

**Description:** Replace batch verification progress and summary output with Phase 3 styled formatting. Each deployment gets a stage indicator with counter. Per-deployment results show per-verifier status. The summary at the end uses a styled table or tree showing all results with verification badges.

**Acceptance Criteria:**
- Batch progress shows `print_stage()` per deployment: `[1/5] Verifying ContractName (0x1234...abcd)`
- Per-verifier result within each deployment shows styled status inline
- Summary output uses `build_table()` with styled Status column (green VERIFIED, red FAILED, yellow PARTIAL)
- Summary table columns: Contract, Address, Status, Badge (verification_badge format)
- When all deployments verified successfully, show success stage emoji at end
- Failed deployments in summary show reason
- `--all --force` output distinguished from regular `--all` (shows "re-verifying" vs "verifying")
- Non-JSON output: progress to stderr, summary to stdout
- Typecheck passes

**Key Files:**
- `crates/treb-cli/src/commands/verify.rs` (batch path output)
- Uses: `output::print_stage`, `output::build_table`, `output::print_table`, `ui::badge::verification_badge`

---

### P6-US-006: Multi-Verifier JSON Output Schema

**Description:** Update `VerifyOutputJson` to support multi-verifier results. For single deployment verification, JSON output should include a `verifiers` object with per-verifier detail. For batch verification, each array element includes the verifiers breakdown. The overall `status` field reflects the aggregate (VERIFIED/PARTIAL/FAILED). Ensure all JSON output uses `print_json()` (which calls `sort_json_keys()`) for deterministic key ordering.

**Acceptance Criteria:**
- `VerifyOutputJson` gains `verifiers: HashMap<String, VerifierResultJson>` field (or equivalent)
- `VerifierResultJson` has `status`, `url`, `reason` fields (camelCase serde)
- Single deployment JSON: `status` is aggregate, `verifiers` contains per-verifier breakdown
- Batch JSON array: each element has aggregate status + verifiers breakdown
- `verifier` field (singular) removed or kept as primary verifier for backward compat (decide based on Go schema)
- All JSON output paths use `output::print_json()` for sorted keys
- `--json` suppresses all non-JSON output (no stderr progress leaks)
- Typecheck passes
- Schema matches Go CLI `treb verify --json` format

**Key Files:**
- `crates/treb-cli/src/commands/verify.rs` (`VerifyOutputJson` struct, JSON output paths)

---

### P6-US-007: Golden File Updates for Verify Command

**Description:** Update existing verify golden files and add new ones covering multi-verifier scenarios, per-verifier flags, PARTIAL status, and styled output. Regenerate all verify golden files with `UPDATE_GOLDEN=1`.

**Acceptance Criteria:**
- Existing golden files updated: `verify_already_verified`, `verify_all_none_unverified`, `verify_json_already_verified`, `verify_error_unknown_verifier`, `verify_uninitialized`, `verify_no_foundry_project`
- New golden files added for:
  - Multi-verifier flags (e.g., `--etherscan --sourcify`)
  - Per-verifier shorthand flag (e.g., `--blockscout`)
  - JSON output with multi-verifier results
  - Batch verification summary format (if testable without live API)
- All golden file tests pass: `cargo test -p treb-cli -- verify`
- Integration test fixtures use `make_verified_deployment()` helpers from existing test infrastructure
- No pre-existing golden file failures introduced
- Typecheck passes

**Key Files:**
- `crates/treb-cli/tests/integration_verify.rs` (test definitions)
- `crates/treb-cli/tests/golden/verify_*/` (golden file directories)

## Functional Requirements

- **FR-1:** The verify command accepts `--etherscan`, `--blockscout`, `--sourcify` boolean flags for per-verifier selection.
- **FR-2:** Multiple verifier flags can be combined to verify against multiple explorers in one invocation.
- **FR-3:** When no shorthand verifier flags are set, `--verifier` (default: etherscan) determines the single verifier.
- **FR-4:** Shorthand flags override `--verifier` when both are specified.
- **FR-5:** API keys resolve from environment variables (`ETHERSCAN_API_KEY`, `BLOCKSCOUT_API_KEY`) when `--verifier-api-key` is not provided.
- **FR-6:** Explicit `--verifier-api-key` takes precedence over environment variables for all verifiers.
- **FR-7:** Multi-verifier results produce aggregate VerificationStatus: VERIFIED (all pass), FAILED (all fail), PARTIAL (mixed).
- **FR-8:** Registry is updated with all per-verifier results using lowercase keys (etherscan, sourcify, blockscout) matching the keys expected by `show` command.
- **FR-9:** Human-readable output uses Phase 3 formatting: stage indicators, styled status colors, verification badges.
- **FR-10:** JSON output includes per-verifier breakdown in a `verifiers` object.
- **FR-11:** `--json` mode suppresses all non-JSON output (no stderr progress in JSON mode).
- **FR-12:** Rate limiting delay (`--delay`) applies between verifier attempts within a deployment and between deployments in batch mode.

## Non-Goals

- **Block explorer API client implementation**: The `forge-verify` crate handles all API communication (submission, polling, standard JSON input). This phase does not implement raw HTTP clients for Etherscan/Blockscout/Sourcify.
- **Config file storage for API keys**: API keys come from CLI flags or environment variables only. No `treb.toml` verifier config section in this phase.
- **Parallel verification**: Multiple verifiers run sequentially per deployment. Parallel verification across verifiers is not in scope.
- **Custom verifier plugins**: Only the three built-in verifiers (etherscan, sourcify, blockscout) are supported.
- **Verification as part of `treb run --verify`**: The `--verify` flag on `run` delegates to forge's built-in verification during script execution. This phase only covers the standalone `treb verify` command.
- **Interactive verifier selection**: No interactive prompt for choosing verifiers. Selection is via CLI flags only.

## Technical Considerations

### Dependencies
- **Phase 3 (Output Formatting Framework)**: Completed. Provides `TreeNode`, color palette, `print_stage()`, `verification_badge()`, `styled()` pattern, `terminal_width()`.
- **Phase 4 (list/show)**: Completed. `show` command already displays per-verifier verification detail in a format the verify command should match. `VERIFIER_DISPLAY_ORDER` constant and `styled_verification_status()` helper exist in `show.rs` and may be extracted to a shared location.
- **forge-verify crate**: Already integrated in `treb-verify`. Handles all block explorer API interaction. No changes needed to treb-verify for multi-verifier support — the loop happens at the command layer.

### Integration Points
- **Registry**: `VerificationInfo.verifiers` HashMap already supports multiple verifier entries. `VerificationStatus::Partial` enum variant already exists. No schema changes needed.
- **show command**: Reads `verifiers` HashMap with keys "etherscan", "sourcify", "blockscout" — verify command must write matching lowercase keys.
- **badge module**: `verification_badge()` / `verification_badge_styled()` already handle multi-verifier display with fixed `VERIFIER_ORDER`.

### Constraints
- Verification requires a live Foundry project with compiled artifacts — golden file tests for actual verification are limited to skip/error paths unless mocked.
- `forge-verify`'s `VerifyArgs::run()` is async and makes real HTTP calls — integration tests should focus on CLI flag parsing, output formatting, and registry updates rather than live API calls.
- `styled()` pattern must gate on `color::is_color_enabled()`, not `owo_colors::set_override` (per Phase 4 learnings).
- All JSON output must use `output::print_json()` which calls `sort_json_keys()` for deterministic key ordering.
