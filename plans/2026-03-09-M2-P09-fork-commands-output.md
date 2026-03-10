# PRD: Phase 9 - fork Commands Output

## Introduction

Phase 9 ports all seven fork subcommand outputs (`enter`, `exit`, `status`, `revert`, `restart`, `history`, `diff`) from the Go CLI's `render/fork.go` to the Rust CLI's `commands/fork.rs`. The current Rust implementation uses `print_stage()` emoji messages and comfy_table tables, while the Go version uses simple indented key-value pairs, arrow-marked history entries, and `+`/`~` prefix diff lines. This phase replaces the Rust output formatting to achieve exact string-for-string parity with Go.

This is a dependency of Phase 14 (JSON audit) and depends on Phase 1 (shared color palette and formatting primitives), which is already complete.

## Goals

1. **Exact human output parity**: All seven fork subcommands produce identical human-readable output to the Go CLI (same indentation, same field labels, same spacing, same markers).
2. **Preserve JSON output**: Existing `--json` output for `exit`, `status`, `history`, `diff`, `revert`, `restart` remains unchanged — only human-readable formatting changes.
3. **Replace comfy_table usage**: Fork commands currently use `build_table()` (comfy_table) for status, history, and diff — replace with Go-matching formats (indented fields, arrow entries, prefix lines).
4. **Golden test coverage**: All fork golden test `.expected` files updated to match the new Go-matching output format.
5. **Zero regressions**: `cargo test -p treb-cli` and `cargo clippy --workspace --all-targets` pass cleanly after all changes.

## User Stories

### P9-US-001: Port fork enter and fork restart Human Output

**Description:** Replace the current `print_stage()` progress messages and plain `println!` lines for `fork enter` and `fork restart` with Go's indented field list format. Both commands share the same output structure (message + indented fields + footer), so they are ported together.

**Go reference:** `render/fork.go` lines 18-38 (`RenderEnter`) and lines 106-125 (`RenderRestart`)

**Current Rust output (enter):**
```
📸 Snapshotting registry for 'localhost'...
✅ Fork mode entered for 'localhost'.
Entered fork mode for network 'localhost'.
Registry snapshot saved to .treb/snapshots/localhost
Run `treb dev anvil start --network localhost` to start a local Anvil node.
```

**Target Go output (enter):**
```
<message>

  Network:      <network>
  Chain ID:     <chain_id>
  Fork URL:     <fork_url>
  Anvil PID:    <pid>
  Env Override: <env_var>=<fork_url>
  Logs:         <log_file>
  Setup:        executed successfully    (conditional)

Run 'treb fork status' to check fork state
Run 'treb fork exit' to stop fork and restore original state
```

**Target Go output (restart):**
```
<message>

  Network:      <network>
  Chain ID:     <chain_id>
  Fork URL:     <fork_url>
  Anvil PID:    <pid>
  Env Override: <env_var>=<fork_url>
  Logs:         <log_file>
  Setup:        executed successfully    (conditional)

Registry restored to initial fork state. All previous snapshots cleared.
```

**Acceptance criteria:**
- `fork enter` human output matches Go `RenderEnter` format exactly: message line, blank line, 2-space indented fields with aligned colons, blank line, two footer help lines
- `fork restart` human output matches Go `RenderRestart` format: same field list as enter, but footer is "Registry restored to initial fork state. All previous snapshots cleared."
- Setup line only displayed when setup script ran (conditional, matching Go)
- Existing `--json` output for both commands unchanged
- `cargo clippy --workspace --all-targets` passes
- Note: The Rust enter command has different data flow from Go (Anvil PID, Fork URL, Env Override, Log file may not all be available at enter time since Rust separates `fork enter` from `dev anvil start`). Map available fields to the Go format; fields not yet known should be omitted or show contextually appropriate values. The key requirement is matching the format structure (indented fields with aligned labels), not inventing data that doesn't exist.

---

### P9-US-002: Port fork exit Human Output

**Description:** Replace the current `print_stage()` messages for `fork exit` with Go's per-network cleanup confirmation format.

**Go reference:** `render/fork.go` lines 40-48 (`RenderExit`)

**Current Rust output:**
```
🔄 Restoring registry for 'mainnet'...
🧹 Cleaning up snapshot for 'mainnet'...
✅ Fork mode exited for 'mainnet'.
Exited fork mode for network 'mainnet'.
Registry restored from snapshot.
```

**Target Go output:**
```
<message>

  - <network>: registry restored, fork cleaned up
```

**Acceptance criteria:**
- `fork exit` human output matches Go `RenderExit` format: message line, blank line, per-network `  - NETWORK: registry restored, fork cleaned up` lines
- When `--all` exits multiple networks, each gets its own `  - NETWORK: ...` line
- Existing `--json` output unchanged
- `cargo clippy --workspace --all-targets` passes

---

### P9-US-003: Port fork status Human Output

**Description:** Replace the comfy_table 10-column table for `fork status` with Go's indented key-value format. This is the most significant formatting change — from a wide table to per-entry indented blocks.

**Go reference:** `render/fork.go` lines 50-81 (`RenderStatus`)

