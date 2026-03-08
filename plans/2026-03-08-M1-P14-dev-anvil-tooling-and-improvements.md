# PRD: Phase 14 - dev anvil Tooling and Improvements

## Introduction

Phase 14 completes the `treb dev anvil` subcommand family and improves the local development experience. The Rust CLI already has working `start`, `stop`, and `status` subcommands (in `crates/treb-cli/src/commands/dev.rs`), but `restart` is a stub, `logs` does not exist, `status` output lacks styling/uptime, and there is no `--name` flag for managing multiple concurrent Anvil instances. This phase fills those gaps to reach parity with the Go CLI's dev anvil tooling.

This phase is independent — no dependency on other phases — and focuses entirely on `crates/treb-cli/src/commands/dev.rs`, the `ForkEntry` type in `treb-core`, and golden file tests.

## Goals

1. **Implement `dev anvil restart`** — stop + start with the same configuration, replacing the current stub.
2. **Implement `dev anvil logs`** — stream the Anvil log file (tail -f style) for a running instance.
3. **Improve `dev anvil status` output** — add uptime calculation, styled output with color, and RPC URL display matching Go conventions.
4. **Add named instance support** — `--name` flag on start/stop/restart/status/logs for managing multiple concurrent Anvil nodes with separate PID/log file tracking.
5. **Add golden file tests** — cover status, restart, and logs subcommands with deterministic golden file output.

## User Stories

### P14-US-001: Named Instance Support and Log File Management

**Description:** Add `--name` flag to all `dev anvil` subcommands and implement log file management. Currently the Rust CLI ignores the `pid_file` and `log_file` fields on `ForkEntry` (they exist for Go compatibility but are never populated). This story populates those fields during `start` and uses them for `logs` and `status`. The `--name` flag allows multiple concurrent Anvil instances (e.g., `--name mainnet-fork` and `--name sepolia-fork`). When `--name` is not provided, the network name is used as the instance name; when neither is provided, the default name is `"default"`.

**Changes:**
- Add `--name` optional arg to `AnvilSubcommand::Start`, `Stop`, `Restart`, `Status`, `Logs` variants in `dev.rs`
- Add helper `resolve_instance_name(name: Option<&str>, network: Option<&str>) -> String` that returns the name or network or `"default"`
- In `run_anvil_start`: populate `ForkEntry.log_file` with `.treb/anvil-{name}.log` and `ForkEntry.pid_file` with `.treb/anvil-{name}.pid`; write the current process PID to the PID file on start; remove on shutdown
- Add `--name` to clap parsing tests

**Acceptance Criteria:**
- `--name` flag is accepted on all five subcommands (start, stop, restart, status, logs)
- `resolve_instance_name(Some("my-node"), None)` returns `"my-node"`
- `resolve_instance_name(None, Some("mainnet"))` returns `"mainnet"`
- `resolve_instance_name(None, None)` returns `"default"`
- `run_anvil_start` writes `.treb/anvil-{name}.pid` containing the process PID
- `run_anvil_start` populates `ForkEntry.log_file` and `ForkEntry.pid_file` with correct paths
- PID file is removed on graceful shutdown
- Clap parsing tests cover `--name` on start and stop
- `cargo check -p treb-cli` passes

---

### P14-US-002: Restart Subcommand Implementation

**Description:** Replace the `restart` stub with a working implementation that stops the current instance and starts a new one with the same configuration. The restart reads the existing `ForkEntry` for the resolved instance name, checks port reachability to determine if stop cleanup is needed, then delegates to the start flow with the same fork URL, block number, and port.

**Changes:**
- Implement `run_anvil_restart` in `dev.rs`: resolve instance name, load fork state, extract config from existing entry, call `run_anvil_stop` if port is reachable, then call `run_anvil_start` with the extracted config
- Add `port` and `fork_block_number` optional args to `Restart` variant (to allow overriding on restart)
- Add fork history entry with action `"anvil-restart"` on successful restart
- Add unit test for restart clap parsing

**Acceptance Criteria:**
- `treb dev anvil restart --network mainnet` reads existing fork entry config, stops the old instance (if reachable), and starts a new one
- `--port` and `--fork-block-number` flags on restart override the existing entry values
- `--name` flag works on restart to target a named instance
- Fork history records an `"anvil-restart"` action
- If no existing fork entry is found, returns a descriptive error
- `cargo check -p treb-cli` passes

---

### P14-US-003: Improved Status Output with Uptime and Styling

**Description:** Enhance `run_anvil_status` to show uptime (human-readable duration since `started_at`), apply styled output using the existing color palette, and improve the JSON output with an `uptime` field. Add `--name` filtering so `status` can show a single named instance instead of all instances.

**Changes:**
- Add `format_uptime(started_at: DateTime<Utc>) -> String` helper that returns e.g. `"2h 15m"`, `"3d 4h"`, `"< 1m"`
- Apply `color::SUCCESS` styling to "running" status and `color::ERROR` to "stopped" status (gated on `is_color_enabled()`)
- Add `name` optional arg to `Status` variant for filtering to a single instance
- Add `"uptime"` field to JSON output
- Use `output::print_json()` (with `sort_json_keys`) instead of raw `serde_json::to_string_pretty` for deterministic JSON output
- When `--name` is specified, filter to matching entry only

