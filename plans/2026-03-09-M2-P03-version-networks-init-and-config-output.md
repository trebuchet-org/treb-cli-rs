# PRD: Phase 3 - version, networks, init, and config Output

## Introduction

Phase 3 ports the exact output format for four simple commands (`version`, `networks`, `init`, `config`) from the Go CLI to the Rust CLI. These commands use direct `fmt.Printf` or minimal render modules in Go, making them self-contained and independent of the tree/table rendering infrastructure built in Phase 2. All four commands rely on the emoji constants and color palette established in Phase 1.

The current Rust implementations use `print_kv()` (right-aligned key-value), `build_table()` (comfy_table bordered tables), and plain `println!()` — all of which must be replaced with Go-matching emoji-prefixed, color-styled output formats.

## Goals

1. **Version output matches Go exactly**: `treb VERSION\n` + optional 7-char commit + formatted build date — replacing the current 7-field `print_kv()` layout
2. **Networks output matches Go exactly**: `🌐 Available Networks:` header followed by `✅`/`❌` per-line entries — replacing the current bordered comfy_table
3. **Init output matches Go patterns**: Per-step `✅`/`❌` indicators + `🎉` success banner + `📋 Next steps:` section with numbered instructions
4. **Config output matches Go exactly**: `📋 Current config:` header with emoji-prefixed fields (`📦`, `📁`) for show; `✅` checkmark for set/remove — replacing current `print_kv()` and plain `println!()`
5. **All golden files updated**: Every golden file affected by the format changes passes with the new output

## User Stories

### P3-US-001 — Port version Command Output

**Description:** Replace the current `print_kv()` output in `version.rs` with the exact Go format: a first line `treb VERSION`, then an optional blank line followed by `commit:` (7-char truncated) and `built:` (formatted date) lines. The Go version does not show Rust Version, Forge Version, Foundry Version, or treb-sol Commit in human output — only version, commit, and date.

**Files to modify:**
- `crates/treb-cli/src/commands/version.rs` — rewrite human output block (lines 36-46)

**Go reference:** `internal/cli/version.go` lines 17-34

**Go output format:**
```
treb 0.1.0

commit: abc1234
built:  2026-03-09 12:00:00 UTC
```

Key details:
- First line: `treb {VERSION}` (no label prefix)
- Blank line before commit/date (only if commit or date are not "unknown")
- `commit:` label with 7-char truncated hash (note: single space after `commit:`)
- `built:` label with `format_build_date()` output (note: two spaces after `built:` for alignment with `commit:`)
- If both commit and date are "unknown", only the first line is printed
- JSON output (`--json`) is unchanged — keep current `VersionInfo` struct and `print_json()`

**Acceptance criteria:**
- [ ] `treb version` prints `treb {VERSION}` as the first line
- [ ] Commit is truncated to 7 characters and shown as `commit: {SHORT_COMMIT}`
- [ ] Build date uses `format_build_date()` from `output.rs` (Phase 1) and shown as `built:  {FORMATTED_DATE}`
- [ ] When commit and date are both "unknown", only `treb {VERSION}` is printed
- [ ] JSON output via `--json` remains unchanged
- [ ] `cargo clippy --workspace --all-targets` passes
- [ ] `cargo test -p treb-cli --test cli_version` passes (may need assertion updates)

---

### P3-US-002 — Port networks Command Output

**Description:** Replace the current comfy_table bordered table in `networks.rs` with Go's emoji-per-line format. The Go output starts with a `🌐 Available Networks:` header line, followed by a blank line, then per-network lines with `✅` for success or `❌` for error.

**Files to modify:**
- `crates/treb-cli/src/commands/networks.rs` — rewrite human output block (lines 122-145)

**Go reference:** `internal/cli/render/networks.go` lines 25-44

**Go output format (success):**
```
🌐 Available Networks:

  ✅ mainnet - Chain ID: 1
  ✅ sepolia - Chain ID: 11155111
```

**Go output format (with errors):**
```
🌐 Available Networks:

  ✅ mainnet - Chain ID: 1
  ❌ custom - Error: unreachable
```

