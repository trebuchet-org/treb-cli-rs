# Master Plan: Exact 1:1 Output Parity with Go CLI

Port every command's display logic from the Go CLI (`../treb-cli`) to the Rust CLI (`treb-cli-rs`) so that human-readable output is an **exact copy** of the Go version. Each phase references the precise Go source files containing the rendering code to translate and the Rust files to modify. JSON output (`--json`) must be schema-identical. Human output must match string-for-string: same emoji, same tree characters, same colors, same spacing, same section headers.

**Reference codebase:** `../treb-cli/internal/cli/render/` (Go rendering module) and `../treb-cli/internal/cli/*.go` (command handlers)
**Target codebase:** `crates/treb-cli/src/output.rs`, `crates/treb-cli/src/ui/`, `crates/treb-cli/src/commands/`

---

## Phase 1 -- Shared Color Palette and Formatting Primitives

Port the exact color definitions, emoji constants, and shared formatting helpers from the Go render module. The Go CLI defines colors in `render/deployments.go:19-34` (nsBg=Yellow, chainBg=Cyan, etc.) and helper functions in `render/helpers.go` (FormatWarning, FormatError, FormatSuccess). The Rust `ui/color.rs` has a different palette that must be replaced.

**Go source files to translate:**
- `internal/cli/render/deployments.go` lines 19-34 (color palette: `nsBg`, `chainBg`, `nsHeader`, `chainHeader`, `addressStyle`, `timestampStyle`, `pendingStyle`, `tagsStyle`, `verifiedStyle`, `notVerifiedStyle`, `sectionHeaderStyle`, `implPrefixStyle`, `forkIndicatorStyle`)
- `internal/cli/render/helpers.go` (FormatWarning `yellow + "warning"`, FormatError `red + "error"`, FormatSuccess `green + "success"`)
- `internal/cli/render/fork.go` lines 84-92 (`formatDuration` — converts duration to `Xh Ym Zs` human-readable)
- `internal/cli/version.go` lines 39-52 (`formatBuildDate` — ISO 8601 to `YYYY-MM-DD HH:MM:SS UTC`)
- All emoji constants used across render files (see full list in discovery)

**Rust files to modify:**
- `crates/treb-cli/src/ui/color.rs` — replace palette with exact Go colors
- `crates/treb-cli/src/output.rs` — update `format_stage`, `format_warning_banner` to match Go format strings
- `crates/treb-cli/src/ui/badge.rs` — update verification badge characters to match Go (`e[checkmark]`, `s[-]`, `b[hourglass]`)

**Deliverables**
- Color palette in `ui/color.rs` matching Go exactly: Yellow bg for namespace, Cyan bg for chain, per-type colors (Magenta=proxy, Blue=library, Green=singleton)
- Emoji constant module with all emoji used in Go render files
- `format_warning()`, `format_error()`, `format_success()` returning exact Go format strings
- `format_duration()` matching Go `formatDuration` output (Xh Ym Zs)
- `format_build_date()` matching Go `formatBuildDate` output
- Verification badge format matching Go: `e[checkmark]`, `s[-]`, `b[hourglass]` with per-status colors
- Golden file updates for any commands affected by color/format changes

**User stories:** 6
**Dependencies:** none

---

## Phase 2 -- Tree Renderer and Table Formatter Alignment

Align the tree rendering and table formatting to match Go exactly. The Go CLI uses `go-pretty/v6/table` with `StyleLight`, no borders, 3-space right padding, and fixed column widths calculated globally. Tree connectors use Unicode box-drawing characters. The Rust `ui/tree.rs` TreeNode must produce identical output, and `output.rs` table builder must match Go's go-pretty configuration.

**Go source files to translate:**
- `internal/cli/render/deployments.go` lines 465-510 (`renderTableWithWidths` — go-pretty table with StyleLight, PaddingRight 3 spaces, no borders, no row/column separators)
- `internal/cli/render/deployments.go` lines 420-460 (`calculateTableColumnWidths` — global max width per column across all tables)
- `internal/cli/render/deployments.go` lines 514+ (`stripAnsiCodes` — strip ANSI for width calculation)
- Tree connector constants used throughout: `|--` (mid), `\--` (last), `|   ` (continuation), `    ` (space)

**Rust files to modify:**
- `crates/treb-cli/src/ui/tree.rs` — update `render()` and `render_styled()` to use exact Go connector characters and indentation
- `crates/treb-cli/src/output.rs` — update `build_table()` to match go-pretty StyleLight configuration (no borders, 3-space padding, fixed widths)
- New: ANSI-stripping utility for column width calculation

**Deliverables**
- Tree renderer producing identical connector characters and indentation as Go
- Table builder configured to match go-pretty StyleLight (no borders, 3-space right padding)
- Global column width calculation across grouped tables (matching `calculateTableColumnWidths`)
- ANSI code stripping for accurate width measurement
- Namespace marker: `   circle namespace: UPPERCASE` with Yellow bg
- Chain marker: `|-- chain-emoji chain: NETWORK (CHAINID)` or `\--` with Cyan bg
- Golden file updates

**User stories:** 5
**Dependencies:** Phase 1

