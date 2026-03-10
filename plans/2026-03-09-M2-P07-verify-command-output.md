# PRD: Phase 7 - verify Command Output

## Introduction

Phase 7 ports the verification command output from the Go CLI (`render/verify.go`) to the Rust CLI so that human-readable output matches the Go version exactly. The Go verify command has two modes: single-deployment verification and batch (`--all`) verification. Batch mode shows skipped contracts (pending/undeployed), contracts to verify with status icons, per-result success/failure details, and a summary line. Single mode shows an already-verified yellow message or per-verifier progress with a success/failure result and verification status breakdown.

This phase depends only on Phase 1 (shared color palette, emoji constants, format helpers), which is already complete.

## Goals

1. **Single-deployment output matches Go exactly**: already-verified yellow message with `--force` suggestion, per-verifier result lines with status icons, and verification status details section.
2. **Batch (`--all`) output matches Go exactly**: skipped contracts section with cyan bold header and skip-emoji per line, contracts-to-verify count header, per-result status icons with location, success/failure details, and summary line.
3. **Status icons match Go mapping**: `🔄` (re-verifying/verified), `⚠️` (failed), `🔁` (partial), `⏳` (unverified/first attempt), `🆕` (default/new).
4. **All existing golden tests pass** after output format changes, with `.expected` files updated to reflect the new Go-matching format.
5. **No changes to JSON output** — `--json` schema and golden files remain unchanged.

## User Stories

### P7-US-001: Port Already-Verified Message and Single-Deployment Success Output

**Description:** Update the single-deployment verification output to match Go's `RenderVerifyResult`. The Go version shows:
- Already verified: yellow `Contract NAME is already verified. Use --force to re-verify.\n`
- Manual contract path override: yellow `Using manual contract path: PATH\n`
- Success: green `✓ Verification completed successfully!\n` followed by a `Verification Status:` section showing per-verifier results with title-cased verifier names, status icons (✓/✗/⏳), and URLs/reasons.
- Failure: red `✗ Verification failed: ERROR\n` per error.

Currently the Rust code uses `styled("\u{2713}", color::SUCCESS)` with custom formatting. Replace with Go-matching format strings.

**Go reference:** `render/verify.go` lines 97-134 (`RenderVerifyResult`), lines 170-197 (`showVerificationStatus`)

**Rust file to modify:** `crates/treb-cli/src/commands/verify.rs` — update the already-verified message (lines 216-221), the per-verifier result display (lines 297-317), and add the verification status section after single-deployment success.

**Acceptance Criteria:**
- Already-verified message uses yellow color and reads: `Contract {display_name} is already verified. Use --force to re-verify.\n` where `display_name` is `Name:label` or `Name`
- Successful single verification prints green `✓ Verification completed successfully!\n`
- After success, a `Verification Status:` section shows per-verifier results with title-cased names (e.g., `Etherscan`), green `✓ Verified` or red `✗ Failed` with URL/reason
- Failed verification prints red `✗ Verification failed: {error}\n` per error
- `cargo clippy --workspace --all-targets` passes
- Existing golden tests `verify_already_verified` and `verify_already_verified_multi_shorthand` updated to match new format

---

### P7-US-002: Add Status Icon Mapping and Display Name Helper

**Description:** Add the Go status icon mapping function (`getStatusIcon`) and ensure the `getDisplayName` equivalent exists. The Go version maps `VerificationStatus` to emoji icons used in batch mode output:
- `Verified` → `🔄` (re-verifying)
- `Failed` → `⚠️` (retrying failed)
- `Partial` → `🔁` (retrying partial)
- `Unverified` → `⏳` (first attempt)
- Default → `🆕` (new)

The display name is `ContractName:label` when label exists, or just `ContractName`.

**Go reference:** `render/verify.go` lines 136-158 (`getStatusIcon`, `getDisplayName`)

**Rust file to modify:** `crates/treb-cli/src/commands/verify.rs` — add `get_status_icon()` function and ensure display name uses `contract_display_name()` from `treb_core::types` (already available).

**Acceptance Criteria:**
- `get_status_icon(status: &VerificationStatus) -> &str` returns the correct emoji for each status variant
- Display name uses `contract_display_name(name, label)` from `treb_core::types` for all verify output (single and batch)
- `cargo clippy --workspace --all-targets` passes
- Unit tests cover all status icon mappings

