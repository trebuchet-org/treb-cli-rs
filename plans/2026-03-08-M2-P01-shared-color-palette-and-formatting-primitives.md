# PRD: Phase 1 - Shared Color Palette and Formatting Primitives

## Introduction

This phase establishes the shared visual foundation for achieving exact 1:1 output parity between the Rust CLI (`treb-cli-rs`) and the Go CLI (`../treb-cli`). The Go CLI defines a specific color palette, emoji constants, and formatting helpers across its render module (`internal/cli/render/`). The Rust CLI currently has a different palette in `ui/color.rs` and different badge characters in `ui/badge.rs`. This phase replaces those with exact Go equivalents and adds missing formatting primitives (`format_warning`, `format_error`, `format_success`, `format_duration`, `format_build_date`, emoji constants) that all subsequent phases depend on.

This is Phase 1 of 14 in the Exact 1:1 Output Parity master plan. Every other phase depends on the primitives established here.

## Goals

1. **Exact color palette match**: Replace the Rust `ui/color.rs` palette constants with styles matching the Go `fatih/color` definitions from `render/deployments.go:19-34` (Yellow bg namespace, Cyan bg chain, Magenta proxy, Blue library, Green singleton, etc.)
2. **Comprehensive emoji catalog**: Provide a single emoji constants module containing every emoji used across all Go render files (24+ distinct characters), so later phases can import them instead of scattering raw Unicode.
3. **Go-matching format helpers**: Implement `format_warning()`, `format_error()`, `format_success()`, `format_duration()`, and `format_build_date()` functions that produce output identical to their Go counterparts.
4. **Verification badge parity**: Update `ui/badge.rs` to use the Go verification symbols (`✔︎` for verified, `-` for failed, `⏳` for pending) with Go-matching colors (green verified, red not-verified, yellow pending).
5. **Zero test regression**: All existing golden file tests and unit tests pass, with golden files updated where format changes affect output.

## User Stories

### P1-US-001: Replace Color Palette Constants in `ui/color.rs`

**Description:** Replace the existing palette constants in `crates/treb-cli/src/ui/color.rs` with styles that exactly match the Go `fatih/color` definitions from `render/deployments.go:19-34`.

**Changes:**
- `crates/treb-cli/src/ui/color.rs` — Replace/add these palette constants:
  - `NS_HEADER`: Black text on Yellow background (Go: `nsHeader = color.New(nsBg, color.FgBlack)`)
  - `NS_HEADER_BOLD`: Black bold text on Yellow background (Go: `nsHeaderBold`)
  - `CHAIN_HEADER`: Black text on Cyan background (Go: `chainHeader`)
  - `CHAIN_HEADER_BOLD`: Black bold text on Cyan background (Go: `chainHeaderBold`)
  - `ADDRESS`: White (Go: `addressStyle = color.New(color.FgWhite)`) — currently White+Dimmed, must be plain White
  - `TIMESTAMP`: Faint/dimmed (Go: `timestampStyle = color.New(color.Faint)`) — matches current `MUTED`
  - `PENDING`: Yellow (Go: `pendingStyle = color.New(color.FgYellow)`)
  - `TAGS`: Cyan (Go: `tagsStyle = color.New(color.FgCyan)`)
  - `VERIFIED`: Green (Go: `verifiedStyle = color.New(color.FgGreen)`) — currently Green+Bold, must be plain Green
  - `NOT_VERIFIED`: Red (Go: `notVerifiedStyle = color.New(color.FgRed)`) — currently Red+Bold, must be plain Red
  - `SECTION_HEADER`: Bold bright-white (Go: `sectionHeaderStyle = color.New(color.Bold, color.FgHiWhite)`)
  - `IMPL_PREFIX`: Faint/dimmed (Go: `implPrefixStyle = color.New(color.Faint)`)
  - `FORK_INDICATOR`: Yellow (Go: `forkIndicatorStyle = color.New(color.FgYellow)`) — currently Yellow+Bold, must be plain Yellow
  - `TYPE_PROXY`: Magenta+Bold (Go: `color.FgMagenta, color.Bold`) — currently Blue+Bold
  - `TYPE_LIBRARY`: Blue+Bold (Go: `color.FgBlue, color.Bold`) — currently Yellow
  - `TYPE_SINGLETON` / default: Green+Bold (Go: `color.FgGreen, color.Bold`) — matches
  - Script renderer colors: `BOLD`, `GRAY` (FgHiBlack), `CYAN`, `YELLOW`, `GREEN`, `RED` as standalone styles
