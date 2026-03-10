# PRD: Phase 13 - dev anvil Command Output

## Introduction

Phase 13 ports the `treb dev anvil` subcommand output from the Go CLI to exact 1:1 parity in the Rust CLI. The Go CLI uses emoji-rich, color-coded output for each of the five anvil subcommands (start, stop, restart, status, logs), while the Rust CLI currently uses plain `println!` text and a comfy_table for status. This phase replaces all human-readable output with Go-matching formats using the shared emoji, color, and formatting primitives established in Phase 1.

The Go rendering lives in `internal/cli/render/anvil.go` (AnvilRenderer) and the command handler in `internal/cli/dev.go`. The Rust target is `crates/treb-cli/src/commands/dev.rs`.

## Goals

1. **Start/restart output matches Go exactly** — green checkmark message, yellow clipboard logs path, blue globe RPC URL, and CreateX deployment status with conditional checkmark/warning.
2. **Stop output matches Go exactly** — green checkmark success message.
3. **Status output matches Go exactly** — replaces the comfy_table with Go's per-instance emoji/color format: cyan bold header, green/red circle status, RPC health, CreateX status, and gray fields when not running.
4. **Logs header matches Go exactly** — cyan bold clipboard-emoji header with Ctrl+C instruction and gray log file path.
5. **All golden test expected files updated** — existing golden tests (`dev_anvil_status_no_instances`, `dev_anvil_status_with_instances`, `dev_anvil_status_json`) pass with the new output format.

## User Stories

### P13-US-001: Port dev anvil start and restart Human Output to Go Format

**Description:** Replace the plain `println!` messages in `run_anvil_start_with_entry()` with Go-matching styled output: green checkmark for success, yellow clipboard-emoji for log file path, blue globe-emoji for RPC URL, and CreateX status with green checkmark or red warning.

**Go reference:** `render/anvil.go` lines 35-49 (`renderStart`) and lines 59-62 (`renderRestart` delegates to `renderStart`).

**Current Rust output (start):**
```
Anvil node started at http://127.0.0.1:8545
CreateX factory deployed at 0xba5Ed...
Fork state updated for network 'mainnet' (port 8545).
Press Ctrl+C (or send SIGTERM) to stop.
```

**Target Go output (start):**
```
✅ Anvil node 'NAME' started successfully
📋 Logs: PATH
🌐 RPC URL: URL
✅ CreateX factory deployed at 0xba5Ed...
```
(Or red `⚠️  Warning: Failed to deploy CreateX` + yellow `Deployments may fail without CreateX factory` on failure.)

**Changes:**
- In `run_anvil_start_with_entry()` (~lines 170-213 of dev.rs), replace the four `println!` calls after spawn/CreateX with styled output using `emoji::CHECK`, `emoji::CLIPBOARD`, `emoji::GLOBE`, `emoji::WARNING`, and color styles `color::GREEN`, `color::YELLOW`, `color::BLUE`, `color::RED`.
- Add a `styled()` helper (or use `owo_colors` directly) for conditional coloring based on `color::is_color_enabled()`.
- The restart subcommand delegates to `run_anvil_start_with_entry()` so it automatically picks up the same format.
- The success message should include the instance name: `"Anvil node '{instance_name}' started successfully"` matching Go's `result.Message`.

**Acceptance criteria:**
- `treb dev anvil start` output shows green `✅ {message}`, yellow `📋 Logs: {path}`, blue `🌐 RPC URL: {url}`, and green `✅ CreateX factory deployed at {addr}`.
- Restart output is identical in format (same function).
- Colors respect `NO_COLOR` / `--no-color`.
- `cargo clippy --workspace --all-targets` passes.
- `cargo test -p treb-cli` compiles (existing tests may need updates in US-004).

---

### P13-US-002: Port dev anvil stop Human Output to Go Format

**Description:** Replace the plain `println!` messages in `run_anvil_stop()` with Go-matching styled output: green checkmark for success messages.

**Go reference:** `render/anvil.go` lines 51-57 (`renderStop` — green `✅ {message}`).

**Current Rust output (stop):**
```
Removed stale fork state entry for network 'mainnet'.
```
or:
```
No stale fork state entries found.
```
or:
```
Network 'mainnet' is still reachable at port 8545; skipping.
```

**Target Go output:**
```
✅ Stopped anvil node 'NAME'
```

**Changes:**
- In `run_anvil_stop()` (~lines 290-335 of dev.rs), replace `println!` removal messages with green styled `✅` checkmark messages matching Go format.
- Keep informational messages (still reachable, no instances) but style them with appropriate colors (yellow for skipping, green checkmark for removed).

**Acceptance criteria:**
- `treb dev anvil stop` success output shows green `✅ {message}`.
- Informational messages (still reachable, no stale entries) are appropriately styled.
- Colors respect `NO_COLOR` / `--no-color`.
- `cargo clippy --workspace --all-targets` passes.

---

### P13-US-003: Port dev anvil status Human Output to Go Format

**Description:** Replace the comfy_table-based status display in `run_anvil_status()` with Go's per-instance emoji/color format. This is the most significant change in this phase — the table is replaced with a per-instance detailed view.