---

### P7-US-003: Port Batch Verification Output (Skipped, To-Verify, Results, Summary)

**Description:** Update the batch (`--all`) verification output to match Go's `RenderVerifyAllResult`. The Go version has four sections:

1. **Skipped contracts** (cyan bold header): `Skipping N pending/undeployed contracts:\n` + per-skipped: `  ⏭️  chain:CHAINID/NS/DISPLAY_NAME (REASON)\n` + blank line.
2. **No contracts to verify**: yellow `No unverified deployed contracts found. Use --force to re-verify all contracts.\n` (without force) or `No deployed contracts found to verify.\n` (with force).
3. **Contracts to verify** (cyan bold header): `Found N unverified deployed contracts to verify:\n` (or with `--force`: `Found N deployed contracts to verify (including verified ones with --force):\n`).
4. **Per-result**: `  {status_icon} chain:CHAINID/NS/DISPLAY_NAME\n` + success: green `    ✓ Verification completed\n` or failure: red `    ✗ ERROR\n` per error. Blank line between items (not after last).
5. **Summary**: `\nVerification complete: X/Y successful\n`.

Currently the Rust batch mode uses `print_stage` with `[i/total]` progress counters and a comfy_table summary. Replace entirely with the Go format.

**Go reference:** `render/verify.go` lines 31-95 (`RenderVerifyAllResult`)

**Rust file to modify:** `crates/treb-cli/src/commands/verify.rs` — restructure `run_batch()` (lines 380-628) to collect skipped deployments, output the skipped section, to-verify header, per-result output with status icons, and summary line. Remove the comfy_table summary.

**Acceptance Criteria:**
- Skipped contracts section renders with cyan bold header and `⏭️` emoji per line in `chain:CHAINID/NS/NAME (REASON)` format
- Empty to-verify list shows yellow message with appropriate text based on `--force` flag
- To-verify header shows correct count with cyan bold, text varies with `--force`
- Per-result shows status icon (from P7-US-002), location in `chain:CHAINID/NS/NAME` format, success/failure details
- Blank line separates results except after last
- Summary line: `Verification complete: X/Y successful`
- `cargo clippy --workspace --all-targets` passes
- Golden test `verify_all_none_unverified` updated to match new format

**Note:** The current Rust batch mode doesn't have a "skipped" concept since it filters candidates differently than Go. If the Rust code doesn't separate skipped vs. to-verify deployments, the skipped section can be omitted (matching Go behavior when `len(result.Skipped) == 0`). The key deliverable is matching Go's output format for the sections that do appear.

---

### P7-US-004: Port Single-Deployment Verification Progress Output

**Description:** Update the in-progress verification output for single deployments. Currently the Rust code uses `print_stage` with a magnifying glass emoji and shows per-verifier results as `  verifier: VERIFIED/FAILED` with badge summary. Replace with Go-matching format:

- Before verification: no header needed (Go uses spinner which we skip in non-interactive)
- Per-verifier success: keep the current per-verifier progress lines but ensure they match Go's verification status section format
- Remove the badge summary line (`ver_badge`) — Go doesn't show badges, it shows the `Verification Status:` section instead (from P7-US-001)

**Go reference:** `render/verify.go` lines 108-134 (`RenderVerifyResult` success/failure paths)

**Rust file to modify:** `crates/treb-cli/src/commands/verify.rs` — update the single-deployment verification loop output (lines 229-347) to remove `print_stage` header, remove badge summary, and use Go-matching per-verifier output format.

**Acceptance Criteria:**
- Single verification no longer prints magnifying glass `print_stage` header
- Per-verifier results show in `Verification Status:` section format (title-cased verifier name, ✓/✗ icon, URL/reason)
- Badge summary line removed from single-deployment output
- `cargo clippy --workspace --all-targets` passes

---

### P7-US-005: Update All Golden Test Expected Files

**Description:** Regenerate all verify golden test expected files to match the new Go-matching output format. Run the golden tests with `UPDATE_GOLDEN=1` targeting only verify tests, then review the diff to ensure all changes are intentional.

**Rust file to modify:** Golden `.expected` files under `crates/treb-cli/tests/golden/verify_*/`

