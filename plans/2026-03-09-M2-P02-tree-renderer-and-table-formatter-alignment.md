# PRD: Phase 2 - Tree Renderer and Table Formatter Alignment

## Introduction

Phase 2 aligns the tree rendering and table formatting infrastructure in the Rust CLI to produce output identical to the Go CLI. The Go CLI uses Unicode box-drawing characters (`├─`, `└─`, `│`) for tree connectors and `go-pretty/v6/table` with `StyleLight` for deployment tables (no borders, 3-space right padding, fixed column widths computed globally). The Rust CLI currently uses ASCII tree connectors (`|--`, `\--`, `|`) and `comfy_table` with `UTF8_FULL` borders and dynamic column widths.

This phase builds the shared infrastructure that Phase 4 (list command), Phase 6 (run command), and other downstream phases depend on. It does NOT rewrite any command's display logic — it provides the building blocks.

**Phase 1 delivered:** Color palette (`ui/color.rs`), emoji constants (`ui/emoji.rs`), format helpers (`format_warning`, `format_error`, `format_success`, `format_duration`, `format_build_date`), and verification badges.

## Goals

1. **Tree parity**: `ui/tree.rs` produces connector characters identical to Go's `buildTreePrefix` — `├─ `, `└─ `, `│  `, `   ` — matching character-for-character.
2. **Table parity**: A new `render_table_with_widths()` function produces output matching Go's `renderTableWithWidths` — no borders, no separators, 3-space right padding, fixed column widths, continuation prefix on first column.
3. **Accurate width measurement**: ANSI stripping and Unicode-aware display width utilities enable correct column alignment for cells containing color codes and wide characters (e.g., `✔︎`, `⏳`).
4. **Golden file consistency**: All golden files affected by connector character changes are updated and pass.

## User Stories

### P2-US-001 — Update Tree Connector Characters to Unicode Box-Drawing

**Description:** Replace ASCII tree connectors in `ui/tree.rs` with Unicode box-drawing characters matching Go's `buildTreePrefix` from `transaction.go` lines 284-301. The Go CLI uses:
- Non-last child: `├─ ` (U+251C U+2500 U+0020)
- Last child: `└─ ` (U+2514 U+2500 U+0020)
- Vertical continuation (non-last parent): `│  ` (U+2502 U+0020 U+0020)
- Empty continuation (last parent): `   ` (3 spaces)

**Files to modify:**
- `crates/treb-cli/src/ui/tree.rs` — update `render_children_plain()` and `render_children_styled()` connector strings and continuation prefixes

**Acceptance criteria:**
- `render()` produces `├─ ` for non-last children and `└─ ` for last children (not `|-- ` and `\-- `)
- Continuation prefixes are `│  ` (pipe + 2 spaces) and `   ` (3 spaces), not `|   ` (pipe + 3 spaces) and `    ` (4 spaces)
- `render_styled()` produces identical connectors (ANSI codes only on labels, not connectors)
- All existing `tree.rs` unit tests updated and passing with new connectors
- Integration tests in `ui/mod.rs` updated and passing
- `cargo test -p treb-cli` passes
- `cargo clippy --workspace --all-targets` clean

**Go reference:** `internal/cli/render/transaction.go` lines 284-301

---

### P2-US-002 — Add ANSI Code Stripping Utility

**Description:** Add a `strip_ansi_codes()` function matching Go's `stripAnsiCodes` from `deployments.go` lines 513-518. This is critical because `comfy_table` (and manual width calculations) count ANSI escape codes in column width, causing styled columns to be wider than visible text. Phase 1 already encountered this issue with four fork golden files.

**Files to modify:**
- `crates/treb-cli/src/output.rs` — add `pub fn strip_ansi_codes(s: &str) -> String` using regex `\x1b\[[0-9;]*[mGKHF]`

