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

**User stories:** 8
**Dependencies:** Phase 2

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

**User stories:** 7
**Dependencies:** Phase 2

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
