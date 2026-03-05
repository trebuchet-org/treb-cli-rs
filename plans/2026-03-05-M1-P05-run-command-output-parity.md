# PRD: Phase 5 - run Command Output Parity

## Introduction

This phase transforms the `treb run` command's output from its current flat table format into the Go CLI's staged execution display with emoji progress indicators, tree-style deployment summaries, gas/transaction detail, and enhanced debug/verbose/dry-run output modes. The Go `run` command shows a clear progression through stages (compiling, executing, broadcasting, recording) with deployment event summaries and transaction details. Phase 3 built the reusable UI framework (TreeNode, color palette, badges, terminal utilities) and Phase 4 established patterns for styled output (`styled()`, `print_header()`, `print_kv()`). This phase applies those patterns to the run command and adds the `--dump-command` flag for debugging in-process forge execution.

## Goals

1. **Staged execution display**: Show clear progress through pipeline stages (compiling, executing, broadcasting, recording) with emoji markers and styled stage labels, replacing the current bare `eprintln!` messages.

2. **Tree-style deployment summary**: Replace the `comfy_table` deployment table with a tree-style summary matching the list command's hierarchical format (namespace > chain > type > deployment), reusing `build_deployment_node` patterns from Phase 4.

3. **Transaction and gas detail**: Display per-transaction hash and gas usage information from `PipelineResult`, giving users visibility into execution costs.

4. **Enhanced mode outputs**: Improve `--debug` (full forge output, save to debug file), `--verbose` (event parsing detail, registry write confirmation), and `--dry-run` (styled banner, "would be" language) to match Go CLI behavior.

5. **Dump command flag**: Add `--dump-command` flag that prints the equivalent `forge script` CLI command, enabling users to debug differences between in-process and subprocess execution.

## User Stories

### P5-US-001: Stage Progress Display Functions

**Description:** Create reusable stage display functions that show execution progress with emoji markers and styled labels. These will be called at each pipeline stage transition point in the run command.

**Details:**
- Add a `run_display` module (or section within `run.rs`) with stage display functions:
  - `print_stage(emoji: &str, message: &str)` — prints `{emoji} {message}` with message styled using `color::STAGE`
  - Predefined stages: compiling ("Compiling..."), executing ("Executing..."), broadcasting ("Broadcasting..."), recording ("Recording...")
- Use emoji markers matching Go CLI conventions (e.g., gear for compiling, rocket for executing, radio for broadcasting, floppy for recording)
- Respect `color::is_color_enabled()` — print plain text when color is disabled
- All stage output goes to stderr (not stdout) to keep stdout clean for result data
- Replace current `eprintln!("Compiling and executing {}...", script)` and `eprintln!("Execution complete.")` with staged display calls

**Acceptance Criteria:**
- `print_stage()` outputs emoji + styled message to stderr when color is enabled
- `print_stage()` outputs emoji + plain message to stderr when color is disabled
- `cargo check -p treb-cli` passes
- Unit test verifying `print_stage` produces expected format string

---

### P5-US-002: Deployment Summary Tree Rendering

**Description:** Replace the `comfy_table` deployment table in `display_result_human()` with tree-style rendering that groups deployments by namespace > chain > type, reusing the patterns established in the list command.

**Details:**
- Extract deployment data from `PipelineResult.deployments` (each `RecordedDeployment` contains a `Deployment`)
- Reuse or adapt `group_deployments()` from `list.rs` to group the result deployments
- Build `TreeNode` hierarchy: namespace > chain_id > deployment type > deployment entry
- Each deployment entry shows: `ContractName[:label] address type`
- For proxy deployments with `proxy_info`, add implementation child node (following `build_deployment_node` pattern)
- Use `render()` for plain output, `render_styled()` for colored output (gated on `is_color_enabled()`)
- Keep the skipped deployments display (bulleted list format)
- Remove the `comfy_table` import from `run.rs` if it was the last user

**Acceptance Criteria:**
- Run output shows tree-style deployment summary instead of table
- Deployments are grouped by namespace > chain > type
- Proxy deployments show implementation child node
- Skipped deployments still display with reason
- `cargo check -p treb-cli` passes

---

### P5-US-003: Transaction and Gas Usage Display

**Description:** Add a transaction detail section to the run output showing per-transaction hash and gas usage, giving users visibility into execution costs.

**Details:**
- After the deployment summary tree, add a "Transactions" section using `print_header("Transactions")`
- For each `RecordedTransaction` in the result, display:
  - Transaction ID (from `transaction.id`)
  - Transaction hash (from `transaction.hash`) — show truncated if present, "pending" if empty
  - Status (from `transaction.status`)
- Display total gas used if available (from `ExecutionResult.gas_used` — this needs to be threaded through `PipelineResult`)
- Add a `gas_used: u64` field to `PipelineResult` to carry gas information from `ExecutionResult`
- Update the summary line to include gas: `"1 deployment recorded, 1 transaction (gas: 123,456)"`
- Gas display uses comma-separated formatting for readability

