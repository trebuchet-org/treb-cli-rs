# PRD: Phase 6 - run Command Output

## Introduction

This phase ports the `treb run` command's human-readable output from the Go CLI format to the Rust CLI, achieving exact visual parity. The Go version uses a `ScriptRenderer` with structured sections: a bold transactions header with per-transaction status/sender/decoded-call lines, a deployment summary with colored contract names and addresses, console logs with a section header, registry update confirmations, and a final success message. The current Rust output uses tree-grouped deployments, numbered transaction lists, and a summary line — all of which must be replaced with Go's section-based format.

This phase depends on Phase 2 (tree renderer and table formatter alignment) and builds on the shared color palette, emoji constants, and formatting helpers established in Phase 1.

## Goals

1. **Section structure parity**: The run command output must follow Go's exact section ordering: Transactions → Collisions → Deployment Summary → Script Logs → Registry Update → Success message.
2. **Transaction display parity**: Each transaction must show a colored status string, green sender name, and decoded operation info (using available data from `PipelineResult`), with a gray footer line showing hash/block/gas details.
3. **Deployment summary parity**: Replace the Rust tree-grouped deployment display with Go's flat `"{name} at {address}"` format using cyan/green coloring, including proxy and label formatting.
4. **Pre-execution banner parity**: The verbose mode pre-execution output must match Go's `PrintDeploymentBanner` format with emoji header, dash separators, and colored indented fields.
5. **Golden file updates**: All 14 run-related golden test directories must produce passing output after the format changes.

## Functional Requirements

- **FR-1**: Transaction section header must be `"\n🔄 Transactions:"` in bold, followed by a 50-character `─` separator in gray.
- **FR-2**: Each transaction must display as `"\n{status} {sender} → {decoded_call}\n"` where status is colored by type (simulated=white/faint, queued=yellow, executed=green, failed=red), sender is green, and decoded call shows operation info.
- **FR-3**: Transaction footer must show pipe-separated details in gray: `"Tx: {hash} | Block: {block} | Gas: {gas}"` (fields shown only when non-empty/non-zero).
- **FR-4**: Collision section (when collisions exist) must show `"\n⚠️  Deployment Collisions Detected:"` in yellow + 50-char separator + per-collision `"{contractName} already deployed at {address}"` with optional Label/Entropy lines + gray note.
- **FR-5**: Deployment summary (when deployments exist) must show `"\n📦 Deployment Summary:"` in bold + 50-char separator + per-deployment `"{name} at {address}"` with name in cyan and address in green. Proxy format: `"{name}[{impl}]"`. Label format: `"{name}:{label}"`.
- **FR-6**: Console logs section (when logs exist) must show `"\n📝 Script Logs:"` in bold + 40-char `─` separator in gray + each log indented with 2 spaces.
- **FR-7**: Registry update message (when not dry-run and success): `"✓ Updated registry for {network} network in namespace {namespace}"` in green when changes exist; `"- No registry changes recorded for {network} network in namespace {namespace}"` in yellow otherwise.
- **FR-8**: Final success message must be `"✓ Script execution completed successfully"` in green (replacing the current `print_stage("✅", "Execution complete.")`).
- **FR-9**: Pre-execution banner (verbose, non-JSON) must show: `"🚀 Running Deployment Script"` (bold) + 50-char separator + indented fields (Script cyan, Network blue with gray chain ID, Namespace magenta, Mode colored by type, Env Vars, Senders) + 50-char separator.
- **FR-10**: DRY RUN indicator: no separate banner in the Go output — dry-run state is conveyed through the Mode field in the pre-execution banner and through the absence of registry update messages.
- **FR-11**: The summary line (`"1 deployment recorded, 1 transaction, 160,053 gas used"`) must be removed — Go does not produce this line.
- **FR-12**: Governor proposals display must be retained and formatted consistently with Go's transaction rendering style.

## User Stories

### P6-US-001: Restructure display_result_human Section Ordering and Add Section Headers

**Description:** Restructure `display_result_human()` in `run.rs` to follow Go's `RenderExecution` output order. Add Go-style bold section headers with gray 50-character dash (`─`) separators. Remove the summary line. This story establishes the new skeleton that subsequent stories fill in.

**Go reference:** `render/script.go:53-91` (RenderExecution flow)