**Current Rust output:** comfy_table with columns: Network, Chain ID, RPC URL, Port, Fork Block, Started At, Uptime, Snapshots, Deployments, Status

**Target Go output:**
```
Active Forks

  <network> (current)
    Chain ID:     <chain_id>
    Fork URL:     <fork_url>
    Anvil PID:    <pid>
    Status:       <health_detail>
    Uptime:       <formatted_duration>
    Snapshots:    <count>
    Fork Deploys: <count>
    Logs:         <log_file>

```

**Acceptance criteria:**
- "No active forks" message when no forks exist (matching Go)
- "Active Forks" header followed by blank line
- Per-entry: 2-space indented network name with optional ` (current)` marker
- Per-entry fields: 4-space indented, labels aligned with Go spacing (14-char label width)
- Uptime uses `output::format_duration()` (already matches Go `formatDuration`)
- Logs line only shown when log file is non-empty (matching Go conditional)
- Blank line between entries
- Existing `--json` output unchanged
- `cargo clippy --workspace --all-targets` passes

---

### P9-US-004: Port fork revert Human Output

**Description:** Replace the current `print_stage()` messages for `fork revert` with Go's message + reverted command + remaining snapshots format.

**Go reference:** `render/fork.go` lines 94-104 (`RenderRevert`)

**Current Rust output:**
```
⏮️ Reverting EVM state for 'mainnet'...
📸 Taking new EVM snapshot for 'mainnet'...
🔄 Restoring registry for 'mainnet'...
✅ Fork reverted for 'mainnet'.
Reverted fork for network 'mainnet' to initial state.
```

**Target Go output:**
```
<message>

  Reverted:   <command>       (conditional, only for single revert)
  Reverted:   <count> snapshot(s)
  Remaining:  <count> snapshot(s)
```

**Acceptance criteria:**
- `fork revert` human output matches Go `RenderRevert` format: message, blank line, indented reverted command (when available), reverted count, remaining count
- `Reverted: <command>` line only shown when `reverted_command` is non-empty (matching Go conditional)
- Field labels aligned at 12 chars (`Reverted:   `, `Remaining:  `) matching Go spacing
- `--all` revert shows aggregate counts
- Existing `--json` output unchanged
- `cargo clippy --workspace --all-targets` passes

---

### P9-US-005: Port fork history Human Output

**Description:** Replace the comfy_table 4-column history table with Go's arrow-marked per-entry format. The Go history is per-network with snapshot indices and current-entry markers, while Rust currently shows a cross-network table with action/details columns. Adapt the display to match Go's format while using the available Rust data.

**Go reference:** `render/fork.go` lines 127-150 (`RenderHistory`)

**Current Rust output:** comfy_table with columns: Timestamp, Action, Network, Details

**Target Go output:**
```
Fork History: <network>

  → [0] initial  (2024-01-01 00:00:00)
    [1] run script/Deploy.s.sol  (2024-01-01 00:05:00)
    [2] run script/Upgrade.s.sol  (2024-01-01 00:10:00)

```

**Acceptance criteria:**
- Header: `Fork History: <network>` followed by blank line
- Per entry: `  <marker>[<index>] <label>  (<timestamp>)` where marker is `→ ` for current entry or `  ` (two spaces) for others
- Label is "initial" for index 0, otherwise the command string
- Timestamps use `%Y-%m-%d %H:%M:%S` format (no `UTC` suffix, matching Go)
- Trailing blank line after entries
- Existing `--json` output unchanged
- `cargo clippy --workspace --all-targets` passes
- Note: The Rust data model stores history as action/network/details/timestamp. Map this to Go's index/command/timestamp format — use entry position as index, map action+details to the label string.

---

### P9-US-006: Port fork diff Human Output

**Description:** Replace the comfy_table 3-column diff table with Go's `+`/`~` prefix format with deployment details and transaction count.

**Go reference:** `render/fork.go` lines 152-184 (`RenderDiff`)

**Current Rust output:** comfy_table with columns: Change, File, Key

**Target Go output:**
```
Fork Diff: <network>

New Deployments (N):
  + ContractName         0xAddress  TYPE
  + AnotherContract      0xAddress  TYPE

Modified Deployments (N):
  ~ ContractName         0xAddress  TYPE

New Transactions: N

```

Or when no changes: `No changes since fork entered.`

**Acceptance criteria:**
- Header: `Fork Diff: <network>` followed by blank line
- "No changes since fork entered." when clean (matching Go exactly)
- "New Deployments (N):" section with `  + %-20s %s  %s` per entry (name left-padded to 20 chars, address, type)
- "Modified Deployments (N):" section with `  ~ %-20s %s  %s` per entry
- "New Transactions: N" line when count > 0
- Blank line between sections and after last section
- Existing `--json` output unchanged
- `cargo clippy --workspace --all-targets` passes
- Note: The Rust diff data model uses file-level changes (change/file/key) rather than Go's deployment-level model (ContractName/Address/Type). Map the available data to Go's format structure — use the key as the contract name, and extract address/type from the registry if feasible, otherwise display what's available in the `+`/`~` prefix format.