**Learnings from implementation:**
- Unicode box-drawing chars (├, └, │) are 1 display column each but 3 bytes in UTF-8 — connector width reduced from 4 to 3 columns per level, matching Go
- `display_width()` in `ui/terminal.rs` no longer strips ANSI — callers must call `strip_ansi_codes()` first (two-step pattern)
- `unicode-width 0.2` and go-runewidth agree on widths for common symbols: ✔︎=1, ⏳=2, CJK=2; U+FE0E (text variation selector) has width 0
- `calculate_column_widths` uses `display_width` for col 2 only; all other columns use byte length after ANSI stripping (matching Go behavior)
- First column width adjustment formula: `widths[0] + 2 + continuation_prefix.len()` where `.len()` is byte count (matching Go `len()`)
- New Go-matching table renderer (`TableData`, `calculate_column_widths`, `render_table_with_widths`) added to `output.rs` — coexists with `build_table()` (comfy_table) until all commands are ported
- comfy_table `ContentArrangement::Dynamic` does NOT strip ANSI codes before measuring column widths — styled cells cause inflated column widths in golden files
- `is_color_enabled()` returns `true` in subprocess golden tests (checks `NO_COLOR`/`TERM=dumb` only, not TTY) — so `styled()` always adds ANSI codes in golden output
- `UPDATE_GOLDEN=1` regenerates ALL golden files in the test binary, not just failing ones — use `--test <binary>` to target specific test binaries and always verify the diff
- `clippy::cloned_ref_to_slice_refs` lint: use `std::slice::from_ref(&table)` instead of `&[table.clone()]`

---

## Phase 3 -- version, networks, init, and config Output

Port the exact output format for four simple commands. These use direct `fmt.Printf` or minimal render modules.

**Go source files to translate:**
- `internal/cli/version.go` lines 11-37 — format: `treb VERSION\n` + optional commit (7 chars) + formatted build date
- `internal/cli/render/networks.go` lines 10-44 — format: header `globe-emoji Available Networks:`, per-network `  checkmark NETWORK - Chain ID: CHAINID\n` or `  cross-mark NETWORK - Error: MSG\n`
- `internal/cli/render/init.go` lines 14-82 — per-step green checkmark/red cross-mark, success message with party-emoji bold, next steps section with numbered instructions and cyan bold header
- `internal/cli/render/config.go` lines 40-91 — show: cyan bold header + emoji fields (clipboard-emoji, package-emoji, folder-emoji) + relative path; set: `checkmark Set KEY to: VALUE\n` + file path; remove: `checkmark Reset/Removed...\n` + file path

**Rust files to modify:**
- `crates/treb-cli/src/commands/version.rs` — replace `print_kv()` with exact Go format
- `crates/treb-cli/src/commands/networks.rs` — replace table with Go emoji-per-line format
- `crates/treb-cli/src/commands/init.rs` — add emoji, per-step status, next steps section
- `crates/treb-cli/src/commands/config.rs` — add emoji fields, relative path display, colored headers

**Deliverables**
- `version` output: `treb VERSION\n` + commit + date matching Go format exactly
- `networks` output: emoji-prefixed per-network lines (not a table)
- `init` output: per-step checkmark/cross-mark + party-emoji success + numbered next steps
- `config show` output: cyan header + emoji-prefixed fields + senders section
- `config set` output: `checkmark Set KEY to: VALUE\n` + config file path
- `config remove` output: `checkmark Reset/Removed...\n` + config file path
- Golden file updates for all four commands

**User stories:** 6
**Dependencies:** Phase 1

**Learnings from implementation:**
- Emoji constants live in `crate::ui::emoji` (e.g., `emoji::GLOBE`, `emoji::CHECK`, `emoji::CROSS`) — import from there for all future phases
- Color style constants: `color::SUCCESS` (green bold) for banners, `color::STAGE` (cyan bold) for headers, `color::WARNING` (yellow), `color::GRAY` (bright_black) for dim/faint text like command examples
- Reusable format helpers in `output.rs`: `format_success(msg)` → `✅ {msg}` green, `format_warning(msg)` → `⚠️  {msg}` yellow, `format_error(msg)` → `❌ {msg}` red — use these in all future phases
- For relative path display: `path.strip_prefix(&cwd).unwrap_or_else(|_| path.as_path())` — `strip_prefix()` returns `Result`, needs fallback
- `cwd` ownership gotcha: `cwd` is moved into `ResolveOpts { project_root: cwd }` — clone it first if you need it later for relative path computation
- JSON output is unchanged when porting human output — `--json` golden files don't need updates unless the JSON schema itself changes
- When replacing `print_kv()`/`print_table()` with direct emoji-formatted output, the backing data structs (e.g., `VersionInfo`) stay unchanged since JSON still uses all fields
- Pre-existing fork golden file failures (table column width drift) were fixed via `UPDATE_GOLDEN=1` — fork golden files now have a clean baseline for Phase 9
- `UPDATE_GOLDEN=1 cargo test -p treb-cli --test <test_binary> -- <test_names>` regenerates specific golden files without touching unrelated ones
- When table formatting changes globally (e.g., comfy_table version bump), all table-based golden files may need `UPDATE_GOLDEN=1` refresh

---

## Phase 4 -- list Command Output

Port the most complex rendering in the CLI: the deployment list tree. The Go `render/deployments.go` groups deployments by namespace, then chain, then type (PROXIES, IMPLEMENTATIONS, SINGLETONS, LIBRARIES) with tree connectors, proxy-implementation relationship rows, verification badges, fork indicators, and tags. This is the signature output of the CLI.