**Changes:**
- `crates/treb-cli/src/commands/run.rs` — rewrite `display_result_human()` to call sections in Go order: transactions → collisions → deployment summary → logs → registry update
- Add helper function `print_section_header(emoji, title, separator_width)` for consistent bold header + gray separator rendering
- Remove the summary line block (lines 847-886)
- Remove the DRY RUN banner at the top (Go doesn't show one in `RenderExecution`)

**Acceptance criteria:**
- `display_result_human()` calls sections in order: transactions, collisions, deployment summary, logs, registry update
- Each section uses `print_section_header()` for consistent formatting
- Summary line is removed
- DRY RUN banner is removed from `display_result_human` (dry-run state conveyed via pre-execution banner Mode field and registry update absence)
- `cargo clippy --workspace --all-targets` passes
- Typecheck passes (`cargo check -p treb-cli`)

---

### P6-US-002: Port Transaction Section with Status Colors and Operation Info

**Description:** Replace the current numbered transaction list (`"1. {hash} (STATUS)"`) with Go's format: colored status string + green sender name + operation description, followed by a gray footer with hash/block/gas details.

**Go reference:** `render/transaction.go:96-168` (status coloring, header format), `render/transaction.go:441-468` (footer)

**Changes:**
- `crates/treb-cli/src/commands/run.rs` — rewrite the transaction rendering block
- Status strings: `"simulated"` (white/faint), `"queued   "` (yellow), `"executed "` (green), `"failed   "` (red), `"unknown  "` (white) — note fixed-width padding with trailing spaces
- Format: `"\n{status} {sender} → {decoded_call}\n"` where sender is green and decoded call comes from `transaction.operations`
- Footer: `"   {details}\n"` in gray with pipe-separated `"Tx: {hash} | Block: {block} | Gas: {gas}"` (only show non-empty fields)
- When no transactions: `"No transactions executed (dry run or all deployments skipped)"` in gray

**Acceptance criteria:**
- Transaction status is colored per Go's palette (simulated=faint, queued=yellow, executed=green, failed=red)
- Sender name is displayed in green
- Operations from `RecordedTransaction.transaction.operations` are used to show call info (e.g., type and target)
- Footer shows hash, block number, and gas in gray, pipe-separated, only when non-empty
- Empty transaction list shows gray fallback message
- Typecheck passes

---

### P6-US-003: Port Deployment Summary and Collision Sections

**Description:** Replace the Rust tree-grouped deployment display (TreeNode by namespace > chain > type) with Go's flat deployment summary format. Add the collision section with per-collision details.

**Go reference:** `render/script.go:141-197` (deployment summary + collisions)

**Changes:**
- `crates/treb-cli/src/commands/run.rs` — replace tree rendering with flat list
- Remove `group_recorded_deployments()`, `RunTypeGroup`, `type_sort_key()`, and `build_run_deployment_node()` if no longer needed
- Collision section: `"\n⚠️  Deployment Collisions Detected:"` (yellow) + 50-char separator + per-collision: `"{contractName} already deployed at {address}"` (cyan name, yellow address) + optional `"    Label: {label}"` and `"    Entropy: {entropy}"` lines + gray note
- Deployment summary: `"\n📦 Deployment Summary:"` (bold) + 50-char separator + per-deployment: `"{name} at {address}"` (cyan name, green address)
- Name formatting: append `":{label}"` if label exists; append `"[{impl_name}]"` for proxy deployments (implementation name from linked deployment or "UnknownImplementation")

**Acceptance criteria:**
- Deployment summary shows flat list, not tree
- Contract names are cyan, addresses are green
- Label appended as `":{label}"` format
- Proxy deployments show `"[{impl}]"` suffix
- Collisions show detailed per-collision output with yellow warning header
- Collision label and entropy shown only when non-empty
- Gray note line appears after collisions
- Empty deployments section is omitted entirely
- Typecheck passes

---

### P6-US-004: Port Console Logs, Registry Update, and Success Message

**Description:** Port the console logs section with proper header, the registry update confirmation message, and replace the final success message with Go's format.

**Go reference:** `render/script.go:200-218` (logs), `render/script.go:76-87` (registry update), `run.go:116` (success line)

**Changes:**
- `crates/treb-cli/src/commands/run.rs`:
  - Console logs: `"\n📝 Script Logs:"` (bold) + 40-char `─` separator in gray + `"  {log}"` per line (2-space indent) + blank line
  - Registry update (non-dry-run, success): `"✓ Updated registry for {network} network in namespace {namespace}"` in green when deployments exist and changes were made; `"- No registry changes recorded for {network} network in namespace {namespace}"` in yellow when no deployments
  - Final success: Replace `print_stage("✅", "Execution complete.")` with `"✓ Script execution completed successfully"` in green
- Note: The registry update message needs network name and namespace. These must be passed to `display_result_human` or extracted from PipelineResult context. If not currently available, add the necessary fields (network name, namespace) to PipelineResult or pass as separate parameters.

**Acceptance criteria:**
- Console logs section has bold emoji header + 40-char gray separator + 2-space indented lines
- Console logs section only appears when logs exist
- Registry update message shows in green when successful with changes
- Registry update message shows in yellow when no changes
- Registry update only appears when not dry-run and execution succeeded
- Final success message reads `"✓ Script execution completed successfully"` in green
- The stage messages for compiling/broadcasting remain on stderr (they are printed before execution, not in `display_result_human`)
- Typecheck passes

---

### P6-US-005: Port Pre-Execution Deployment Banner (Verbose Mode)

**Description:** Replace the current verbose mode `eprint_kv()` pre-execution output with Go's `PrintDeploymentBanner` format, which uses an emoji header, dash separators, and per-field coloring.

**Go reference:** `render/script.go:221-263` (PrintDeploymentBanner)

**Changes:**
- `crates/treb-cli/src/commands/run.rs` — replace the verbose pre-execution block (lines 323-350) with:
  - Blank line
  - `"🚀 Running Deployment Script\n"` in bold
  - 50-char `─` separator in gray
  - `"  Script:    {name}"` in cyan
  - `"  Network:   {network} ({chainID})"` — network in blue, chain ID in gray
  - `"  Namespace: {namespace}"` in magenta
  - `"  Mode:      {mode}"` — DRY_RUN yellow, FORK magenta, LIVE green
  - `"  Env Vars:  {KEY}={VALUE}\n"` — key in yellow, value in green (first on same line, subsequent indented to align)
  - `"  Senders:   {senders}"` in gray
  - 50-char `─` separator in gray
- Also update verbose post-execution output to match Go format (or remove if Go doesn't have equivalent)

**Acceptance criteria:**
- Pre-execution banner matches Go's `PrintDeploymentBanner` format exactly
- Script name is cyan
- Network name is blue, chain ID in gray parentheses
- Namespace is magenta
- Mode is colored by type (DRY_RUN=yellow, FORK=magenta, LIVE=green)
- Env vars show key=value with proper colors and alignment
- Senders shown in gray
- 50-char dash separators bookend the field list
- Output goes to stdout (Go uses the renderer's writer, not stderr)
- Typecheck passes

---

### P6-US-006: Port Governor Proposals Display and Debug Output Format

**Description:** Update the governor proposals section to be consistent with Go's transaction rendering style. Update the debug log file format for consistency.

**Go reference:** `run.go:100-117` (command handler flow — governor is handled via standard transaction rendering in Go)

**Changes:**
- `crates/treb-cli/src/commands/run.rs`:
  - Governor proposals: Update formatting to use Go-style section header (`"🏛️  Governor Proposals:"` bold + separator) and per-proposal details with colored status
  - Skipped deployments: Update to use Go-style section format if Go has equivalent (Go handles skipped via collisions, not a separate section)
  - Debug output: Update debug log file to match Go's debug format if different, or keep as-is if Go doesn't define a specific debug log format
  - Remove `format_governor_proposal_details()` if replaced by inline formatting

**Acceptance criteria:**
- Governor proposals section has a bold emoji header with separator (consistent with other sections)
- Per-proposal shows proposal ID, governor/timelock addresses, status, and linked transaction count
- Skipped deployments section either matches Go format or is removed if Go handles skipping through collisions only
- Debug log file writes successfully with updated format
- Typecheck passes

---

### P6-US-007: Update All Golden Test Expected Files

**Description:** Regenerate all run-related golden test expected files to match the new Go-parity output format. Verify all tests pass.

**Changes:**
- Run `UPDATE_GOLDEN=1 cargo test -p treb-cli --test integration_run` to regenerate golden files for all 14 run test directories
- Manually verify the diff of each `.golden` file to confirm the new output matches Go format expectations
- Run the full test suite to ensure no regressions

**Acceptance criteria:**
- All 14 run golden test directories produce passing tests
- Golden file diffs show the expected format changes (section headers, separators, colored fields, removed summary line)
- `cargo test -p treb-cli --test integration_run` passes without `UPDATE_GOLDEN`
- `cargo test -p treb-cli --test cli_run` passes (argument parsing tests unaffected)
- `cargo clippy --workspace --all-targets` passes
- No regressions in other test suites: `cargo test -p treb-cli` passes fully

## Non-Goals

- **Transaction trace tree rendering**: The Go version renders full decoded trace trees with nested calls, events, and CREATE handling via `TransactionRenderer.displayTraceTree()`. The Rust `PipelineResult` does not currently carry forge trace data (only registry `Transaction` objects with `operations`). Implementing trace tree rendering would require forge pipeline changes beyond this display-formatting phase. The per-transaction display will use available operation data and show a format consistent with Go's structure.
- **ABI decoding of function calls**: The Go version uses `abi.TransactionDecoder` to decode function selectors into human-readable `Contract::method(args)` format. This requires an ABI resolver infrastructure not present in the Rust pipeline output. Decoded call display will use operation metadata available in `RecordedTransaction`.
- **JSON output changes**: The `--json` output schema is not changed in this phase. JSON schema parity is handled in Phase 14.
- **Forge pipeline data model changes**: No modifications to `PipelineResult`, `RecordedTransaction`, or other pipeline types to add trace data, decoded calls, or network name fields. If registry update messages need network/namespace context, these are passed as parameters rather than modifying the pipeline result type.

## Technical Considerations

### Dependencies
- **Phase 1** (completed): Color palette (`color::*`), emoji constants (`emoji::*`), format helpers (`format_success`, `format_warning`)
- **Phase 2** (completed): Table rendering (`render_table_with_widths`, `calculate_column_widths`) — may not be needed for this phase since Go's run output uses simple formatted lines rather than tables

### Data Availability
- `PipelineResult.transactions` contains `RecordedTransaction` wrapping registry `Transaction` objects. These have `sender`, `hash`, `status`, `block_number`, `operations` (Vec<Operation>), and `safe_context`, but NOT decoded function signatures or trace data.
- `PipelineResult.collisions` contains `ExtractedCollision` with `existing_address`, `contract_name`, `label`, `strategy`, `salt`, `bytecode_hash`, `init_code_hash` — sufficient for Go's collision display.
- `PipelineResult.deployments` contains `RecordedDeployment` wrapping `Deployment` domain objects — has all fields needed for Go's deployment summary.
- Network name and namespace are available in the command handler context but not in `PipelineResult` — pass as parameters to display functions.

### Key Patterns from Previous Phases
- `styled(text, style)` helper for conditional coloring based on `color::is_color_enabled()` — used in Phase 5 show command
- `emoji::*` constants for all Go emoji — established in Phase 1
- `color::STAGE` (cyan bold) for section headers, `color::GRAY` for separators and secondary info
- `color::SUCCESS` (green bold) for success messages
- Section-based output with `print_section()` / `print_field()` helpers — pattern from Phase 5

### Golden Test Infrastructure
- 14 golden test directories under `crates/treb-cli/tests/golden/run_*`
- Tests in `crates/treb-cli/tests/integration_run.rs` (golden file comparison) and `crates/treb-cli/tests/cli_run.rs` (argument parsing)
- Normalizers strip ANSI codes, timestamps, hashes, paths — output changes only affect the structural format
- Target with `--test integration_run` to avoid updating unrelated golden files
- The `commands.golden` files capture stdout; stderr output (stage messages, verbose context) may or may not be captured depending on test setup

### Removed Code
- `group_recorded_deployments()`, `RunTypeGroup`, `type_sort_key()`, `build_run_deployment_node()`, `format_run_deployment_entry()` — these implement the tree-grouped deployment display that is being replaced with Go's flat format. They can be removed if no other code depends on them.
- `format_governor_proposal_details()` — may be inlined or restructured
