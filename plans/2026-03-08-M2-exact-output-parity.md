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

**Learnings from implementation:**
- Verify human output goes to stderr via `eprintln!` — only JSON and table output go to stdout; this pattern applies when porting batch/progress output in other commands
- `styled(text, color::STYLE)` helper in verify.rs for conditional coloring — same pattern as show.rs, use `color::is_color_enabled()` check
- `get_status_icon` maps `VerificationStatus` to Go-matching emoji: Verified→🔄(REFRESH), Failed→⚠️(WARNING), Partial→🔁(REPEAT), Unverified→⏳(HOURGLASS) — capture status BEFORE running verification to show pre-verification state
- `title_case()` helper added for verifier name capitalization (etherscan → Etherscan) — useful pattern for normalizing lowercase enum/key names for display
- `print_verification_status()` sorts verifier names alphabetically for deterministic output — always sort dynamic keys for reproducible output
- Batch verify uses `chain:CHAINID/NS/NAME` location format — the `namespace` field on Deployment is required for this format
- Batch output writes progress/summary to stderr (not stdout) — unlike old table which used stdout via `print_table`; stdout is reserved for structured (JSON/table) output
- The `resolved` deployment has a `label` field that must be captured before the borrow is released — clone needed values before moving/releasing borrows
- Pre-existing test failure `show_proxy_deployment_shows_proxy_info` in cli_list_show was unrelated to verify changes — caused by missing `NO_COLOR=1` env; always investigate pre-existing failures before assuming your changes caused them
- When changing batch output, CLI tests with hardcoded old-format expectations need updating — search for command-specific test functions (e.g., `verify_all_*`) in `cli_verify.rs`
- Golden test column widths for fork commands drifted after table rendering changes — run `UPDATE_GOLDEN=1` after any rendering changes to catch cross-command drift

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

**Learnings from implementation:**
- `print_dry_run_plan()` renamed to `print_execution_plan()` — reused for both dry-run plan display and pre-execution plan display (same plan, two call sites)
- `color::GREEN` vs `color::SUCCESS`: GREEN is plain green (Go `FgGreen`), SUCCESS is green+bold (Go `FgGreen+Bold`). Same distinction for `color::RED` vs `color::ERROR` — choose based on Go reference
- `color::CYAN` (plain cyan) vs `color::STAGE` (cyan+bold) — use CYAN for data labels (component names), STAGE for section headers
- Summary/progress output goes to stderr (`eprintln!`), JSON output goes to stdout (`println!`) — this is a key convention across all commands
- The `styled()` helper in compose.rs wraps text with color only if `color::is_color_enabled()` is true — same pattern as show.rs and verify.rs
- `#[serde(skip_serializing_if = "Option::is_none")]` is the pattern for optional fields that should not appear in JSON when absent — keeps JSON backward-compatible
- Rust `{:?}` format for HashMap produces `{"KEY": "VALUE"}` which differs from Go `map[KEY:VALUE]` — acceptable difference per plan
- When adding new output that includes error messages, check existing tests for negative assertions that do substring matching — they may need tightening to avoid false matches
- Fork golden tests had pre-existing table width drift from earlier phases — always run full `cargo test -p treb-cli` not just the command-specific subset
- Golden tests normalize colors away (NO_COLOR), so color changes only show as text changes in golden diffs
- All 88 compose golden tests passed without needing `UPDATE_GOLDEN=1` — execution-time output changes (non-dry-run) don't affect golden files since golden tests exercise dry-run paths

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

**Notes from Phase 8:**
- `color::GREEN` (plain green) vs `color::SUCCESS` (green+bold), `color::RED` (plain red) vs `color::ERROR` (red+bold) — match against Go reference to pick the right variant
- `color::CYAN` (plain) for data labels, `color::STAGE` (cyan+bold) for section headers — don't confuse them
- Always run full `cargo test -p treb-cli` to catch cross-command golden drift, not just fork-specific tests

**User stories:** 7
**Dependencies:** Phase 1