**Go output format (empty):**
```
No networks configured in foundry.toml [rpc_endpoints]
```

Key details:
- Header: `🌐 Available Networks:` followed by a blank line
- Each success network: `  ✅ {NAME} - Chain ID: {CHAIN_ID}` (2-space indent, chain ID as integer)
- Each error network: `  ❌ {NAME} - Error: {STATUS}` (2-space indent)
- Networks with unresolved env vars show as errors: `  ❌ {NAME} - Error: unresolved env var`
- Empty state message: `No networks configured in foundry.toml [rpc_endpoints]` (note: Go uses `[rpc_endpoints]` not the current Rust wording)
- Use emoji constants from `ui::emoji` (`GLOBE`, `CHECK`, `CROSS`)
- JSON output via `--json` remains unchanged
- No table, no borders, no column headers

**Acceptance criteria:**
- [ ] `treb networks` prints `🌐 Available Networks:` header with blank line
- [ ] Each reachable network shows `  ✅ {NAME} - Chain ID: {ID}`
- [ ] Each unreachable/error network shows `  ❌ {NAME} - Error: {MSG}`
- [ ] Empty state prints `No networks configured in foundry.toml [rpc_endpoints]`
- [ ] Emoji constants imported from `crate::ui::emoji`
- [ ] JSON output via `--json` remains unchanged
- [ ] `cargo clippy --workspace --all-targets` passes
- [ ] `cargo test -p treb-cli --test cli_networks` passes (may need assertion updates)

---

### P3-US-003 — Port init Command Output

**Description:** Replace the current plain-text init output with Go-matching format: per-step `✅`/`❌` indicators for each init operation, a success banner with `🎉` (or `⚠️` for already-initialized), and a numbered "Next steps" section with `📋` header in cyan bold and command examples in dim gray.

The Rust init has different internal steps than Go (no treb-sol check, no treb.toml creation, no .env.example). The output format must match Go patterns but reflect the Rust init's actual operations:
- Fresh init: show checkmarks for registry creation and config creation, then success banner + next steps
- Already initialized (no `--force`): show `⚠️` warning message
- Force reset: show checkmark for config reset

**Files to modify:**
- `crates/treb-cli/src/commands/init.rs` — rewrite all `println!()` calls with emoji-styled output

**Go reference:** `internal/cli/render/init.go` lines 19-82

**Go output format (fresh init):**
```
✅ Valid Foundry project detected
✅ treb-sol library found
✅ Created v2 registry structure in .treb/
✅ Created treb.toml with default sender config
✅ Created .env.example

🎉 treb initialized successfully!

📋 Next steps:
1. Copy .env.example to .env and configure your deployment keys:
   • Set DEPLOYER_PRIVATE_KEY for your deployment wallet
   • Set RPC URLs for networks you'll deploy to
   • Set API keys for contract verification

2. Configure deployment environments in treb.toml:
   • Add accounts in [accounts.<name>] and map them in [namespace.<name>]
   • See documentation for Safe multisig and hardware wallet support

3. Generate your first deployment script:
   treb gen deploy Counter

4. Predict and deploy:
   treb deploy predict Counter --network sepolia
   treb deploy Counter --network sepolia

5. View and manage deployments:
   treb list
   treb show Counter
   treb tag Counter v1.0.0
```

Key details:
- Step lines: green `✅ {MESSAGE}` for success, red `❌ {NAME}` + indented error for failure
- Success banner: green bold `🎉 treb initialized successfully!` (preceded by blank line)
- Already initialized: yellow `⚠️  treb was already initialized in this project` (note: two spaces after warning emoji)
- Next steps header: cyan bold `📋 Next steps:` (preceded by blank line)
- Numbered instructions with `•` bullet sub-items
- Command examples in dim gray (`color::FgHiBlack` equivalent — use `color::DIM` or appropriate dim style)
- Adapt the step messages to Rust's actual init operations (e.g., "Initialized registry in .treb/", "Created config.local.json")
- Adapt the next steps instructions to be relevant to the Rust CLI's available commands and workflow
- Import emoji from `crate::ui::emoji` and colors from `crate::ui::color`