- Keep existing `should_use_color()`, `color_enabled()`, `is_color_enabled()` functions unchanged
- Keep `style_for_deployment_type()` but update its mappings to the new constants
- Retain the `Send + Sync` compile-time check

**Acceptance Criteria:**
- [ ] Every Go color variable from `deployments.go:19-34` has a corresponding Rust `Style` constant
- [ ] `style_for_deployment_type(Proxy)` returns Magenta+Bold
- [ ] `style_for_deployment_type(Library)` returns Blue+Bold
- [ ] `VERIFIED` is Green (not Green+Bold); `NOT_VERIFIED` is Red (not Red+Bold)
- [ ] `FORK_INDICATOR` is Yellow (not Yellow+Bold)
- [ ] `ADDRESS` is White (not White+Dimmed)
- [ ] Existing unit tests updated and pass
- [ ] `cargo clippy --workspace --all-targets` passes

---

### P1-US-002: Create Emoji Constants Module

**Description:** Create a new `crates/treb-cli/src/ui/emoji.rs` module with named constants for every emoji character used across the Go CLI render files. Register it in `ui/mod.rs`.

**Changes:**
- New file: `crates/treb-cli/src/ui/emoji.rs`
- `crates/treb-cli/src/ui/mod.rs` — Add `pub mod emoji;`

**Emoji constants to define (from comprehensive Go source audit):**

| Constant | Emoji | Unicode | Go usage |
|----------|-------|---------|----------|
| `CHECK` | ✅ | U+2705 | Success/completion (init, anvil, config, networks, tag, prune, generate) |
| `CROSS` | ❌ | U+274C | Error/failure (prune, config, networks, anvil, init, compose) |
| `CHECK_MARK` | ✓ | U+2713 | Inline success (run, migrate, register, sync, compose, verify) |
| `CROSS_MARK` | ✗ | U+2717 | Inline failure (verify) |
| `WARNING` | ⚠️ | U+26A0+FE0F | Warning indicator (helpers, prune, init, anvil, config, script) |
| `ROCKET` | 🚀 | U+1F680 | Deployment execution (transaction, script) |
| `REFRESH` | 🔄 | U+1F504 | Transactions section header (script), re-verifying (verify) |
| `REPEAT` | 🔁 | U+1F501 | Retry/partial verification (verify) |
| `HOURGLASS` | ⏳ | U+23F3 | Pending/queued status (deployments, verify) |
| `FAST_FORWARD` | ⏭️ | U+23ED+FE0F | Skip indicator (verify) |
| `NEW` | 🆕 | U+1F196 | New verification status (verify) |
| `PACKAGE` | 📦 | U+1F4E6 | Deployment summary (script, config, compose) |
| `MEMO` | 📝 | U+1F4DD | Script logs section (script) |
| `CLIPBOARD` | 📋 | U+1F4CB | List/plan (anvil, init, config, compose) |
| `CHART` | 📊 | U+1F4CA | Statistics/status (anvil, compose) |
| `FOLDER` | 📁 | U+1F4C1 | File location (config) |
| `PARTY` | 🎉 | U+1F389 | Celebration/success banner (init, compose) |
| `TARGET` | 🎯 | U+1F3AF | Orchestration target (compose) |
| `GLOBE` | 🌐 | U+1F310 | Network/RPC URL (anvil, networks) |
| `WRENCH` | 🔧 | U+1F527 | Maintenance/pruning (prune) |
| `GREEN_CIRCLE` | 🟢 | U+1F7E2 | Running status (anvil) |
| `RED_CIRCLE` | 🔴 | U+1F534 | Stopped status (anvil) |
| `MAGNIFYING_GLASS` | 🔍 | U+1F50D | Checking/searching (prune) |
| `WASTEBASKET` | 🗑️ | U+1F5D1+FE0F | Items to delete (prune) |
| `VERIFIED_WIDE` | ✔︎ | U+2714+FE0E | Verified status in deployment table (deployments) |
| `CIRCLE` | ● | U+25CF | Namespace marker in tree |
| `CHAIN_EMOJI` | ⛓ | U+26D3 | Chain marker in tree |

**Acceptance Criteria:**
- [ ] `crates/treb-cli/src/ui/emoji.rs` exists with all constants from the table above
- [ ] `ui/mod.rs` exports the module
- [ ] Each constant is a `pub const &str`
- [ ] `cargo clippy --workspace --all-targets` passes
- [ ] No other files are modified (this is additive only)

---

### P1-US-003: Add Format Helper Functions (`format_warning`, `format_error`, `format_success`)

**Description:** Add `format_warning()`, `format_error()`, and `format_success()` functions to `crates/treb-cli/src/output.rs` that produce output identical to the Go `render/helpers.go` implementations.