---

### P9-US-007: Update All Golden Test Expected Files

**Description:** Regenerate all fork golden test `.expected` files to match the new output format. Verify that all tests pass, including non-fork golden tests to catch any cross-command drift.

**Acceptance criteria:**
- Run `UPDATE_GOLDEN=1 cargo test -p treb-cli --test integration_fork` to regenerate fork golden files
- Run `cargo test -p treb-cli` (full CLI test suite) to verify no cross-command regressions
- Review the diff of all updated `.expected` files to confirm they match Go output format
- Error-path golden files (e.g., `fork_enter_not_initialized`, `fork_exit_not_forked`) should be unchanged or only minimally affected
- `cargo clippy --workspace --all-targets` passes
- All 175+ golden tests pass

## Functional Requirements

- **FR-1:** `fork enter` displays message + indented fields (Network, Chain ID, Fork URL, Anvil PID, Env Override, Logs, Setup) + footer help lines, matching Go `RenderEnter`.
- **FR-2:** `fork exit` displays message + per-network `  - NETWORK: registry restored, fork cleaned up` lines, matching Go `RenderExit`.
- **FR-3:** `fork status` displays "Active Forks" header + per-entry indented fields (Chain ID, Fork URL, Anvil PID, Status, Uptime, Snapshots, Fork Deploys, Logs) with optional `(current)` marker, matching Go `RenderStatus`.
- **FR-4:** `fork revert` displays message + `  Reverted: COMMAND` (conditional) + `  Reverted: N snapshot(s)` + `  Remaining: N snapshot(s)`, matching Go `RenderRevert`.
- **FR-5:** `fork restart` displays same field list as enter + "Registry restored to initial fork state..." footer, matching Go `RenderRestart`.
- **FR-6:** `fork history` displays `Fork History: NETWORK` + arrow-marked entries `  [MARKER] [INDEX] LABEL  (TIMESTAMP)`, matching Go `RenderHistory`.
- **FR-7:** `fork diff` displays `Fork Diff: NETWORK` + `+` new / `~` modified deployments with aligned columns + transaction count, matching Go `RenderDiff`.
- **FR-8:** All `--json` output remains schema-identical (no changes to JSON serialization).
- **FR-9:** `format_duration()` from `output.rs` is used for uptime display in status (already Go-matching).
- **FR-10:** Timestamps in history use `%Y-%m-%d %H:%M:%S` format (matching Go, no UTC suffix in this context).

## Non-Goals

- **No changes to fork command business logic** — only display formatting changes.
- **No changes to JSON output schemas** — `--json` output is preserved exactly.
- **No new fork subcommands** — all seven subcommands already exist.
- **No changes to `format_duration()`** — already matches Go `formatDuration` from Phase 1.
- **No changes to fork data model types** (`ForkEntry`, `ForkHistoryEntry`, etc.) — use existing structs.
- **No color/styling changes** — this phase uses the color palette and emoji from Phase 1 as-is; Go fork output is uncolored plain text with no emoji, so the main change is structural formatting.
- **No changes to error messages or error-path output** — only success-path human output is ported.

## Technical Considerations

### Dependencies
- **Phase 1 (complete):** Color palette, emoji constants, `format_duration()`, `format_success()`, `format_warning()` — all available in `crate::ui::emoji`, `crate::ui::color`, and `crate::output`.

### Key Differences Between Go and Rust Data Models

1. **Enter/Restart:** Go has `ForkEntry` with `Network`, `ChainID`, `ForkURL`, `AnvilPID`, `EnvVarName`, `LogFile` fields. Rust's fork enter separates fork entry from anvil start (`dev anvil start`), so some fields (PID, ForkURL, LogFile) may not be populated at enter time. The display must handle fields that are available and omit those that aren't.

2. **History:** Go uses per-network `ForkHistoryEntry` with `Index`, `Command`, `Timestamp`, `IsCurrent`, `IsInitial`. Rust stores `ForkHistoryEntry` with `action`, `network`, `details`, `timestamp`. The display adapter must map action+details to Go's label format and use position as index.

3. **Diff:** Go uses structured `ForkDiffEntry` with `ContractName`, `Address`, `Type`. Rust uses file-level changes (`change`, `file`, `key`). The display should use the `+`/`~` prefix format structure from Go with whatever data is available from the Rust model.

### Formatting Patterns
- Go fork output uses plain `fmt.Printf` with fixed-width labels (e.g., `"  Network:      %s\n"` — 14 chars for label). No emoji, no colors in the Go fork renderer.
- The Rust `styled()` helper pattern (conditional coloring) from show.rs/verify.rs can be used if any styling is desired, but Go fork output is plain text.
- Replace `build_table()` calls with direct `println!`/`eprintln!` using Go-matching format strings.

### Testing Strategy
- Use `--test integration_fork` to target fork golden tests during development.
- Run full `cargo test -p treb-cli` after all changes to catch cross-command drift.
- Set `NO_COLOR=1` in tests that check human output with `contains()` assertions.
- `UPDATE_GOLDEN=1` to regenerate `.expected` files after formatting changes.