**Acceptance criteria:**
- [ ] Fresh init shows `✅` per operation with descriptive messages
- [ ] Fresh init shows `🎉 treb initialized successfully!` in green bold
- [ ] Fresh init shows `📋 Next steps:` in cyan bold with numbered instructions
- [ ] Command examples in "Next steps" are styled with dim/gray color
- [ ] Already-initialized shows `⚠️` message in yellow (no `--force`)
- [ ] Force reset shows `✅` for config reset
- [ ] Emoji imported from `crate::ui::emoji`
- [ ] Colors imported from `crate::ui::color`, respects `is_color_enabled()`
- [ ] `cargo clippy --workspace --all-targets` passes

---

### P3-US-004 — Port config show Command Output

**Description:** Replace the current `print_kv()` config show output with Go-matching format: a `📋 Current config:` header, emoji-prefixed fields for namespace, network, config source (`📦`), and config file path (`📁`) shown as a relative path. Keep the senders table but ensure consistent styling.

**Files to modify:**
- `crates/treb-cli/src/commands/config.rs` — rewrite `show()` human output block (lines 53-76)

**Go reference:** `internal/cli/render/config.go` lines 41-71

**Go output format:**
```
📋 Current config:
Namespace: default
Network:   (not set)

📦 Config source: treb.toml
📁 config file: .treb/config.local.json
```

Key details:
- Header: `📋 Current config:` (no color in Go, but could match Phase 1 header style)
- Namespace: `Namespace: {VALUE}` (left-aligned, no padding)
- Network: `Network:   {VALUE}` or `Network:   (not set)` (padded to align with Namespace)
- Config source: `📦 Config source: {SOURCE}` (preceded by blank line)
- Config file path: `📁 config file: {RELATIVE_PATH}` — use relative path from cwd
- If not initialized: `❌ No .treb/config.local.json file found` + `⚠️  Without config, commands require explicit --namespace and --network flags`
- Senders table: keep showing after config fields (Go doesn't have this, but Rust does and it's useful — append after a blank line)
- Profile field: the Go version doesn't show profile — consider removing from human output for parity, or keep it as an extra field. Follow Go format for the fields Go shows, then append Rust-specific extras.
- JSON output via `--json` remains unchanged
- Use `std::path::Path::strip_prefix()` or `pathdiff::diff_paths()` to compute relative path from cwd to config file

**Acceptance criteria:**
- [ ] `treb config show` prints `📋 Current config:` header
- [ ] Namespace and Network shown as `Namespace: {VALUE}` / `Network:   {VALUE}`
- [ ] Network shows `(not set)` when unset
- [ ] Config source shown with `📦` emoji
- [ ] Config file path shown with `📁` emoji as relative path
- [ ] Senders table still displayed (if senders exist)
- [ ] Emoji imported from `crate::ui::emoji`
- [ ] JSON output via `--json` remains unchanged
- [ ] `cargo clippy --workspace --all-targets` passes

---

### P3-US-005 — Port config set and config remove Command Output

**Description:** Replace the current plain `println!()` messages in `config set` and `config remove` with Go-matching format: `✅ Set KEY to: VALUE` + `📁 config saved to: RELATIVE_PATH` for set; `✅ Reset namespace to: default` or `✅ Removed network from config (will be required as flag)` + `📁 config saved to: RELATIVE_PATH` for remove.

**Files to modify:**
- `crates/treb-cli/src/commands/config.rs` — rewrite `set()` output (line 97) and `remove()` output (line 118)

**Go reference:** `internal/cli/render/config.go` lines 74-91

**Go output format (set):**
```
✅ Set namespace to: production
📁 config saved to: .treb/config.local.json
```

**Go output format (remove namespace):**
```
✅ Reset namespace to: default
📁 config saved to: .treb/config.local.json
```

**Go output format (remove network):**
```
✅ Removed network from config (will be required as flag)
📁 config saved to: .treb/config.local.json
```