**Go reference:** `render/anvil.go` lines 64-91 (`renderStatus`).

**Current Rust output (status with instances):**
```
┌─────────┬──────────┬────────────────────────┬───────┬──────────┬────────────┬──────────────────────┬─────────┬─────────┐
│ Network ┆ Instance ┆ RPC URL                ┆ Port  ┆ Chain ID ┆ Fork Block ┆ Started At           ┆ Uptime  ┆ Status  │
...
```

**Target Go output (running):**
```
📊 Anvil Status ('NAME'):
Status: 🟢 Running (PID N)
RPC URL: http://127.0.0.1:8545
Log file: .treb/anvil-NAME.log
RPC Health: ✅ Responding
CreateX Status: ✅ Deployed at 0xba5Ed...
```

**Target Go output (not running):**
```
📊 Anvil Status ('NAME'):
Status: 🔴 Not running
PID file: .treb/anvil-NAME.pid
Log file: .treb/anvil-NAME.log
```

**Changes:**
- In `run_anvil_status()` (~lines 434-534 of dev.rs), replace the `build_table()` / `print_table()` block with Go-matching per-instance output.
- Cyan bold header: `📊 Anvil Status ('{instance_name}'):` using `color::STAGE` and `emoji::CHART`.
- Running state: green `Status: 🟢 Running (PID {pid})`, blue `RPC URL: {url}`, yellow `Log file: {path}`, RPC health check with `✅ Responding` or `❌ Not responding`, CreateX status with `✅ Deployed at {addr}` or `❌ Not deployed`.
- Not running state: red `Status: 🔴 Not running`, gray `PID file: {path}`, gray `Log file: {path}`.
- For multiple instances, render each with the same per-instance block.
- Keep the "No active Anvil instances." message for empty state.
- Remove `comfy_table` imports if no longer used in dev.rs.
- JSON output (`--json`) is unchanged.
- RPC health check: attempt a JSON-RPC call (e.g., `eth_chainId`) to verify responsiveness.
- CreateX status: check if CreateX bytecode exists at the canonical address.
- Note: The Go `renderStatus` takes a single instance result — when Rust lists multiple instances, render each one with its own `📊 Anvil Status ('NAME'):` header.

**Acceptance criteria:**
- `treb dev anvil status` with running instance shows cyan bold chart header, green circle status with PID, blue RPC URL, yellow log file, RPC health check, CreateX status.
- `treb dev anvil status` with stopped instance shows red circle status, gray PID/log paths.
- `treb dev anvil status` with no instances shows "No active Anvil instances."
- JSON output (`--json`) is unchanged (golden file passes as-is).
- `comfy_table` imports removed from dev.rs if no longer used.
- Colors respect `NO_COLOR` / `--no-color`.
- `cargo clippy --workspace --all-targets` passes.

---

### P13-US-004: Port dev anvil logs Header to Go Format

**Description:** Add the Go-matching logs header before streaming log contents. Currently, `run_anvil_logs()` just prints raw log file contents with no header.

**Go reference:** `render/anvil.go` lines 93-98 (`RenderLogsHeader`).

**Target output:**
```
📋 Showing anvil 'NAME' logs (Ctrl+C to exit):
Log file: .treb/anvil-NAME.log

<log contents>
```

**Changes:**
- In `run_anvil_logs()` (~lines 546-574 of dev.rs), add a header before printing/streaming log file contents.
- Cyan bold: `📋 Showing anvil '{instance_name}' logs (Ctrl+C to exit):` using `color::STAGE` and `emoji::CLIPBOARD`.
- Gray: `Log file: {path}` using `color::GRAY`.
- Print an empty line after the header (before log contents).
- Only show the `(Ctrl+C to exit)` text when `--follow` is active; for non-follow mode, omit it or adjust text (Go shows it always since logs implies streaming).

**Acceptance criteria:**
- `treb dev anvil logs` output starts with cyan bold clipboard header and gray log file path before log contents.
- `treb dev anvil logs --follow` shows the same header.
- Colors respect `NO_COLOR` / `--no-color`.
- `cargo clippy --workspace --all-targets` passes.

---

### P13-US-005: Update Golden Test Expected Files for dev anvil Output

**Description:** Update all golden test expected files and integration test assertions to match the new Go-format output.

**Golden files to update:**
- `tests/golden/dev_anvil_status_no_instances/commands.golden` — currently shows `"No active Anvil instances."` (may stay the same or need minor format changes)
- `tests/golden/dev_anvil_status_with_instances/commands.golden` — currently shows comfy_table output, must change to per-instance emoji/color format
- `tests/golden/dev_anvil_status_json/commands.golden` — JSON output should be unchanged

**Integration test file:**
- `tests/integration_dev.rs` — `sample_anvil_entry()` and `seed_anvil_status()` helpers may need updates; test functions `dev_anvil_status_no_instances`, `dev_anvil_status_with_instances`, `dev_anvil_status_json` may need assertion changes.