**Go source files to translate:**
- `internal/cli/render/deployments.go` lines 53-67 (`RenderDeploymentList` — entry point, iterates namespaces)
- `internal/cli/render/deployments.go` lines 69-100 (namespace discovery hint — shows other namespaces with counts when current namespace is empty)
- `internal/cli/render/deployments.go` lines 103-310 (per-namespace rendering: chain grouping, type sections with bold white headers, deployment table rows)
- `internal/cli/render/deployments.go` lines 312-450 (deployment row formatting: contract name colored by type, address, verification `e[checkmark] s[-] b[hourglass]`, timestamp, fork `[fork]` badge, tags `(tag1)` in cyan, implementation rows `\-- impl_address`)
- `internal/cli/list.go` lines 128-162 (JSON output structure: `{"deployments": [...], "otherNamespaces": {...}}`)
- Footer line: `Total deployments: N`

**Rust files to modify:**
- `crates/treb-cli/src/commands/list.rs` — rewrite `group_deployments()`, `format_deployment_entry()`, `build_deployment_node()` and display logic to match Go rendering exactly

**Deliverables**
- Namespace headers: `   circle-marker namespace: UPPERCASE` with Yellow bg, Black text
- Chain headers: `|-- chain-emoji chain: NETWORK (CHAINID)` with Cyan bg, Black text (or `\--` for last)
- Type section headers: `PROXIES`, `IMPLEMENTATIONS`, `SINGLETONS`, `LIBRARIES` in bold white
- Deployment table rows: Contract (type-colored), Address, Verification badges, Timestamp
- Implementation rows: `\-- impl_address` with faint prefix
- Fork indicator: `[fork]` in yellow on fork-added deployments
- Tags display: `(tag1)` in cyan after contract name
- Namespace discovery hint when no deployments in current namespace
- Footer: `Total deployments: N`
- JSON output schema: `{"deployments": [...], "otherNamespaces": {...}}`
- Golden file updates for all list variants

**Notes from Phase 2:**
- Use the new `render_table_with_widths()` and `calculate_column_widths()` from `output.rs` (not `build_table()` / comfy_table) for deployment table rows — these match Go go-pretty output exactly
- Tree connectors now use Unicode box-drawing (├─, └─, │) with 3-column indent per level
- Styled cells always include ANSI in golden tests — `calculate_column_widths` handles ANSI stripping internally
- Target golden tests with `--test cli_list_show` to avoid updating unrelated golden files

**User stories:** 8
**Dependencies:** Phase 2