**Acceptance Criteria:**
- Transaction section displays after deployment tree with transaction ID, hash, and status
- Gas usage appears in summary line when non-zero
- `PipelineResult` now includes `gas_used` field
- `cargo check -p treb-cli` and `cargo check -p treb-forge` pass

---

### P5-US-004: Dry-Run Output Styling

**Description:** Enhance the dry-run output with styled banner, clear "would be" language throughout, and a consistent visual treatment that distinguishes simulation from real execution.

**Details:**
- Replace plain `"[DRY RUN] No changes were written to the registry."` with a styled banner:
  - `print_stage()` with a distinct dry-run emoji and `color::WARNING` style
  - Banner text: `"[DRY RUN] Simulating — no transactions will be broadcast, no registry changes will be written."`
- Stage display in dry-run mode should use "Simulating..." instead of "Broadcasting..." for the broadcast stage
- Summary line uses "would be recorded" (already exists) and "would use" for gas
- Skipped deployments in dry-run use softer "would be skipped" language
- Dry-run JSON output (`RunOutputJson`) is unchanged

**Acceptance Criteria:**
- Dry-run banner is styled with warning color and dry-run emoji
- No "Broadcasting..." stage appears in dry-run mode
- Summary uses "would be" language
- `--json` dry-run output is unchanged
- `cargo check -p treb-cli` passes

---

### P5-US-005: Verbose Mode Enhancement

**Description:** Enhance `--verbose` mode to show additional execution context before and after the pipeline run, including resolved configuration, event parsing detail, and registry write confirmations.

**Details:**
- **Pre-execution context** (already partially exists, enhance):
  - Config source, namespace, network/RPC, sender address (existing)
  - Add: chain ID, script path, function signature, broadcast mode
  - Format using `print_kv()` for aligned display instead of raw `eprintln!`
- **Post-execution detail**:
  - Event count summary: "Decoded N events: X deployments, Y transactions, Z collisions"
  - Per-deployment registry write confirmation: "Recorded: ContractName at 0x..."
  - Console log count (existing, keep)
- All verbose output goes to stderr
- Verbose output is suppressed when `--json` is active (existing behavior, keep)

**Acceptance Criteria:**
- Verbose pre-execution shows aligned key-value context using `print_kv` pattern
- Verbose post-execution shows event count and per-deployment confirmations
- `--verbose --json` suppresses verbose output (json-only on stdout)
- `cargo check -p treb-cli` passes

---

### P5-US-006: Debug Mode and Dump Command Flag

**Description:** Implement `--debug` mode to show full forge execution output and save it to a debug file, and add the `--dump-command` flag that prints the equivalent `forge script` CLI command.

**Details:**
- **Debug mode (`--debug`)**:
  - After execution, dump the full execution traces/logs to stderr
  - Save debug output to `.treb/debug-<timestamp>.log` file
  - Print the debug file path: "Debug output saved to .treb/debug-<timestamp>.log"
  - Include: execution traces, console logs, raw event count, gas details
- **Dump command (`--dump-command`)**:
  - Add `--dump-command` CLI flag (boolean) to the run command's clap definition
  - Before pipeline execution, construct the equivalent `forge script` CLI command string from `ScriptConfig` fields
  - Print the command to stderr and exit (do not execute the pipeline)
  - Format: `forge script <path> --sig "<sig>" --rpc-url <url> --sender <addr> [--broadcast] [--slow] [--legacy] [--verify]`
  - This is purely for debugging — helps users compare in-process vs subprocess behavior
- Wire the `--dump-command` flag through to the run function parameter list

**Acceptance Criteria:**
- `--debug` saves full output to `.treb/debug-<timestamp>.log` and prints file path
- `--dump-command` prints equivalent forge command and exits without executing
- `--dump-command` includes all relevant flags from ScriptConfig
- `cargo check -p treb-cli` passes

---

### P5-US-007: Update Golden Files for Run Command

**Description:** Regenerate all existing run golden files to match the new output format and add new test cases for the enhanced modes.

**Details:**
- Regenerate existing golden files:
  - `run_basic/commands.golden` — tree output instead of table, stage indicators, gas summary
  - `run_dry_run/commands.golden` — styled dry-run banner, "would be" language, no broadcast stage
  - `run_verbose/commands.golden` — enhanced pre/post verbose context with `print_kv` format
  - `run_debug/commands.golden` — debug file path message
  - `run_basic_json/commands.golden` — JSON schema may gain `gas_used` field
- Registry artifact golden files (`deployments.golden`, `transactions.golden`) should be unaffected unless `PipelineResult` schema changes affect serialization
- Add new golden test: `run_dump_command` — verifies `--dump-command` prints forge command and exits
- Add normalizers as needed: `DebugFileNormalizer` for the timestamp in debug file paths
- Run `UPDATE_GOLDEN=1 cargo test -p treb-cli -- run` to auto-regenerate all run golden files
- Verify all tests pass: `cargo test -p treb-cli -- run`

**Acceptance Criteria:**
- All existing run golden file tests pass with updated output format
- New `run_dump_command` golden test validates forge command output
- `run_basic_json` golden file reflects any JSON schema changes (e.g., `gas_used`)
- No regressions in non-run golden tests (list, show, etc.)
- Full test suite passes: `cargo test -p treb-cli`