**Learnings from implementation:**
- `render_fork_fields()` helper uses `{:<14}` label alignment with 2-space indent for key-value field lists — pattern works well for Go-matching indented output without comfy_table
- Different indent levels per context: 2-space for top-level fields (enter/restart), 4-space for fields nested under network name (status) — match Go nesting depth
- Fork revert uses 12-char label alignment (`{:<12}`) for `Reverted:` / `Remaining:` vs 14-char for enter/status — alignment width varies per subcommand in Go
- `insert_active_fork()` takes ownership of `ForkEntry` — clone before passing if you need the entry afterward for display
- Fork commands now use NO comfy_table — all tables replaced with indented key-value pairs or `+`/`~` prefix format; comfy_table is only needed for commands using the two-pass `render_table_with_widths` pattern (list, run)
- After removing the last comfy_table usage from a command, check for dead `styled()` / `owo_colors` imports — they become unused when no styled table cells remain
- TWO test files cover fork diff: `integration_fork.rs` (golden) and `fork_integration.rs` (assertions) — always search for ALL test files covering a command before changing output format
- When adding fields to `ForkSubcommand` enum variants (e.g., `--all` flag), update BOTH `run()` dispatch AND `mod tests` parse tests
- `TimestampNormalizer` in the test framework matches `%Y-%m-%d %H:%M:%S` with or without UTC suffix — no normalizer changes needed when dropping UTC suffix from timestamps
- `println!()` between sections produces trailing blank lines that the golden test framework trims — canonical output via `UPDATE_GOLDEN=1` may differ from manual edits; always regenerate with `UPDATE_GOLDEN=1` as final step
- Single-result JSON uses object directly, multi-result (e.g., `--all` exit) uses array — keeps backward compatibility for existing consumers
- History entries stored most-recent-first; reverse per-network for chronological display with index 0 = "initial"
- Fork diff extracts `address` and `type` from deployment JSON values; `format_diff_suffix()` gracefully handles missing `type` field (shows just address)

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

**Notes from Phase 7:**
- Human progress/status output should go to stderr (`eprintln!`), reserving stdout for structured output (JSON/table) — verify sync progress uses the same pattern
- `styled(text, color::STYLE)` helper pattern for conditional coloring is established in both show.rs and verify.rs — reuse for tag/sync output

**Notes from Phase 8:**
- `color::GREEN` (plain green) vs `color::SUCCESS` (green+bold), `color::RED` vs `color::ERROR` — match Go reference to pick the right variant
- Summary/progress output to stderr (`eprintln!`), JSON to stdout (`println!`) — confirmed as universal convention

**Notes from Phase 9:**
- Indented key-value format with aligned labels (e.g., `{:<14}`) is a proven alternative to comfy_table for simple displays — consider for sync metrics and tag show output instead of tables
- After removing comfy_table from a command, check for dead `styled()` / `owo_colors` imports — they become unused when no styled table cells remain
- Always search for ALL test files covering a command before changing output — there may be both golden and assertion-based test files

**Learnings from implementation:**
- Sync output uses `println!` for stdout results and `output::print_stage` (`eprintln`) for stderr progress — don't mix them within a single command's output flow
- `emoji::CHECK_MARK` is ✓ (U+2713), `emoji::CHECK` is ✅ (U+2705) — different emoji for different contexts (sync footer uses CHECK_MARK, tag add/remove uses CHECK)
- `color::GREEN` is plain green (Go `FgGreen`), `color::SUCCESS` is green+bold — use GREEN for inline styled text within a sentence
- Bullet character `\u{2022}` (•) for Go-matching section bullets — not a dash or asterisk
- `color::SECTION_HEADER` is `bold().bright_white()` — matches Go `FgHiWhite, Bold` for label text like "Address:" and "Tags:"
- Extract `format_*` helper functions that return `String` to make output testable without stdout capture — avoids needing to capture stdout in unit tests
- To disable color in unit tests: call `color::color_enabled(true)` (sets `COLOR_ENABLED` atomic to false), restore with `owo_colors::set_override(true)` after — `owo_colors::set_override(false)` alone is insufficient
- `#[allow(clippy::too_many_arguments)]` needed on format functions with 8+ params — consider a struct parameter if the function grows beyond this
- The `errors` Vec in sync.rs serves dual purpose: warnings section display AND footer message selection (empty → "synced successfully", non-empty → "completed with warnings")
- Tags need `let mut` to sort alphabetically before display — sort in-place before rendering
- Golden files can be pre-updated during review fix commits — always check git log before assuming regeneration is needed; US-006 may become verification-only

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

**Notes from Phase 7:**
- Human progress/status output (checking, pruning, success messages) should go to stderr (`eprintln!`) — stdout reserved for structured output
- Pre-existing test failures may surface when running full test suite — always check with `NO_COLOR=1` env for tests asserting on formatted text