Key details:
- Set: `✅ Set {KEY} to: {VALUE}` (note: `to:` with colon)
- Remove namespace: `✅ Reset namespace to: default` (special case — resets to "default")
- Remove network: `✅ Removed network from config (will be required as flag)` (different message)
- Config path: `📁 config saved to: {RELATIVE_PATH}` (relative path from cwd)
- Use same relative path computation as P3-US-004
- Use emoji constants from `crate::ui::emoji`

**Acceptance criteria:**
- [ ] `treb config set namespace production` prints `✅ Set namespace to: production`
- [ ] `treb config set network sepolia` prints `✅ Set network to: sepolia`
- [ ] Set command prints `📁 config saved to: {RELATIVE_PATH}` on second line
- [ ] `treb config remove namespace` prints `✅ Reset namespace to: default`
- [ ] `treb config remove network` prints `✅ Removed network from config (will be required as flag)`
- [ ] Remove command prints `📁 config saved to: {RELATIVE_PATH}` on second line
- [ ] Config path is displayed as relative path from current directory
- [ ] `cargo clippy --workspace --all-targets` passes

---

### P3-US-006 — Update Golden Files for All Changed Commands

**Description:** Regenerate all golden files affected by the output changes in P3-US-001 through P3-US-005. Update CLI integration test assertions (`cli_*.rs`) that check for old format strings.

**Files to update:**
- `crates/treb-cli/tests/golden/version_human/commands.golden`
- `crates/treb-cli/tests/golden/networks_no_endpoints/commands.golden`
- `crates/treb-cli/tests/golden/networks_unresolved_env_vars/commands.golden`
- `crates/treb-cli/tests/golden/init_fresh/commands.golden`
- `crates/treb-cli/tests/golden/init_idempotent/commands.golden`
- `crates/treb-cli/tests/golden/init_force/commands.golden`
- `crates/treb-cli/tests/golden/config_show_default/commands.golden`
- `crates/treb-cli/tests/golden/config_set_show_round_trip/commands.golden`
- `crates/treb-cli/tests/golden/config_remove_show_round_trip/commands.golden`
- `crates/treb-cli/tests/golden/config_set_invalid_key/commands.golden` (if error format changed)
- `crates/treb-cli/tests/cli_version.rs` — update string assertions
- `crates/treb-cli/tests/cli_networks.rs` — update string assertions
- `crates/treb-cli/tests/cli_init.rs` — update string assertions
- `crates/treb-cli/tests/cli_config.rs` — update string assertions

**Procedure:**
1. Run `UPDATE_GOLDEN=1 cargo test -p treb-cli --test integration_version --test integration_networks --test integration_init --test integration_config` to regenerate golden files
2. Verify the diff of each golden file matches expected new format
3. Update `cli_*.rs` integration test assertions to check for new format strings (e.g., check for `treb 0.1.0` instead of `Version:  0.1.0`)
4. Run full test suite: `cargo test -p treb-cli` to confirm no regressions

**Acceptance criteria:**
- [ ] All golden files reflect the new Go-matching output format
- [ ] `cargo test -p treb-cli --test integration_version` passes
- [ ] `cargo test -p treb-cli --test integration_networks` passes
- [ ] `cargo test -p treb-cli --test integration_init` passes
- [ ] `cargo test -p treb-cli --test integration_config` passes
- [ ] `cargo test -p treb-cli --test cli_version` passes
- [ ] `cargo test -p treb-cli --test cli_networks` passes
- [ ] `cargo test -p treb-cli --test cli_init` passes
- [ ] `cargo test -p treb-cli --test cli_config` passes
- [ ] `cargo test --workspace --all-targets` passes (no regressions in other tests)
- [ ] `cargo clippy --workspace --all-targets` passes
- [ ] Golden file diffs reviewed — each file shows expected format change

## Functional Requirements