**Learnings from implementation:**
- `DisplayCategory` enum (Proxy, Implementation, Singleton, Library) separates display grouping from domain `DeploymentType` — display-specific enums prevent coupling display logic to domain types
- `ListResult` struct centralizes all display context (deployments, other_namespaces, network_names, fork_deployment_ids) — complex commands benefit from a single result struct holding all render context
- Two-pass table rendering pattern: first pass collects `TableData` per type group, `calculate_column_widths()` computes global widths, second pass renders with `render_table_with_widths()` — this is the canonical pattern for Go-matching table output
- `ImplNameLookup` maps `(namespace_lower, chain_id, address_lower)` → contract name for proxy→implementation resolution; address matching is case-insensitive (lowercase comparison)
- When `--type` filter excludes implementations from the deployment list, `ImplNameLookup` won't find them — impl row falls back to truncated address; this matches Go behavior
- JSON schema changes ripple across many test files: when wrapping output (e.g., `{"deployments": [...]}` instead of raw array), search for `as_array()` and command-specific `--json` patterns across ALL test files including `e2e/mod.rs` helpers
- `serde`'s `skip_serializing_if` needs a custom `is_false` function for `bool` fields — no built-in `"is_false"` predicate
- `print_json()` sorts keys recursively, so Go field order is handled automatically by alphabetical sorting — no need to control struct field ordering
- Namespace discovery hint uses `{:<20}` (Go's `%-20s`) for namespace alignment; `build_other_namespaces` uses case-insensitive exclusion matching with `eq_ignore_ascii_case`
- ANSI codes appear between tree prefixes and labels — test assertions should not span ANSI boundaries; use partial patterns like `"\n│ \n└─"` instead of full styled label strings
- Fork deployment detection uses `namespace.starts_with("fork/")` — consistent across the codebase
- When golden files are updated incrementally across multiple stories, the final "update all golden files" story may be a no-op verification — still valuable for consistency confirmation
- `network_names` field currently empty (no centralized chain_id→name mapping) — future phases needing network names must populate from config or foundry.toml RPC endpoint keys

---

## Phase 5 -- show Command Output

Port the detailed deployment view from Go `render/deployment.go`. The Go version uses an 80-char equals-sign divider, labeled sections (Basic Information, Deployment Strategy, Proxy Information, etc.), and per-field coloring.

**Go source files to translate:**
- `internal/cli/render/deployment.go` lines 26-200+ (`RenderDeployment` — header with `Deployment: ID` + optional `[fork]`, 80-char `=` divider, sections)
- Sections rendered:
  - `Basic Information:` — Contract (yellow), Address, Type, Namespace, Network, Label (magenta)
  - `Deployment Strategy:` — Method, Factory, Salt, Entropy, InitCodeHash
  - `Proxy Information:` — Type, Implementation (cyan), Admin, Upgrade History
  - `Artifact Information:` — Path, Compiler, BytecodeHash, ScriptPath, GitCommit
  - `Verification Status:` — Status (green/red), URLs, VerifiedAt, per-verifier details
  - `Transaction Information:` — Hash, Status, Sender, Block, Safe context
- `internal/cli/show.go` lines 12-102 (command handler, JSON format: `{"deployment": {...}, "fork": bool}`)

**Rust files to modify:**
- `crates/treb-cli/src/commands/show.rs` — replace section headers (`-- Title --` format) with Go format, add 80-char divider, match field ordering and coloring

**Deliverables**
- Header: `Deployment: ID` + optional `[fork]` badge
- 80-character `=` divider line
- Section headers matching Go: `Basic Information:`, `Deployment Strategy:`, etc.
- Per-field coloring: Contract yellow, Label magenta, Implementation cyan, Status green/red
- Upgrade History sub-section for proxies
- Safe context display in Transaction section
- JSON output: `{"deployment": {...}, "fork": bool}`
- Golden file updates

**User stories:** 6
**Dependencies:** Phase 1

**Notes from Phase 3:**
- Use `output::format_success()`, `format_warning()`, `format_error()` for status lines — these match Go format strings exactly
- Color styles: `color::SUCCESS` for banners, `color::STAGE` for section headers (cyan bold), `color::GRAY` for dim text
- Emoji imports from `crate::ui::emoji` — all Go emoji constants are available there

**Learnings from implementation:**
- Show command uses local helpers: `print_field(key, value)` for `  Key: Value` 2-space indent, `print_section(title)` for `\nTitle:` headers, `print_deployment_header(id, fork_badge)` for cyan bold header + 80-char `=` divider — reuse this pattern for other section-based commands (e.g., tag show, register)
- `styled(text, style)` helper in show.rs conditionally applies owo-colors Style based on `color::is_color_enabled()` — useful pattern for inline conditional coloring
- `output::print_kv()` is no longer used by show.rs after porting; marked `#[allow(dead_code)]` but kept as public utility — future phases may also stop using it
- `contract_display_name(name, label)` already exported from `treb_core::types` — returns `Name:label` format, no need to reimplement
- No magenta-only color constant in `color.rs` — use `Style::new().magenta()` inline for label coloring
- DeploymentStrategy has `entropy` and `init_code_hash` fields with `skip_serializing_if = "String::is_empty"` — conditionally display only when non-empty
- Zero-hash check for Salt field: skip when empty OR equals `0x000...000` (66 chars total)
- All human-readable timestamps must use `.format("%Y-%m-%d %H:%M:%S")` for Go parity — never use `to_rfc3339()` in human output
- `VerificationInfo.status` is a `VerificationStatus` enum (Verified/Unverified/Failed/Partial) — use enum matching, not string comparison
- `VerificationInfo.etherscan_url` is a top-level field on the verification struct, separate from per-verifier data
- Go shows proxy Implementation plain (no color) when unresolved — only uses colored styling when resolved via linked runtime field
- `serde_json::json!` macro + `serde_json::Value` implements `Serialize`, works directly with `output::print_json(&wrapper)` for JSON wrapper objects
- Tests using `stdout.contains()` with styled output must set `NO_COLOR=1` env var — nested `styled()` calls insert ANSI escape codes that break plain-text matching
- `helpers::seed_registry(ctx.path())` seeds from `treb-core/tests/fixtures/deployments_map.json` — useful for show/list tests

---

## Phase 6 -- run Command Output

Port the run command execution output from Go. The Go version uses a `ScriptRenderer` with staged output: transaction list with decoded function calls, deployment summary, registry update confirmations, and a final success message.

**Go source files to translate:**
- `internal/cli/render/script.go` lines 25-200+ (`ScriptRenderer.RenderExecution` — transactions section with bold header, gray separator, per-tx status/sender/function, deployment summary, registry updates)
- `internal/cli/render/transaction.go` lines 1-200+ (transaction tree rendering with trace data, decoded function signatures)
- `internal/cli/run.go` lines 111-132 (command handler output: green success message `checkmark Script execution completed successfully`)
- Format elements:
  - `refresh-emoji Transactions:` (bold header)
  - Gray separator: `---...`
  - Per transaction: Status icon (colored), Sender (green), decoded function call
  - `Deployment Summary:` section
  - Registry update: green checkmark or yellow dash
  - Final: `checkmark Script execution completed successfully` (green)

**Rust files to modify:**
- `crates/treb-cli/src/commands/run.rs` — update `display_result_human()` (lines 746-878) to match Go ScriptRenderer format exactly, update verbose mode output (lines 333-350), update debug mode

**Deliverables**
- Transaction list with bold header, gray separator line, per-tx decoded output
- Deployment summary section matching Go format
- Registry update confirmations (green checkmark per deployment)
- Console.log output block matching Go format
- `[DRY RUN]` banner matching Go format
- Verbose pre-execution context matching Go format
- Final success message: `checkmark Script execution completed successfully`
- Debug output file format matching Go
- Golden file updates for run variants

**Notes from Phase 2:**
- Use `render_table_with_widths()` and `calculate_column_widths()` from `output.rs` for any tabular deployment/transaction output — these match Go go-pretty exactly
- Remember `display_width()` does not strip ANSI; the table renderer handles this internally via `strip_ansi_codes()`

**User stories:** 7
**Dependencies:** Phase 2

**Notes from Phase 4:**
- Two-pass table rendering is the canonical pattern: collect all `TableData`, call `calculate_column_widths()` for global widths, then `render_table_with_widths()` — proven in Phase 4 list command
- Continuation prefix after headers ("│ " or "  ") works correctly with `render_table_with_widths` despite different byte lengths (│ is 3 bytes but 1 display char)

---

## Phase 7 -- verify Command Output

Port verification output from Go `render/verify.go`. The Go version shows skipped contracts, per-verifier results with status icons, and a summary line.

**Go source files to translate:**
- `internal/cli/render/verify.go` lines 1-150+ (`NewVerifyRenderer` + `RenderVerifyAllResult` + `RenderVerifyResult`)
- Format elements:
  - Skipped header (cyan bold): `Skipping %d pending/undeployed contracts:\n`
  - Skipped line: `  skip-emoji  chain:CHAINID/NS/CONTRACT (STATUS)\n`
  - Contracts to verify (cyan bold): `Found %d unverified/deployed contracts to verify:\n`
  - Per result: checkmark or cross-mark icon, location, verification details (green/red)
  - Summary: `Verification complete: X/Y successful\n`
  - Already verified: Yellow message with `--force` suggestion
- `internal/cli/verify.go` lines 13-140 (command handler)

**Rust files to modify:**
- `crates/treb-cli/src/commands/verify.rs` — update single and batch display to match Go render/verify.go format

**Deliverables**
- Skipped contracts section with cyan bold header and skip-emoji per line
- Contracts-to-verify section with cyan bold count header
- Per-verifier result display: status icon + location + details (green/red)
- Summary line: `Verification complete: X/Y successful`
- Already-verified message with `--force` suggestion in yellow
- Batch mode progress and results matching Go
- Golden file updates

**User stories:** 5
**Dependencies:** Phase 1

**Notes from Phase 5:**
- `VerificationInfo.status` is a `VerificationStatus` enum (Verified/Unverified/Failed/Partial) — use enum matching, not string comparison
- Go uses overall `verification.status` + `etherscan_url`, not per-verifier breakdown — simpler display than Rust's current per-verifier approach
- Color constants: `color::VERIFIED` (green), `color::NOT_VERIFIED` (red), `color::FAILED` (red), `color::UNVERIFIED` (dimmed)

---

## Phase 8 -- compose Command Output

Port compose orchestration output from Go `render/compose.go`. The Go version shows an execution plan, per-component status with emoji, and a summary with separator.

**Go source files to translate:**
- `internal/cli/render/compose.go` lines 1-117 (`NewComposeRenderer` + `RenderComposeResult`)
- Format elements:
  - Plan header (bold cyan): `target-emoji Orchestrating %s\n` + `clipboard-emoji Execution plan: %d components\n`
  - Per component: `N. ComponentName -> ScriptPath (depends on: [deps])`
  - Env vars (yellow): `Env: {...}`
  - Step result: Green checkmark or Red cross-mark with deployment count
  - Summary (bold): `===...` separator, success/failure message, stats
- `internal/cli/compose.go` lines 13-140 (command handler)

**Rust files to modify:**
- `crates/treb-cli/src/commands/compose.rs` — update display logic to match Go render/compose.go format

**Deliverables**
- Plan header with target-emoji and clipboard-emoji, bold cyan styling
- Numbered component list with script path and dependency display
- Env vars display in yellow
- Per-step checkmark/cross-mark result with deployment count
- Summary with `===` separator line, success/failure message, stats
- Golden file updates

**User stories:** 5
**Dependencies:** Phase 6

---

## Phase 9 -- fork Commands Output

Port all seven fork subcommand outputs from Go `render/fork.go`. Each subcommand has a distinct format with indented key-value pairs, tree markers, and status indicators.

**Go source files to translate:**
- `internal/cli/render/fork.go` lines 18-38 (`RenderEnter` — message + indented fields: Network, Chain ID, Fork URL, Anvil PID, Env Override, Logs, Setup status + footer help lines)
- `internal/cli/render/fork.go` lines 40-48 (`RenderExit` — message + per-network `  - NETWORK: registry restored, fork cleaned up\n`)
- `internal/cli/render/fork.go` lines 50-81 (`RenderStatus` — `Active Forks` header, per entry: Network + optional `(current)`, indented fields: ChainID, ForkURL, AnvilPID, Status, Uptime `formatDuration`, Snapshots, ForkDeploys, Logs)
- `internal/cli/render/fork.go` lines 94-104 (`RenderRevert` — message + `  Reverted: COMMAND\n` + `  Remaining: N snapshot(s)\n`)
- `internal/cli/render/fork.go` lines 106-125 (`RenderRestart` — similar to Enter + "Registry restored to initial fork state...")
- `internal/cli/render/fork.go` lines 127-150 (`RenderHistory` — `Fork History: NETWORK\n` + per entry: `  [MARKER] [INDEX] LABEL (TIMESTAMP)` where marker is `-> ` for current or `   ` for others, label is "initial" or command name)
- `internal/cli/render/fork.go` lines 152-184 (`RenderDiff` — `Fork Diff: NETWORK\n` + "No changes" or sections: New Deployments `  + %-20s ADDRESS TYPE\n`, Modified Deployments `  ~ %-20s ADDRESS TYPE\n`, New Transactions count)
- `internal/cli/fork.go` lines 14-419 (command handler for all subcommands)

**Rust files to modify:**
- `crates/treb-cli/src/commands/fork.rs` — update all seven subcommand display sections to match Go render/fork.go format exactly

**Deliverables**
- `fork enter`: indented field list (Network, Chain ID, Fork URL, Anvil PID, Env Override, Logs, Setup) + footer
- `fork exit`: per-network cleanup confirmation line
- `fork status`: `Active Forks` header + per-entry indented fields with formatted duration
- `fork revert`: reverted command + remaining snapshots count
- `fork restart`: enter-like output + registry restore message
- `fork history`: `Fork History: NETWORK` + arrow-marked entries with index and timestamp
- `fork diff`: `Fork Diff: NETWORK` + `+` new / `~` modified deployments + transaction count
- Golden file updates for all fork variants

**Notes from Phase 2:**
- Fork golden files (`fork_diff_with_changes`, `fork_history_network_filter`, `fork_history_with_entries`, `fork_status_with_active_fork`) currently use comfy_table with ANSI-inflated column widths — these were pre-existing failures also present on main
- When porting fork table output, replace `build_table()` (comfy_table) with `render_table_with_widths()` to eliminate ANSI width inflation and match Go output
- Use `--test integration_fork` to target only fork golden tests

**Notes from Phase 3:**
- Pre-existing fork golden file failures (table column width drift) were fixed in Phase 3 via `UPDATE_GOLDEN=1` — fork golden files now start from a clean baseline
- Use `output::format_success()` for checkmark lines, `color::STAGE` (cyan bold) for section headers, `color::GRAY` for dim text
- Emoji imports from `crate::ui::emoji` — all Go emoji constants are available there
- For relative path display: `path.strip_prefix(&cwd).unwrap_or_else(|_| path.as_path())`

**Notes from Phase 5:**
- All human-readable timestamps must use `.format("%Y-%m-%d %H:%M:%S")` for Go parity — never use `to_rfc3339()` in human output
- Tests checking styled output with `contains()` must set `NO_COLOR=1` or strip ANSI codes — nested `styled()` calls break plain-text matching

**User stories:** 7
**Dependencies:** Phase 1

---

## Phase 10 -- sync and tag Command Output

Port sync and tag rendering from Go. Sync uses a multi-section format with metrics and warnings. Tag uses deployment display with tag list management.

**Go source files to translate:**
- `internal/cli/render/sync.go` lines 23-70 (`RenderSyncResult` — `Syncing registry...\n` header, `Safe Transactions:` section with cyan bold + bullet metrics: Checked, Executed (green), Transactions updated, Deployments updated; Cleanup section with removed count; Warnings in yellow; Footer: green checkmark `Registry synced successfully` or `completed with warnings`)
- `internal/cli/sync.go` lines 11-64 (command handler)
- `internal/cli/render/tag.go` lines 23-145 (`Render` — show: cyan bold `Deployment: NS/CHAINID/NAME\n` + white bold `Address: ADDR\n` + `Tags:` with cyan tags comma-separated or "No tags" faint; add: green `checkmark Added tag 'TAG' to ID\n` + `Current tags:` list; remove: green `checkmark Removed tag 'TAG' from ID\n` + `Remaining tags:` list or "No tags")
- `internal/cli/tag.go` lines 18-98 (command handler)

**Rust files to modify:**
- `crates/treb-cli/src/commands/sync.rs` — update display to match Go render/sync.go section format
- `crates/treb-cli/src/commands/tag.rs` — update show/add/remove display to match Go render/tag.go format

**Deliverables**
- `sync`: `Syncing registry...\n` header, `Safe Transactions:` cyan bold section, bullet metrics, Cleanup section, Warnings in yellow, footer with green checkmark
- `tag show`: cyan bold deployment header + white bold address + `Tags:` with cyan list or faint "No tags"
- `tag add`: green checkmark + `Current tags:` list
- `tag remove`: green checkmark + `Remaining tags:` list or "No tags"
- Golden file updates

**User stories:** 6
**Dependencies:** Phase 1

**Notes from Phase 3:**
- Use `output::format_success()`, `format_warning()` for status lines — these match Go format strings exactly
- Color styles: `color::SUCCESS` (green bold) for banners, `color::STAGE` (cyan bold) for section headers, `color::WARNING` (yellow), `color::GRAY` for dim text
- Emoji imports from `crate::ui::emoji`

**Notes from Phase 4:**
- In the list command, only the first tag is shown as `(tag1)` in cyan after contract name — verify tag display in `tag show` uses a different format (comma-separated, all tags)

**Notes from Phase 5:**
- Show command's `print_field(key, value)` / `print_section(title)` helpers are a useful pattern for section-based output — consider reusing or extracting to shared output module if tag show needs similar formatting
- `contract_display_name(name, label)` from `treb_core::types` can be reused for tag show's deployment display

---

## Phase 11 -- register, prune, and reset Command Output

Port rendering for three registry management commands. Register uses interactive prompts with contract selection. Prune uses staged emoji output with dry-run preview. Reset uses summary counts with confirmation.

**Go source files to translate:**
- `internal/cli/register.go` lines 384-397 (register output: cyan bold per-contract header `\nContract N of N: ADDRESS (Kind)\n`, success bold green `checkmark Successfully registered N deployment(s)\n\n`, per deployment indented: `  Deployment N:\n`, `    Deployment ID: ID\n`, `    Address: ADDR\n`, `    Contract: NAME\n`, `    Label: LABEL\n`)
- `internal/cli/render/prune.go` lines 22-78 (`RenderItemsToPrune` — `magnifying-glass Checking registry entries...\n`, nothing to prune: green checkmark `All registry entries are valid.\n`, items found: `wastebasket Found N items to prune:\n\n`, per section (Deployments, Transactions, SafeTransactions): `SectionName (N):\n` + `  - ID DETAILS [STATUS] (reason: REASON)\n`, confirmation: `warning Are you sure...? [y/N]: `, prune start: `wrench Pruning registry entries...\n`, success: `checkmark Successfully pruned N items.\n`)
- `internal/cli/reset.go` lines 50-103 (reset output: `Found N items to reset for namespace 'NS' on network 'NET' (chain CHAINID):\n\n` + per type counts: `  Deployments:        N\n`, `  Transactions:       N\n`, `  Safe Transactions:  N\n`, confirmation, success: `Successfully reset N items.\n`)

**Rust files to modify:**
- `crates/treb-cli/src/commands/register.rs` — update output to match Go per-deployment format
- `crates/treb-cli/src/commands/prune.rs` — update staged output to match Go render/prune.go
- `crates/treb-cli/src/commands/reset.rs` — update summary and confirmation to match Go format

**Deliverables**
- `register`: cyan bold per-contract header, green bold success with count, per-deployment indented details
- `prune`: magnifying-glass checking header, green checkmark if clean, wastebasket with count, per-section items with reason, wrench pruning, checkmark success
- `reset`: `Found N items...` with namespace/network/chain, per-type count lines with aligned spacing, confirmation prompt, success count
- Golden file updates

**User stories:** 6
**Dependencies:** Phase 1

**Notes from Phase 3:**
- Use `output::format_success()`, `format_warning()`, `format_error()` for status lines
- Emoji imports from `crate::ui::emoji`; color styles: `color::SUCCESS`, `color::STAGE`, `color::WARNING`, `color::GRAY`

---

## Phase 12 -- gen-deploy and migrate Command Output

Port gen-deploy and migrate output from Go. Gen-deploy has minimal output (success + instructions). Migrate has extensive interactive output with account naming, namespace pruning, preview, and cleanup steps.

**Go source files to translate:**
- `internal/cli/render/generate.go` lines 16-23 (`Render` — `\ncheckmark Generated deployment script: PATH\n` + instructions list)
- `internal/cli/generate.go` lines 29-115 (command handler)
- `internal/cli/migrate.go` lines 19-590 (migrate output: profile extraction with count, interactive naming `Found N deduplicated account(s). Name each one:\n` + per account `  SUMMARY -- name [DEFAULT]`, namespace pruning prompts, yellow warning if treb.toml exists, content preview display, confirm write `Write this to treb.toml? [y/N]:`, green bold checkmark `treb.toml written successfully\n`, cleanup offer `Remove [profile.*.treb.*] sections from foundry.toml?`, green bold checkmark `foundry.toml cleaned up\n`, numbered next steps list)

**Rust files to modify:**
- `crates/treb-cli/src/commands/gen_deploy.rs` — update success output to match Go format
- `crates/treb-cli/src/commands/migrate.rs` — update interactive flow output to match Go format

**Deliverables**
- `gen-deploy`: `\ncheckmark Generated deployment script: PATH\n` + instructions
- `migrate`: profile extraction count, interactive account naming with defaults, namespace pruning, yellow warning, content preview, write confirmation, green success, cleanup offer, numbered next steps
- Golden file updates

**User stories:** 5
**Dependencies:** Phase 1

**Notes from Phase 3:**
- Use `output::format_success()`, `format_warning()` for status lines
- Emoji imports from `crate::ui::emoji`; color styles: `color::SUCCESS`, `color::STAGE`, `color::WARNING`, `color::GRAY`
- For relative path display: `path.strip_prefix(&cwd).unwrap_or_else(|_| path.as_path())`

---

## Phase 13 -- dev anvil Command Output

Port dev anvil subcommand output from Go `render/anvil.go`. Each subcommand (start, stop, restart, status, logs) has distinct formatting with emoji and status indicators.

**Go source files to translate:**
- `internal/cli/render/anvil.go` lines 35-49 (`renderStart` — green `checkmark MESSAGE\n`, yellow `clipboard-emoji Logs: PATH\n`, blue `globe-emoji RPC URL: URL\n`, CreateX: green checkmark or red warning + factory address)
- `internal/cli/render/anvil.go` lines 51-57 (`renderStop` — green `checkmark MESSAGE\n`)
- `internal/cli/render/anvil.go` lines 59-62 (`renderRestart` — calls renderStart)
- `internal/cli/render/anvil.go` lines 64-91 (`renderStatus` — cyan bold `chart-emoji Anvil Status ('NAME'):\n`, running: green `Status: green-circle Running (PID N)\n` + blue `RPC URL: URL\n` + yellow `Log file: PATH\n` + RPC Health checkmark/cross-mark + CreateX checkmark/cross-mark; not running: red `Status: red-circle Not running\n` + gray PID file + gray Log file)
- `internal/cli/render/anvil.go` lines 93-98 (`RenderLogsHeader` — cyan bold `clipboard-emoji Showing anvil 'NAME' logs (Ctrl+C to exit):\n` + gray `Log file: PATH\n\n`)
- `internal/cli/dev.go` lines 11-178 (command handler)

**Rust files to modify:**
- `crates/treb-cli/src/commands/dev.rs` — update all anvil subcommand output to match Go render/anvil.go format

**Deliverables**
- `dev anvil start`: green checkmark message, yellow clipboard-emoji logs path, blue globe-emoji RPC URL, CreateX status
- `dev anvil stop`: green checkmark message
- `dev anvil restart`: same as start format
- `dev anvil status`: cyan bold chart-emoji header, running status with green/red circle indicators, RPC health, CreateX status
- `dev anvil logs`: cyan bold clipboard-emoji header with Ctrl+C instruction
- Golden file updates

**User stories:** 5
**Dependencies:** Phase 1

**Notes from Phase 3:**
- Use `output::format_success()` for checkmark lines, `color::STAGE` (cyan bold) for section headers
- Emoji imports from `crate::ui::emoji`; color styles: `color::SUCCESS`, `color::STAGE`, `color::GRAY`
- For relative path display: `path.strip_prefix(&cwd).unwrap_or_else(|_| path.as_path())`

---

## Phase 14 -- JSON Output Schema Parity and Command Grouping

Audit every command's `--json` output to ensure schema-identical output with Go. Also port the command grouping from Go `root.go` (Main Commands vs Management Commands groups) and verify `--help` descriptions match.

**Go source files to translate:**
- `internal/cli/list.go` lines 128-162 (JSON: `{"deployments": [...], "otherNamespaces": {...}}`)
- `internal/cli/show.go` (JSON: `{"deployment": {...}, "fork": bool}`)
- `internal/cli/register.go` (JSON: `{"deployments": [{deploymentId, address, contractName, label}]}`)
- `internal/cli/root.go` (command groups: Main Commands = init, list, show, gen, run, verify, compose, fork; Management Commands = sync, tag, register, networks, prune, reset, config, migrate, dev)
- All commands: verify `--json` field names match Go (camelCase), field ordering, null vs omit behavior

**Rust files to modify:**
- `crates/treb-cli/src/main.rs` — update clap command groups to match Go
- All command files — audit and fix JSON output structs for schema parity

**Deliverables**
- JSON schema parity for all commands verified against Go output
- Command grouping: "Main Commands" and "Management Commands" matching Go
- `--help` text for each command matching Go descriptions
- Field ordering in JSON output matching Go (alphabetical via sorted keys or insertion order)
- Null vs omit behavior matching Go (`omitempty` equivalent in serde)
- Golden file updates for all JSON output variants

**User stories:** 6
**Dependencies:** Phases 3-13

**Notes from Phase 3:**
- Confirmed that porting human output does NOT change JSON output — `--json` golden files are unaffected unless the JSON schema itself changes
- Data structs (e.g., `VersionInfo`) stay unchanged even when human display format changes completely

**Notes from Phase 4:**
- `list --json` already ported to Go schema: `{"deployments": [...], "otherNamespaces": {...}}` wrapper with `ListJsonEntry` (8 Go fields) and `ListJsonOutput` — no audit needed for list
- `print_json()` sorts keys recursively so field ordering is automatically alphabetical — verify other commands also use `print_json()` for consistency
- When auditing JSON schema changes, search for `as_array()` and command-specific `--json` patterns across all test files including `e2e/mod.rs` helpers — JSON schema changes break shared helpers silently
- `serde` `skip_serializing_if` with custom `is_false` function is the pattern for omitting false booleans (Go `omitempty` equivalent for bool)

**Notes from Phase 5:**
- `show --json` already ported to Go schema: `{"deployment": {...}}` wrapper with conditional `"fork": true` field (absent for non-fork) — no audit needed for show
- `serde_json::json!` macro works with `output::print_json()` for constructing wrapper objects with conditional fields — proven pattern for JSON wrappers
- Fork detection in JSON uses `namespace.starts_with("fork/")` — `"fork"` field is absent (not `false`) for non-fork deployments, matching Go `omitempty` behavior

---

## Dependency Graph (ASCII)

```
Phase 1 (colors/emoji/helpers) ──┬──> Phase 2 (tree/table) ──┬──> Phase 4 (list)
                                 │                            └──> Phase 6 (run) ──> Phase 8 (compose)
                                 │
                                 ├──> Phase 3 (version/networks/init/config)
                                 ├──> Phase 5 (show)
                                 ├──> Phase 7 (verify)
                                 ├──> Phase 9 (fork)
                                 ├──> Phase 10 (sync/tag)
                                 ├──> Phase 11 (register/prune/reset)
                                 ├──> Phase 12 (gen-deploy/migrate)
                                 └──> Phase 13 (dev anvil)

Phases 3-13 ──> Phase 14 (JSON audit + command groups)
```

---

## Summary Table

| Phase | Title | Stories | Depends On |
|------:|-------|--------:|------------|
| 1 | Shared Color Palette and Formatting Primitives | 6 | -- |
| 2 | Tree Renderer and Table Formatter Alignment | 5 | 1 |
| 3 | version, networks, init, and config Output | 6 | 1 |
| 4 | list Command Output | 8 | 2 |
| 5 | show Command Output | 6 | 1 |
| 6 | run Command Output | 7 | 2 |
| 7 | verify Command Output | 5 | 1 |
| 8 | compose Command Output | 5 | 6 |
| 9 | fork Commands Output | 7 | 1 |
| 10 | sync and tag Command Output | 6 | 1 |
| 11 | register, prune, and reset Command Output | 6 | 1 |
| 12 | gen-deploy and migrate Command Output | 5 | 1 |
| 13 | dev anvil Command Output | 5 | 1 |
| 14 | JSON Output Schema Parity and Command Grouping | 6 | 3-13 |
| **Total** | | **83** | |