**Changes:**
- `crates/treb-cli/src/output.rs` — Add three new public functions

**Go behavior to match:**
- `FormatWarning(message)`: Splits on `: `, takes last part. Special-cases "already exists" and "does not exist" tag messages. Falls back to `⚠️  {msg}` in yellow.
- `FormatError(message)`: Splits on `: `, takes last part, capitalizes first letter. Returns `❌ {msg}` in red.
- `FormatSuccess(message)`: Returns `✅ {msg}` in green.

**Acceptance Criteria:**
- [ ] `format_warning("tag error: tag 'v1' already exists")` returns yellow-styled `⚠️  Deployment already has tag 'v1'`
- [ ] `format_warning("tag error: tag 'v1' does not exist")` returns yellow-styled `⚠️  Deployment doesn't have tag 'v1'`
- [ ] `format_warning("some warning")` returns yellow-styled `⚠️  some warning`
- [ ] `format_error("cli error: something failed")` returns red-styled `❌ Something failed` (note: capitalized)
- [ ] `format_success("done")` returns green-styled `✅ done`
- [ ] Each function respects `is_color_enabled()` for styled output
- [ ] Unit tests cover all cases (tag already exists, tag does not exist, fallback warning, error capitalization, success)
- [ ] `cargo clippy --workspace --all-targets` passes

---

### P1-US-004: Add `format_duration()` and `format_build_date()` Helpers

**Description:** Add two formatting helpers to `crates/treb-cli/src/output.rs` matching the Go `formatDuration` (from `render/fork.go:84-92`) and `formatBuildDate` (from `version.go:39-52`).

**Changes:**
- `crates/treb-cli/src/output.rs` — Add two new public functions

**Go behavior to match:**
- `formatDuration(d)`:
  - `d < 1 minute` → `"{seconds}s"` (e.g., `"45s"`)
  - `1 minute <= d < 1 hour` → `"{minutes}m{seconds}s"` (e.g., `"5m30s"`)
  - `d >= 1 hour` → `"{hours}h{minutes}m"` (e.g., `"2h15m"`)
- `formatBuildDate(date)`:
  - Input like `"2025-01-26T15:04:05Z"` → `"2025-01-26 15:04:05 UTC"`
  - If not ISO 8601 format, return input unchanged

**Acceptance Criteria:**
- [ ] `format_duration(Duration::from_secs(45))` returns `"45s"`
- [ ] `format_duration(Duration::from_secs(330))` returns `"5m30s"`
- [ ] `format_duration(Duration::from_secs(8100))` returns `"2h15m"`
- [ ] `format_duration(Duration::from_secs(0))` returns `"0s"`
- [ ] `format_build_date("2025-01-26T15:04:05Z")` returns `"2025-01-26 15:04:05 UTC"`
- [ ] `format_build_date("unknown")` returns `"unknown"`
- [ ] `format_build_date("2025-01-26")` returns `"2025-01-26"` (no T, returned unchanged)
- [ ] Unit tests cover edge cases
- [ ] `cargo clippy --workspace --all-targets` passes

---

### P1-US-005: Update Verification Badges to Match Go Format

**Description:** Update `crates/treb-cli/src/ui/badge.rs` to use Go-matching verification symbols and colors. The Go code uses `✔︎` (U+2714 + variation selector) for verified (green), `-` for failed (red), `⏳` for pending (yellow), and `-` for missing. The current Rust code uses `V` for verified and `X` for failed.

**Changes:**
- `crates/treb-cli/src/ui/badge.rs` — Update `status_symbol()` and `status_style()` functions, add pending status handling

**Go behavior to match (from `deployments.go:396-449`):**
- Verified: `✔︎` (green, with extra padding space after `]` due to wide character)
- Failed: `-` (red)
- Pending: `⏳` (yellow, with extra padding space after `]` due to wide character)
- Missing/unknown: `-` (dimmed)
- Format with padding: verified/pending get `"{prefix}[{symbol}] "` (trailing space), others get `"{prefix}[{symbol}]"`
- Also update `FORK_BADGE` style constant usage: Go uses `forkIndicatorStyle` which is plain Yellow (not Yellow+Bold)

**Acceptance Criteria:**
- [ ] `verification_badge()` for verified etherscan returns `e[✔︎]` (not `e[V]`)
- [ ] `verification_badge()` for failed sourcify returns `s[-]` (not `s[X]`)
- [ ] `verification_badge_styled()` applies green to verified segments, red to failed
- [ ] Pending status (`"PENDING"`) returns `⏳` symbol in yellow
- [ ] Padding: verified/pending badges include trailing space for wide-character compensation
- [ ] `fork_badge_styled()` uses `FORK_INDICATOR` (Yellow, not Yellow+Bold)
- [ ] Existing unit tests updated to expect new symbols
- [ ] `cargo clippy --workspace --all-targets` passes