**Changes:**
- Run `UPDATE_GOLDEN=1 cargo test -p treb-cli --test integration_dev` to regenerate golden expected files.
- Verify that `dev_anvil_status_json` golden file is unchanged.
- Verify that `dev_anvil_status_no_instances` golden file reflects any formatting changes.
- Verify that `dev_anvil_status_with_instances` golden file now shows per-instance emoji format instead of comfy_table.
- Update any unit tests in `dev.rs` `mod tests` that assert on human output text.
- Run full `cargo test -p treb-cli` to confirm no cross-command golden drift.
- Run `cargo clippy --workspace --all-targets` to confirm no warnings.

**Acceptance criteria:**
- All golden tests pass: `cargo test -p treb-cli --test integration_dev`.
- JSON golden file (`dev_anvil_status_json`) is unchanged.
- Status golden files reflect Go-matching per-instance emoji format.
- Full test suite passes: `cargo test -p treb-cli`.
- `cargo clippy --workspace --all-targets` passes with no warnings.

## Functional Requirements

- **FR-1:** `dev anvil start` output: green `✅ {message}`, yellow `📋 Logs: {path}`, blue `🌐 RPC URL: {url}`, CreateX status with green `✅` or red `⚠️` warning.
- **FR-2:** `dev anvil stop` output: green `✅ {message}` for successful cleanup.
- **FR-3:** `dev anvil restart` output: identical to start format (delegates to same function).
- **FR-4:** `dev anvil status` output: cyan bold `📊 Anvil Status ('{name}'):` header, green/red circle status indicator, RPC health check result, CreateX deployment status.
- **FR-5:** `dev anvil status` (not running): red `🔴 Not running` status with gray PID/log file paths.
- **FR-6:** `dev anvil logs` output: cyan bold `📋 Showing anvil '{name}' logs (Ctrl+C to exit):` header with gray log file path.
- **FR-7:** All output respects `NO_COLOR` env var, `TERM=dumb`, and `--no-color` flag.
- **FR-8:** JSON output (`--json`) is unchanged — no schema modifications.
- **FR-9:** Emoji constants from `crate::ui::emoji` used consistently (not hardcoded strings).
- **FR-10:** Color styles from `crate::ui::color` used consistently (not inline ANSI codes).

## Non-Goals

- **No JSON schema changes.** The `--json` output for `dev anvil status` remains unchanged.
- **No new subcommands.** The five existing subcommands (start, stop, restart, status, logs) are ported as-is.
- **No behavioral changes.** The underlying logic (port reachability, fork state management, CreateX deployment, signal handling) is untouched — only the display formatting changes.
- **No RPC health check or CreateX check implementation for status.** If the existing Rust status code does not perform these checks, add minimal helper calls (reuse `is_port_reachable()` and a simple `eth_getCode` check), but do not implement full health monitoring.
- **No comfy_table removal from output.rs.** Only remove comfy_table usage from dev.rs; other commands may still use it.

## Technical Considerations

### Dependencies
- **Phase 1 (completed):** Provides `crate::ui::emoji` constants (`CHECK`, `CLIPBOARD`, `GLOBE`, `CHART`, `GREEN_CIRCLE`, `RED_CIRCLE`, `WARNING`, `CROSS`), `crate::ui::color` styles (`GREEN`, `BLUE`, `YELLOW`, `RED`, `STAGE`, `GRAY`, `SUCCESS`), and `color::is_color_enabled()`.

### Key Patterns from Prior Phases
- **`styled()` helper pattern** (from show.rs, verify.rs, fork.rs): Conditionally applies `owo_colors::Style` based on `color::is_color_enabled()`. Create a local `styled()` helper in dev.rs or use the pattern inline.
- **`emoji::CHECK_MARK` vs `emoji::CHECK`:** Go uses `✅` (CHECK) for start/stop/status success lines. Match the Go reference.
- **Output channel:** Start/stop/restart success output goes to stdout (`println!`). Logs header can go to stderr (`eprintln!`) since log content goes to stdout.
- **Integration test color:** Tests asserting on human output must use `.env("NO_COLOR", "1")` to strip ANSI codes (from Phase 12 learnings).

### Files Modified
- `crates/treb-cli/src/commands/dev.rs` — all five subcommand output sections
- `crates/treb-cli/tests/golden/dev_anvil_status_with_instances/commands.golden` — table → per-instance format
- `crates/treb-cli/tests/golden/dev_anvil_status_no_instances/commands.golden` — verify/update
- `crates/treb-cli/tests/integration_dev.rs` — test helper and assertion updates

### RPC Health and CreateX Checks for Status
The Go status checks `result.Status.RPCHealthy` and `result.Status.CreateXDeployed`. The Rust `run_anvil_status()` currently only checks `is_port_reachable()`. To match Go:
- **RPC Health:** Reuse `is_port_reachable()` or add a minimal `eth_chainId` JSON-RPC call.
- **CreateX Status:** Add an `eth_getCode` call to the canonical CreateX address (`0xba5Ed099633D3B313e4D5F7bdc1305d3c28ba5Ed`) — if code is non-empty, CreateX is deployed.
- Both checks only run when the instance is running (port reachable).