**Dependencies:** Add `regex` to `treb-cli` dependencies if not already present (check workspace Cargo.toml — it's likely already available since `tree.rs` tests use it).

**Acceptance criteria:**
- `strip_ansi_codes("\x1b[31mred\x1b[0m")` returns `"red"`
- `strip_ansi_codes("no codes")` returns `"no codes"`
- `strip_ansi_codes("")` returns `""`
- Handles nested/multiple codes: `strip_ansi_codes("\x1b[1m\x1b[31mtext\x1b[0m")` → `"text"`
- Handles codes with multiple parameters: `\x1b[38;5;196m` (256-color), `\x1b[0;1;31m` (combined)
- At least 5 unit tests covering edge cases
- Typecheck passes: `cargo check -p treb-cli`

**Go reference:** `internal/cli/render/deployments.go` lines 513-518

---

### P2-US-003 — Add Unicode-Aware Display Width Function

**Description:** Add a `display_width()` function matching Go's `displayWidth` from `deployments.go` lines 566-569 which uses `go-runewidth`. This is needed for the verification column (column index 2) where Unicode characters like `✔︎` (U+2714+FE0E) and `⏳` (U+231B) have display widths different from their byte or char count. The `unicode-width` crate (already a workspace dependency) provides the equivalent of `go-runewidth`.

**Files to modify:**
- `crates/treb-cli/src/output.rs` — add `pub fn display_width(s: &str) -> usize` using `unicode_width::UnicodeWidthStr`

**Acceptance criteria:**
- `display_width("hello")` returns `5`
- `display_width("✔︎")` returns correct display width matching `go-runewidth` output
- `display_width("⏳")` returns correct display width matching `go-runewidth` output
- `display_width("")` returns `0`
- `display_width("e[✔︎]  s[-] b[-]")` returns the same width that Go's `runewidth.StringWidth` would
- Input must be ANSI-stripped before calling (function does not strip ANSI itself)
- At least 4 unit tests
- Typecheck passes: `cargo check -p treb-cli`

**Go reference:** `internal/cli/render/deployments.go` lines 566-569

---

### P2-US-004 — Add Global Column Width Calculator and Table Renderer

**Description:** Add two functions matching Go's `calculateTableColumnWidths` (lines 521-564) and `renderTableWithWidths` (lines 466-511):

1. `calculate_column_widths(tables: &[TableData]) -> Vec<usize>` — iterates all tables, strips ANSI, uses `display_width` for column 2 (verification) and `len` for others, returns max width per column.

2. `render_table_with_widths(table: &TableData, widths: &[usize], continuation_prefix: &str) -> String` — renders table rows with:
   - No borders, no row separators, no column separators, no header separator
   - 3-space right padding between columns
   - Fixed column widths from `widths` parameter
   - First column width adjusted: `width + 2 + continuation_prefix.chars().count()` (matching Go line 485)
   - `continuation_prefix` prepended to first cell of each row
   - Left-aligned text, padded to column width (using display_width-aware padding for column 2)

Define `pub type TableData = Vec<Vec<String>>` (a table is a list of rows, each row is a list of cell strings).

**Files to modify:**
- `crates/treb-cli/src/output.rs` — add `TableData` type alias, `calculate_column_widths()`, and `render_table_with_widths()`

**Note:** The existing `build_table()` function is NOT modified or removed. It continues to work for commands that haven't been ported yet. The new functions are additional infrastructure for Phase 4 and Phase 6.

**Acceptance criteria:**
- `calculate_column_widths` returns correct widths across multiple tables with ANSI-coded cells
- `calculate_column_widths` uses `display_width` for column index 2 and byte length for others
- `calculate_column_widths(&[])` returns empty vec
- `render_table_with_widths` produces rows with no border characters (no `│`, `─`, `┌`, `└`, etc.)
- `render_table_with_widths` has exactly 3 spaces between columns (right padding)
- `render_table_with_widths` prepends `continuation_prefix` to each row's first cell
- `render_table_with_widths` adjusts first column width by `2 + len(continuation_prefix)`
- `render_table_with_widths` pads cells to fixed width (left-aligned)
- `render_table_with_widths` with empty table data returns `""`
- At least 6 unit tests covering: empty input, single table, multiple tables, ANSI cells, continuation prefix, width adjustment
- Typecheck passes: `cargo check -p treb-cli`

**Go reference:**
- `internal/cli/render/deployments.go` lines 466-511 (`renderTableWithWidths`)
- `internal/cli/render/deployments.go` lines 521-564 (`calculateTableColumnWidths`)

---

### P2-US-005 — Update Golden Files for Tree Connector Changes

**Description:** US-001 changes tree connectors from ASCII to Unicode, which will cause golden file mismatches for any tests that render tree output. Identify all affected golden files, update them with `UPDATE_GOLDEN=1`, and verify they pass.

**Files to modify:**
- `crates/treb-cli/tests/golden/list_*/commands.golden` — any list tests that show tree output
- `crates/treb-cli/tests/golden/run_*/commands.golden` — any run tests that show deployment tree output
- Any other golden files containing `|-- `, `\-- `, `|   ` tree connectors

**Process:**
1. Run `cargo test -p treb-cli` to identify all failures
2. For each failure, verify the diff is ONLY connector character changes (`|--` → `├─`, `\--` → `└─`, `|   ` → `│  `)
3. Run `UPDATE_GOLDEN=1 cargo test -p treb-cli` to regenerate
4. Review each updated `.golden` file to confirm only expected changes

**Acceptance criteria:**
- `cargo test -p treb-cli` passes with zero failures
- Every updated golden file diff shows ONLY tree connector character changes (no unintended formatting changes)
- No golden files are deleted — only updated
- `cargo clippy --workspace --all-targets` clean

---

## Functional Requirements

- **FR-1**: Tree connector characters match Go exactly: `├─ ` (non-last), `└─ ` (last), `│  ` (continuation), `   ` (last-parent continuation)
- **FR-2**: `strip_ansi_codes` removes all ANSI escape sequences matching regex `\x1b\[[0-9;]*[mGKHF]`
- **FR-3**: `display_width` returns Unicode-aware display width using `unicode-width` crate, matching Go's `go-runewidth`
- **FR-4**: `calculate_column_widths` computes global max width per column across multiple table data sets, using ANSI stripping and display width for column 2
- **FR-5**: `render_table_with_widths` produces go-pretty StyleLight output: no borders/separators, 3-space right padding, fixed widths, continuation prefix on first column
- **FR-6**: `render_table_with_widths` adjusts first column width by `2 + len(continuation_prefix)` matching Go line 485
- **FR-7**: Existing `build_table()` function remains unchanged and continues working for unported commands
- **FR-8**: All golden files pass after tree connector updates

## Non-Goals

- **Not rewriting command display logic** — Phase 4 (list), Phase 6 (run), and other phases will update individual commands to use the new infrastructure
- **Not modifying namespace/chain header rendering** — the `◎ namespace:` and `⛓ chain:` formatted headers are part of the list command output (Phase 4), not the tree renderer infrastructure
- **Not removing comfy_table** — it remains for commands not yet ported; removal happens when all commands are updated
- **Not implementing deployment row formatting** — colored contract names, verification badges in table cells, fork indicators, tags — these are Phase 4 deliverables
- **Not changing JSON output** — Phase 14 handles JSON schema parity

## Technical Considerations

### Dependencies
- **Phase 1 (completed)**: Color palette, emoji constants, format helpers are available in `ui/color.rs`, `ui/emoji.rs`, `output.rs`
- **regex crate**: Already used in `tree.rs` tests (`regex::Regex`). Verify it's in `treb-cli` dependencies; add if not
- **unicode-width crate**: Already a workspace dependency (`unicode-width = "0.2"` in root `Cargo.toml`, `unicode-width = { workspace = true }` in `treb-cli/Cargo.toml`)

### Key Design Decision: New Functions vs. Replacing build_table()
The existing `build_table()` uses `comfy_table` with `UTF8_FULL` preset and borders. The new `render_table_with_widths()` is a fundamentally different rendering approach (manual string formatting with fixed widths, no library table). Both coexist until all commands are ported in later phases.

### Width Calculation Nuance
Go's `calculateTableColumnWidths` uses `len(stripped)` (byte length) for most columns but `displayWidth(stripped)` (Unicode-aware width via `go-runewidth`) specifically for column index 2 (the verification column). This is because verification badges contain wide Unicode characters (`✔︎`, `⏳`) whose display width differs from byte count. The Rust implementation must replicate this column-specific behavior.

### Connector Width Change Impact
The Go tree connectors are 3 characters wide per level (`├─ `, `└─ `, `│  `, `   `), while the current Rust connectors are 4 characters wide (`|-- `, `\-- `, `|   `, `    `). This changes the indentation of all tree-rendered content, affecting golden file output width. All affected golden files must be updated.

### comfy_table ANSI Issue (Phase 1 Learning)
Phase 1 discovered that `comfy_table`'s `ContentArrangement::Dynamic` counts ANSI escape codes in column width calculations, making styled columns wider than intended. The new `render_table_with_widths` function avoids this by manually computing widths after ANSI stripping, matching Go's approach exactly.