- **FR-1:** `treb version` human output must be `treb {VERSION}\n` optionally followed by a blank line, `commit: {7-CHAR-HASH}\n`, and `built:  {FORMATTED-DATE}\n`
- **FR-2:** `treb version --json` output must remain unchanged (all 7 fields in camelCase)
- **FR-3:** `treb networks` human output must use `🌐 Available Networks:` header + `✅`/`❌` per-line format (no table)
- **FR-4:** `treb networks` empty state must print `No networks configured in foundry.toml [rpc_endpoints]`
- **FR-5:** `treb networks --json` output must remain unchanged
- **FR-6:** `treb init` must show per-operation `✅`/`❌` indicators, `🎉` success banner (green bold), and `📋 Next steps:` section (cyan bold)
- **FR-7:** `treb init` on already-initialized project must show `⚠️` warning in yellow
- **FR-8:** `treb config show` must use `📋 Current config:` header, `📦` config source, `📁` config file path (relative)
- **FR-9:** `treb config set` must output `✅ Set {KEY} to: {VALUE}` + `📁 config saved to: {PATH}`
- **FR-10:** `treb config remove namespace` must output `✅ Reset namespace to: default` + path line
- **FR-11:** `treb config remove network` must output `✅ Removed network from config (will be required as flag)` + path line
- **FR-12:** All emoji must be imported from `crate::ui::emoji` constants (Phase 1)
- **FR-13:** All color styling must use `crate::ui::color` palette and respect `is_color_enabled()` / `NO_COLOR`
- **FR-14:** Config file paths must be displayed as relative paths from the current working directory
- **FR-15:** All golden files must be updated to match new output format

## Non-Goals

- **No behavioral changes**: Command logic, config resolution, chain ID resolution, and registry operations remain unchanged — only output formatting is modified
- **No JSON schema changes**: All `--json` output structures remain identical
- **No new commands or subcommands**: Only existing command output is reformatted
- **No init architecture refactor**: The Rust init command keeps its current 3-case flow (fresh/idempotent/force) — no step-based result struct like Go's `InitProjectResult`
- **No senders display removal**: The Rust `config show` keeps its senders table even though Go doesn't have it
- **No error message changes**: Error messages and error paths remain unchanged
- **No tree or table renderer changes**: This phase uses direct `println!`/`eprintln!` with emoji, not the tree/table infrastructure from Phase 2

## Technical Considerations

### Dependencies
- **Phase 1 (completed):** Emoji constants in `ui/emoji.rs` (`CHECK`, `CROSS`, `GLOBE`, `PARTY`, `CLIPBOARD`, `PACKAGE`, `FOLDER`, `WARNING`) and color palette in `ui/color.rs`
- **Phase 1 (completed):** `format_build_date()` helper in `output.rs`

### Relative Path Computation
Both `config show` (P3-US-004) and `config set/remove` (P3-US-005) need to display the config file path relative to cwd. Use `std::path::Path::strip_prefix()` with the current directory, falling back to the absolute path if the prefix strip fails. This is the same pattern as Go's `getRelativePath()` in `render/config.go:26-38`.

### Color in Tests
From Phase 2 learnings: `is_color_enabled()` returns `true` in subprocess golden tests (checks `NO_COLOR`/`TERM=dumb` only, not TTY). Styled output will include ANSI codes in golden files. Use the normalizer's ANSI-stripping if golden files should be color-agnostic, or include ANSI codes in expected output.

### Init Output Adaptation
The Go init has 5 steps with a step-based renderer. The Rust init has 3 code paths (fresh/idempotent/force) without a step abstraction. Rather than adding a step-based architecture, add `println!`-based emoji output inline in each code path:
- Fresh: `✅ Initialized registry` + `✅ Created config.local.json` + success banner + next steps
- Idempotent: `⚠️  Project already initialized...`
- Force: `✅ Reset local config` + (optionally) abbreviated next steps

### Golden File Update Strategy
From Phase 2 learnings: Use `UPDATE_GOLDEN=1` with `--test` targeting specific test binaries to avoid regenerating unrelated golden files. Always verify the diff after regeneration.

### Version Output — Field Reduction
The Go version shows only 3 fields (version, commit, date) in human output. The Rust currently shows 7 fields. For Go parity, the human output drops Rust Version, Forge Version, Foundry Version, and treb-sol Commit. These fields remain available in `--json` output via the unchanged `VersionInfo` struct.