**Notes from Phase 8:**
- When adding output that includes error messages (e.g., `• Error: ...` bullets in summary), check existing tests for negative assertions that do substring matching — they may need tightening
- `color::GREEN` (plain) vs `color::SUCCESS` (green+bold), `color::RED` vs `color::ERROR` — match Go reference to pick the right variant

**Notes from Phase 9:**
- Indented aligned labels (e.g., `{:<14}` or `{:<12}`) work well for registry management summary output (e.g., reset per-type counts) — simpler than comfy_table
- When adding flags to subcommand enum variants, update BOTH `run()` dispatch AND `mod tests` parse tests
- `println!()` between sections produces trailing blank lines trimmed by the golden framework — always finalize with `UPDATE_GOLDEN=1`

**Notes from Phase 10:**
- Extract `format_*` helper functions returning `String` for testable output — avoids stdout capture in unit tests; proven pattern in both sync.rs and tag.rs
- Bullet character `\u{2022}` (•) for Go-matching section bullets in multi-section output (e.g., prune sections with item lists)
- `color::SECTION_HEADER` (`bold().bright_white()`) for label text like "Address:", "Tags:" — matches Go `FgHiWhite, Bold`
- To disable color in unit tests: `color::color_enabled(true)` then restore with `owo_colors::set_override(true)` — proven test pattern for format function tests
- `#[allow(clippy::too_many_arguments)]` needed on format functions with 8+ params — consider struct parameter if function grows

**Learnings from implementation:**
- `RegisteredDeploymentJson` doesn't carry a `label` field (to keep JSON unchanged) — labels tracked in a parallel `dep_labels: Vec<String>`; clone `dep_label` before it's moved into the `Deployment` struct
- `PruneCandidate` uses `#[serde(skip)]` (not `skip_serializing_if`) for display-only fields (`address`, `status`) that should never appear in JSON — use `skip` for fields that are purely for human rendering
- Transaction status stored as uppercase (e.g., "EXECUTED") but displayed as title case (e.g., "Executed") in human output — normalize case for display
- When removing `styled()` helper from a command, also remove `owo_colors` and `color` imports to avoid dead code warnings — search for all related imports
- `backup_registry()` is still called but the backup path is no longer displayed in human output — use `_backup_path` prefix to suppress unused variable warnings
- Reset uses `--network` as chain ID integer; defaults are namespace="default", network="31337" — the chain ID value appears in both `network` and `chain` positions in the output message
- Per-type count lines only printed when count > 0 (matching Go behavior) — don't show zero-count sections
- E2E test helper `run_human()` added for capturing human (non-JSON) CLI output; `run_json()` adds `--json` automatically — use `run_human()` for all future E2E human output assertions
- When implementation stories (US-001/002/003) already update golden tests, the dedicated test-update story (US-004/006) becomes a no-op verification — still valuable as a consistency check but expect no code changes
- Unit tests for prune/reset test pure logic (candidate detection, filtering, scope resolution) — they don't test CLI output formatting, so output changes don't break them
- Error golden files are inherently stable since error paths don't go through human output formatting code — no need to update error golden files when changing success/normal output
- Prune section headers include count in parentheses: "Deployments (N):" — match with "Deployments (" pattern in assertions, not exact count

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

**Notes from Phase 10:**
- Extract `format_*` helper functions returning `String` for testable output — proven pattern for unit testing formatted output without stdout capture
- To disable color in unit tests: `color::color_enabled(true)` then restore with `owo_colors::set_override(true)`

**Notes from Phase 11:**
- When removing `styled()` helper from a command, also remove `owo_colors` and `color` imports to avoid dead code warnings
- When implementation stories already update golden files, the dedicated golden-update story becomes a no-op verification — expect no code changes but still run as consistency check
- Error golden files are stable when changing success/normal output formatting — no updates needed
- E2E human output assertions: use `run_human()` helper (captures stdout without `--json` flag)