---

### P1-US-006: Update Golden Files for Changed Output

**Description:** Run the full test suite, identify golden files that break due to palette/badge changes from US-001 through US-005, and regenerate them with `UPDATE_GOLDEN=1`.

**Changes:**
- Any `.golden` files in `crates/treb-cli/tests/golden/` that reference verification badges (e.g., `e[V]` → `e[✔︎]`), color styles, or fork badge formatting
- Possible files: `list_*`, `show_*`, `verify_*`, `tag_*` golden tests

**Acceptance Criteria:**
- [ ] `cargo test --workspace --all-targets` passes with zero failures
- [ ] `cargo clippy --workspace --all-targets` passes
- [ ] Only golden files with actual output differences are updated (not blanket regeneration)
- [ ] Each updated golden file reflects the new Go-matching format (verified symbols, color changes)

## Functional Requirements

- **FR-1**: The color palette in `ui/color.rs` must define Style constants for every color variable in Go `render/deployments.go:19-34` and `render/script.go:17-23`.
- **FR-2**: An emoji constants module (`ui/emoji.rs`) must provide named `&str` constants for all 26+ emoji characters used across Go render files.
- **FR-3**: `format_warning()` must split error chains on `: `, special-case tag messages, and return yellow-styled output with `⚠️` prefix.
- **FR-4**: `format_error()` must split error chains on `: `, capitalize the first letter of the extracted message, and return red-styled output with `❌` prefix.
- **FR-5**: `format_success()` must return green-styled output with `✅` prefix.
- **FR-6**: `format_duration()` must produce `Xs`, `XmYs`, or `XhYm` format matching Go `formatDuration` exactly.
- **FR-7**: `format_build_date()` must convert ISO 8601 dates (`2025-01-26T15:04:05Z`) to `2025-01-26 15:04:05 UTC` format, passing through non-ISO strings unchanged.
- **FR-8**: Verification badges must use Go symbols: `✔︎` (verified/green), `-` (failed/red), `⏳` (pending/yellow), `-` (missing/dimmed).
- **FR-9**: Wide-character badge symbols (`✔︎`, `⏳`) must include trailing padding space for visual alignment in tables.
- **FR-10**: All formatting functions must respect `NO_COLOR` / `TERM=dumb` / `--no-color` via the existing `is_color_enabled()` mechanism.

## Non-Goals

- **No command output changes yet**: This phase only provides primitives. Actual command rendering changes (list, show, run, etc.) happen in Phases 2-13.
- **No tree or table rendering changes**: Tree connectors and table formatting are Phase 2.
- **No new CLI flags or arguments**: No user-facing CLI changes.
- **No JSON output changes**: JSON output is unaffected by color/emoji primitives.
- **No interactive prompt changes**: Selector, confirmation, and prompt UI are out of scope.
- **No Go code modifications**: The Go CLI is read-only reference material.

## Technical Considerations

- **owo-colors vs fatih/color**: The Go CLI uses `fatih/color` which maps to ANSI codes. `owo-colors` `Style` constants are set at compile time and compose differently. Background colors (Yellow bg, Cyan bg) require `.on_yellow()` / `.on_cyan()` in owo-colors — verify these produce the same ANSI sequences as Go's `color.BgYellow` / `color.BgCyan`.
- **Wide character handling**: The Go code explicitly adds trailing spaces after `✔︎` and `⏳` in badge formatting to compensate for their display width. The Rust `terminal::display_width()` function already exists and handles ANSI stripping — verify it correctly measures these emoji widths.
- **Variation selectors**: `✔︎` is `U+2714` + `U+FE0E` (text presentation selector). Ensure Rust string literals preserve this correctly and it renders as expected in terminal output.
- **Breaking existing tests**: Changing `TYPE_PROXY` from Blue to Magenta and badge symbols from `V`/`X` to `✔︎`/`-` will break golden files that capture styled or plain output. US-006 handles this systematically.
- **`#[allow(dead_code)]`**: Many palette constants will initially be unused until later phases consume them. Keep `#[allow(dead_code)]` annotations to avoid warnings.
- **Backward compatibility**: The `style_for_deployment_type()` function is used by existing list/show commands. Changing its return values (Proxy: Blue→Magenta, Library: Yellow→Blue) will immediately affect those commands' colored output, which is the intended behavior.