**Acceptance Criteria:**
- All verify golden tests pass: `cargo test -p treb-cli --test cli_verify`
- All verify integration tests pass: `cargo test -p treb-cli --test integration_verify`
- Full test suite passes: `cargo test -p treb-cli`
- `cargo clippy --workspace --all-targets` passes
- No non-verify golden files are modified
- Git diff reviewed: all changes are intentional format updates matching Go output

## Functional Requirements

- **FR-1:** Already-verified message must be yellow and read `Contract {display_name} is already verified. Use --force to re-verify.\n` matching Go exactly.
- **FR-2:** Status icon mapping must match Go: Verified→🔄, Failed→⚠️, Partial→🔁, Unverified→⏳, default→🆕.
- **FR-3:** Display name must use `ContractName:label` format when label exists, plain `ContractName` otherwise.
- **FR-4:** Batch skipped section must use cyan bold header `Skipping N pending/undeployed contracts:\n` with `⏭️` per line.
- **FR-5:** Batch to-verify header must use cyan bold with count, varying text based on `--force` flag.
- **FR-6:** Batch per-result must show status icon + `chain:CHAINID/NS/NAME` location + success green `✓ Verification completed` or failure red `✗ ERROR`.
- **FR-7:** Batch summary must read `Verification complete: X/Y successful\n`.
- **FR-8:** Single success must print green `✓ Verification completed successfully!\n` followed by `Verification Status:` section with per-verifier details.
- **FR-9:** Single failure must print red `✗ Verification failed: {error}\n`.
- **FR-10:** All output must respect `NO_COLOR`/`TERM=dumb`/`--no-color` (use `color::is_color_enabled()` and existing `styled()` helper).
- **FR-11:** JSON output (`--json`) must remain completely unchanged.

## Non-Goals

- **No changes to verification logic** — only display output is being ported, not the verification orchestration, API key resolution, or registry update logic.
- **No changes to JSON output schema** — `--json` output is already correct and will not be modified.
- **No spinner implementation** — Go uses a spinner for interactive single verification; the Rust CLI does not implement spinners and this phase does not add one.
- **No changes to `--all` candidate filtering** — the Rust code filters candidates differently than Go (no separate skipped/to-verify split at the use-case layer). The display format will match Go but the filtering logic stays as-is.
- **No changes to command flags or clap definitions** — all verify command flags are already implemented.
- **No changes to verification badge module** (`ui/badge.rs`) — badges are used by the list command; this phase removes badge usage from verify output.

## Technical Considerations

### Dependencies
- **Phase 1 (complete):** Emoji constants in `ui/emoji.rs` (FAST_FORWARD/⏭️, REFRESH/🔄, WARNING/⚠️, REPEAT/🔁, HOURGLASS/⏳, NEW/🆕), color styles (`color::STAGE` for cyan bold headers, `color::WARNING` for yellow, `color::SUCCESS`/green, `color::FAILED`/red), and `styled()` conditional coloring pattern.

### Key Patterns from Previous Phases
- Use `styled(text, style)` pattern for conditional coloring (already defined locally in verify.rs).
- Use `contract_display_name(name, label)` from `treb_core::types` for `Name:label` display format.
- Output to stderr via `eprintln!` for human output (consistent with current verify.rs behavior).
- Color constants: `color::VERIFIED` (green), `color::FAILED` (red), `color::MUTED` (dimmed), `color::STAGE` (cyan bold), `color::WARNING` (yellow).

### Differences Between Go and Rust Architecture
- Go's `VerifyAllResult` separates `Skipped` and `ToVerify` at the use-case layer. Rust's `run_batch()` filters candidates inline. The skipped section display may be empty if no deployments are skipped, which is valid (Go handles `len(result.Skipped) == 0` by simply not printing the section).
- Go renders output through a `VerifyRenderer` struct. Rust will keep inline formatting in `verify.rs` (consistent with the pattern used in other ported commands like show.rs and list.rs).
- Go's per-verifier output uses `cases.Title(language.English).String(verifier)` for title-casing. Rust should capitalize the first letter of verifier names (e.g., "etherscan" → "Etherscan").

### Testing
- Target verify golden tests with: `cargo test -p treb-cli --test cli_verify`
- Target verify integration tests with: `cargo test -p treb-cli --test integration_verify`
- Regenerate golden files with: `UPDATE_GOLDEN=1 cargo test -p treb-cli --test cli_verify`
- Always verify the git diff after golden file regeneration to confirm only expected changes.