**Learnings from implementation:**
- gen-deploy success output goes to stderr via `eprintln!` and `output::print_stage` (not stdout) — instruction lines after success are also plain text to stderr (no styling), matching Go
- `TemplateContext` struct has `is_library: bool` and `proxy: Option<String>` fields available at the gen-deploy output point (around line 874-902 in gen_deploy.rs)
- migrate config success uses `styled()` + `color::SUCCESS` with `emoji::CHECK_MARK` (✓), NOT `print_stage` with ✅ — different emoji/styling pattern than gen-deploy
- migrate config human output goes to stdout via `println!`, but warnings go to stderr via `eprintln!` — mixed stdout/stderr in same command is valid when matching Go behavior
- `write_or_print_v2_with_prompt` has two sequential prompts when treb.toml exists: overwrite prompt first, then write/preview prompt — tests must handle both prompts
- Integration tests asserting on human output text must use `.env("NO_COLOR", "1")` + `--no-color` to strip ANSI codes; `color_enabled()` defaults to `true` so ANSI codes appear even when stdout is piped
- Golden files may already be updated by review-fix commits on prior stories — check test status before regenerating; error case and JSON golden files are typically unaffected by human output format changes

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

**Notes from Phase 9:**
- Indented key-value format with conditional field omission (fields not shown when value is empty/zero) is a proven pattern from fork enter/status — reuse for dev anvil status output
- Different indent levels distinguish nesting: 2-space for top-level labels, 4-space for fields nested under a group header

**Notes from Phase 10:**
- Extract `format_*` helper functions returning `String` for testable output — proven pattern for unit testing formatted output without stdout capture
- `emoji::CHECK_MARK` (✓ U+2713) vs `emoji::CHECK` (✅ U+2705) — use the correct variant per Go reference
- To disable color in unit tests: `color::color_enabled(true)` then restore with `owo_colors::set_override(true)`

**Notes from Phase 11:**
- When removing `styled()` helper from a command, also remove `owo_colors` and `color` imports to avoid dead code warnings
- Error golden files are stable when changing success/normal output formatting — no updates needed
- E2E human output assertions: use `run_human()` helper (captures stdout without `--json` flag)

**Notes from Phase 12:**
- Integration tests asserting on human output must use `.env("NO_COLOR", "1")` + `--no-color` to strip ANSI codes — `color_enabled()` defaults to `true` even when stdout is piped
- `emoji::CHECK_MARK` (✓) for success lines in styled context, `emoji::CHECK` (✅) for `print_stage` — dev anvil start/stop/status should match Go reference for which emoji variant to use
- Success output may go to stderr (`eprintln!` / `print_stage`) not stdout — verify per-subcommand in Go reference

**Learnings from implementation:**
- Path helpers (`pid_file_path`, `log_file_path`) are pure path computations — safe to call early for output formatting before the anvil instance is running
- `human_display_path()` helper exists in dev.rs for converting absolute paths to relative display paths — use instead of manual `strip_prefix` when available
- CreateX deployment in Go CLI is non-fatal (warning on failure) — Rust now matches; when porting Go behavior, check whether errors are fatal or non-fatal (warnings)
- `check_rpc_health()` and `check_createx_deployed()` use JSON-RPC calls (`eth_chainId`, `eth_getCode`) with 5s timeout — RPC probes only run when instance is running (port reachable)
- The `styled()` helper is duplicated across many command files (dev.rs, show.rs, verify.rs, compose.rs, etc.) — same `fn styled(text: &str, style: Style) -> String` pattern; consider extracting to shared module in future cleanup
- Restart delegates to the same function as start (`run_anvil_start_with_entry()`), so output changes propagate automatically — shared render functions reduce porting effort
- comfy_table was used inline (`comfy_table::Cell::new(...)`) with no import statement in dev.rs — removing usage automatically removes all references without needing to clean up imports
- `entry.pid_file` / `entry.log_file` may be empty strings — use path helper functions (`pid_file_path`, `log_file_path`) as fallback when entry fields are empty
- Golden files were already updated by review-fix commits on prior stories — the final "verify golden files" story (US-005) was a no-op verification confirming consistency
- No golden tests exist for the logs command — not all subcommands have golden test coverage; check golden file inventory before planning test updates

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

**Notes from Phase 8:**
- `compose --json` added optional `env` field to `PlanEntry` with `#[serde(skip_serializing_if = "Option::is_none")]` — verify this matches Go JSON schema (env present only when non-empty)
- `#[serde(skip_serializing_if = "Option::is_none")]` is the canonical pattern for optional fields that should be absent (not null) in JSON when unset — equivalent to Go `omitempty` for pointer/slice types