## Functional Requirements

- **FR-1:** `treb run` displays execution progress through stages (compiling, executing, broadcasting, recording) with emoji markers and styled labels on stderr.
- **FR-2:** Deployment results are displayed as a tree hierarchy (namespace > chain > type > deployment) instead of a flat table, consistent with the list command format.
- **FR-3:** Transaction details (ID, hash, status) are displayed after the deployment summary.
- **FR-4:** Gas usage is shown in the summary line when available.
- **FR-5:** `--dry-run` mode shows a styled warning banner and uses "would be" language throughout; the "broadcasting" stage is replaced with "simulating."
- **FR-6:** `--verbose` mode shows aligned pre-execution context (config, namespace, chain, RPC, sender, script, sig) and post-execution detail (event counts, registry write confirmations).
- **FR-7:** `--debug` mode saves full execution output to `.treb/debug-<timestamp>.log` and prints the file path.
- **FR-8:** `--dump-command` prints the equivalent `forge script` CLI command to stderr and exits without executing.
- **FR-9:** `--json` output on stdout remains the sole output in JSON mode — all stage/verbose/debug output on stderr is suppressed when `--json` is active.
- **FR-10:** All stage and styled output respects `NO_COLOR`, `TERM=dumb`, and non-TTY detection via `color::is_color_enabled()`.

## Non-Goals

- **No changes to pipeline execution logic.** The forge compilation, script execution, event decoding, hydration, and registry recording pipeline in `treb-forge` is unchanged beyond threading `gas_used` through `PipelineResult`.
- **No changes to JSON output schema** beyond potentially adding `gas_used`. The `RunOutputJson` structure and its serialization remain backward-compatible.
- **No changes to CLI flags beyond `--dump-command`.** Existing flags (`--broadcast`, `--dry-run`, `--slow`, `--legacy`, `--verify`, `--verbose`, `--debug`, `--json`, `--env`, `--target-contract`, `--non-interactive`) are unchanged.
- **No changes to sender resolution or Safe/Governor flows.** The sender system's behavior is unaffected; this phase only changes how results are displayed.
- **No changes to the list or show commands.** Phase 4 output is stable.
- **No interactive TUI or spinner.** Progress display is simple print statements, not interactive terminal updates.
- **No real-time streaming output.** Stage indicators are printed at transitions, not during long operations.

## Technical Considerations

### Dependencies
- **Phase 3 UI framework**: `TreeNode`, `color::*`, `badge::*` from `crates/treb-cli/src/ui/` for tree rendering and styled output
- **Phase 4 patterns**: `styled()` helper, `print_header()`, `print_kv()` patterns from show.rs — consider extracting shared helpers to a common location if they are duplicated
- **`group_deployments()` from list.rs**: Can be reused directly for grouping run result deployments into the tree hierarchy — the function accepts `&[&Deployment]` which can be constructed from `PipelineResult.deployments`
- **No new crate dependencies.** Everything builds on existing workspace crates.

### Integration Points
- `run.rs` will import `ui::{tree::TreeNode, color, badge}` for tree construction and styling
- `run.rs` will reuse `list::{group_deployments, build_deployment_node}` — ensure these are `pub` (they are already)
- `PipelineResult` in `treb-forge/src/pipeline/types.rs` gains a `gas_used: u64` field, set from `ExecutionResult.gas_used` in the orchestrator
- `ScriptConfig` needs a `to_forge_command()` method (or standalone function) for `--dump-command` — it has all the fields needed to reconstruct the CLI command
- The `--dump-command` flag needs to be added to the clap argument definitions (likely in `crates/treb-cli/src/cli.rs` or wherever the `run` subcommand's args are defined)

### Golden File Regeneration
- Use `UPDATE_GOLDEN=1 cargo test -p treb-cli -- run` to auto-regenerate golden files after output changes
- Stage indicators go to stderr — verify golden file framework captures stderr (it does based on existing verbose/debug tests)
- A `DebugFileNormalizer` may be needed to normalize the timestamp portion of `.treb/debug-<timestamp>.log` paths in golden output
- The `GasNormalizer` already exists and handles gas values — ensure it covers the new summary line format
- Changing the deployment output format in run will NOT affect list/show golden files since those use different test fixtures and code paths

### Patterns to Follow
- `styled(text, style)` for conditional color application (from show.rs)
- `print_header(title)` for section headers with `color::STAGE` (from show.rs)
- `print_kv(pairs)` for aligned key-value display (from output.rs)
- `color::is_color_enabled()` check before `render_styled()` vs `render()` (from list.rs)
- `output::truncate_address()` for `0xABCD...EFGH` format (from output.rs)
- All non-data output to stderr, data output to stdout (existing convention)

### Shared Helper Extraction
- The `styled()` and `print_header()` functions are currently defined in `show.rs` — if `run.rs` needs them too, consider moving them to `output.rs` or a shared `ui/helpers.rs` module. Alternatively, duplicate them in `run.rs` if the implementations are trivial (they are ~5 lines each).