**Acceptance Criteria:**
- Status table includes an "Uptime" column showing human-readable duration
- "running" status is green, "stopped" is red when color is enabled
- `--json` output includes `"uptime": "2h 15m"` field for each instance
- `--json` output uses `output::print_json()` for sorted keys
- `--name` filters output to a single instance
- `format_uptime` returns `"< 1m"` for durations under 60 seconds, `"Xm"` for minutes, `"Xh Ym"` for hours, `"Xd Yh"` for days
- `cargo test -p treb-cli` passes (unit tests for `format_uptime`)

---

### P14-US-004: Logs Subcommand and Golden File Tests

**Description:** Implement `treb dev anvil logs` to stream the log file for a running Anvil instance, and add golden file tests for status output. The logs subcommand reads the `log_file` path from the fork state entry and tails it. Also add `--follow` flag for continuous streaming (default: print current contents and exit). Add golden file tests for the status subcommand (both table and JSON modes).

**Changes:**
- Add `Logs` variant to `AnvilSubcommand` with `--name`, `--network`, and `--follow` flags
- Implement `run_anvil_logs`: resolve instance name, load fork state, read `log_file` path, print contents; if `--follow`, use async file watching / polling to stream new lines
- Wire `Logs` in the dispatch match arm
- Add golden file tests for `dev anvil status` (table output with seeded fork state, empty state)
- Add golden file test for `dev anvil status --json`
- Add `TimestampNormalizer` or reuse existing normalizer for uptime/timestamp fields in golden output

**Acceptance Criteria:**
- `treb dev anvil logs` prints log file contents for the default instance
- `treb dev anvil logs --name my-node` prints log file for the named instance
- `treb dev anvil logs --follow` continuously streams new log lines (blocking)
- Returns descriptive error if no log file exists or instance not found
- Golden file tests exist for: status with active instances (table), status with no instances, status `--json`
- Golden files use normalizers for timestamps and uptime values
- `cargo test -p treb-cli` passes

## Functional Requirements

- **FR-1:** All `dev anvil` subcommands accept an optional `--name <string>` flag for instance identification.
- **FR-2:** Instance name resolution follows precedence: explicit `--name` > `--network` value > `"default"`.
- **FR-3:** `dev anvil start` writes a PID file at `.treb/anvil-{name}.pid` and populates `ForkEntry.pid_file` and `ForkEntry.log_file`.
- **FR-4:** `dev anvil restart` reads the existing fork entry configuration (fork URL, block number, port) and reuses it for the new instance, unless overridden by flags.
- **FR-5:** `dev anvil status` displays uptime as human-readable duration, and status is color-coded (green=running, red=stopped).
- **FR-6:** `dev anvil status --json` output uses `output::print_json()` for deterministic key ordering and includes an `uptime` field.
- **FR-7:** `dev anvil logs` reads the log file path from the fork state and prints its contents; `--follow` enables continuous streaming.
- **FR-8:** PID files are cleaned up on graceful shutdown (SIGINT/SIGTERM).
- **FR-9:** Fork history entries are recorded for restart actions with action `"anvil-restart"`.

## Non-Goals

- **Background/daemon mode:** The Go CLI runs Anvil as a background subprocess with PID tracking. The Rust CLI uses in-process Anvil nodes that block the foreground. Converting to background daemon mode is out of scope — the foreground model is simpler and more reliable.
- **Log capture from in-process Anvil:** The `AnvilConfig` uses `.silent()` mode which suppresses Anvil output. Capturing in-process Anvil logs to a file would require changes to `NodeConfig` and is deferred. The `logs` command will work with log files that exist (e.g., from Go CLI instances or future enhancements).
- **Multi-node orchestration:** Managing multiple Anvil nodes as a coordinated set (e.g., L1 + L2) is out of scope.
- **Auto-restart on crash:** No crash detection or watchdog functionality.
- **Windows-specific signal handling:** The existing `wait_for_shutdown_signal` already has a Windows fallback (Ctrl+C only); no additional Windows work.

## Technical Considerations

### Dependencies
- No new crate dependencies required. Uses existing `chrono` (uptime), `tokio` (signals, file I/O), `serde_json`, and the `crate::output` + `crate::ui::color` modules.

### Key Files
| File | Changes |
|------|---------|
| `crates/treb-cli/src/commands/dev.rs` | All subcommand implementations, helpers, tests |
| `crates/treb-cli/src/main.rs` | No changes needed (dispatch already wired) |
| `crates/treb-core/src/types/fork.rs` | No schema changes (fields already exist) |
| `crates/treb-cli/tests/` | Golden file tests for status output |

### Existing Patterns to Follow
- **Styled output:** Use `styled()` helper pattern gated on `color::is_color_enabled()` (established in Phase 4)
- **Tables:** Use `output::build_table()` + `output::print_table()` (already used in current `run_anvil_status`)
- **JSON output:** Use `output::print_json()` which calls `sort_json_keys()` for deterministic output
- **Stage messages:** Use `output::print_stage(emoji, message)` for restart progress (established in Phase 5)
- **Golden files:** Follow `GoldenFile::new().compare()` pattern from fork integration tests; add normalizers for dynamic values

### In-Process vs Subprocess Model
The Rust CLI spawns Anvil in-process via `anvil::try_spawn(NodeConfig)`. This means:
- No actual PID file is needed for process management (the Rust process IS the Anvil process)
- The PID file is written for Go CLI compatibility and status display
- Log file support is limited since `NodeConfig::silent()` suppresses output; the `logs` command handles the case where no log file exists gracefully
- `restart` can reuse `run_anvil_stop` (cleanup stale entries) + `run_anvil_start` (spawn new instance) since stop doesn't kill a process — it cleans up fork state for unreachable ports