**Notes from Phase 9:**
- Fork exit `--all` emits array for multi-network results, object for single-network — verify this single-vs-multi JSON pattern is consistent with Go output across all commands that support `--all`
- Fork diff JSON includes `removed` deployments (in snapshot but not current) even though human output has no "Removed Deployments" section — audit whether Go JSON includes removed or omits them

**Notes from Phase 10:**
- JSON golden tests confirmed unaffected by human output changes (again) — `sync_json_empty`, `sync_governor_json` passed without changes; no JSON audit needed for sync/tag
- Golden files can be pre-updated during review fix commits — when auditing JSON in Phase 14, check git log for prior golden updates before assuming regeneration is needed

**Notes from Phase 11:**
- `PruneCandidate` uses `#[serde(skip)]` for display-only fields (`address`, `status`) — these never appear in JSON; no JSON audit needed for prune
- `RegisteredDeploymentJson` doesn't include `label` field — labels handled separately in human output; JSON output unchanged by register porting
- JSON output confirmed unchanged by register/prune/reset human output changes — no JSON audit needed for these three commands

**Notes from Phase 12:**
- JSON output confirmed unchanged by gen-deploy/migrate human output changes — no JSON audit needed for gen-deploy/migrate
- Error case and JSON golden files are typically unaffected by human output format changes — confirmed again across all Phase 12 stories
- Golden files may already be updated by review-fix commits on implementation stories — check git log before assuming regeneration is needed

**Notes from Phase 13:**
- JSON output confirmed unchanged by dev anvil human output changes — dev anvil status `--json` output was not modified; no JSON audit needed for dev anvil
- `styled()` helper pattern confirmed duplicated across 5+ command files (dev.rs, show.rs, verify.rs, compose.rs, tag.rs, sync.rs) — Phase 14 may want to note this as a future cleanup candidate but it does not affect JSON audit scope
- All Phase 1-13 human output porting is complete — Phase 14 JSON audit can proceed with full confidence that human output is finalized

**Learnings from implementation:**
- Clap 4.x does NOT support multiple subcommand headings — workaround: hide all subcommands via `cmd.mut_subcommand(name, |s| s.hide(true))`, generate grouped text via `build_grouped_help()` using `cmd.find_subcommand(name).get_about()`, inject via `after_help()` + custom `help_template` that excludes `{subcommands}`
- Hiding subcommands removes `<COMMAND>` from auto-generated usage — use `override_usage("treb [OPTIONS] <COMMAND>")` to restore it
- `{after-help}` template variable adds a leading `\n` — place directly after `{usage}` without explicit `\n` for correct spacing
- `{options}` template variable renders options WITHOUT the "Options:" heading — must add heading manually in template
- Replaced `Cli::try_parse()` with `build_grouped_command().try_get_matches()` + `Cli::from_arg_matches()` for proper grouped help rendering (requires importing `clap::FromArgMatches` trait)
- Clap derives `about` from first `///` doc comment line and `long_about` from full block — use `#[command(long_about = "...")]` to set them independently
- `build_grouped_help()` reads `get_about()` from subcommands — changing doc comments on enum variants automatically updates the grouped help display
- No golden files or tests assert on help description text content — description changes don't require golden file updates
- All Option fields in JSON output structs MUST have `#[serde(skip_serializing_if = "Option::is_none")]` to match Go `omitempty` behavior — avoid producing null values in JSON
- Exception: `Deployment` struct (treb-core) intentionally serializes `proxyInfo` and `tags` as null (not omitted) — matches Go behavior for these specific fields
- When using inline `serde_json::json!()` with Option values, use conditional field insertion instead of direct mapping (which produces null)
- `type` is a Rust keyword — use `deployment_type` with `#[serde(rename = "type")]` for JSON fields named "type"
- For removed entries in fork diff, data must come from snapshot (`in_snapshot`) since `in_current` is None — use `in_current.or(in_snapshot).unwrap()`
- `Deployment` struct takes ownership of `dep_label` — clone before passing if needed for JSON struct construction
- Removing fields from JSON output structs can make variables unused — check for compiler warnings after schema changes
- Golden file normalization for `foundryVersion` uses `<VERSION>` placeholder — auto-normalizes when foundry version changes
- `UPDATE_GOLDEN=1` produces no extra changes when tests already pass — safe to run as a final verification step
- Go CLI reference source for command descriptions: `/home/sol/projects/treb-cli/internal/cli/` — each command file has `Short`/`Long` cobra fields

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
