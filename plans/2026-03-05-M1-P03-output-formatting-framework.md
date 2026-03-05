# PRD: Phase 3 - Output Formatting Framework

## Introduction

This phase builds the shared output formatting framework that all subsequent command output phases (4-10) depend on. The Go CLI uses `fatih/color` + `go-pretty` tables with styled backgrounds for namespaces/chains, tree characters for deployment hierarchy, and compact verification badges. The Rust CLI already has basic output utilities (`print_kv`, `build_table`, `truncate_address` in `output.rs`) and a color palette with `NO_COLOR`/`TERM=dumb` support (`ui/color.rs`). This phase extends that foundation with tree rendering, a Go-matching color palette, badge formatters, and terminal-width-aware layout — all as reusable building blocks.

## Goals

1. **Tree rendering**: Provide a `TreeNode` API that renders hierarchical data (namespace > chain > type > deployment > implementation) using `|--`, `\--`, `|` prefix characters, consumable by `list`, `show`, `compose`, and `fork` commands.

2. **Go-matching color palette**: Extend `ui/color.rs` with deployment-specific styles — namespace (yellow bg), chain (cyan bg), type colors (magenta=proxy, blue=library, green=singleton) — so downstream phases apply them without per-command color logic.

3. **Badge formatters**: Implement compact verification badges (`e[V] s[-] b[-]`) and fork indicators (`[fork]`) with per-status coloring, replacing the current verbose `UNVERIFIED`/`VERIFIED` text in table columns.

4. **Terminal-width-aware output**: Detect terminal width (with graceful 80-col fallback for non-TTY/piped output) so tables and tree output can adapt to available space.

5. **Unit test coverage**: Every formatter module has tests covering normal output, edge cases, and `NO_COLOR` behavior; no golden file changes in this phase (those happen when commands adopt the framework in Phases 4+).

## User Stories

### P3-US-001: Tree Renderer - TreeNode Struct and Render Logic

**Description:** Create a tree rendering module at `crates/treb-cli/src/ui/tree.rs` with a `TreeNode` struct that holds a label and children, and a `render()` method producing indented lines with box-drawing prefixes.

**Details:**
- `TreeNode { label: String, children: Vec<TreeNode> }` with builder methods (`new`, `child`, `children`)
- `render(&self) -> Vec<String>` produces lines: root has no prefix, intermediate children get `|-- `, last child gets `\-- `, nested continuation uses `|   ` or `    `
- The renderer is pure (no stdout writes) — callers iterate the returned lines and print them
- Register the module in `ui/mod.rs`

**Acceptance Criteria:**
- `TreeNode::new("root").child(TreeNode::new("a")).child(TreeNode::new("b")).render()` produces:
  ```
  root
  |-- a
  \-- b
  ```
- Three-level nesting works: root > mid > leaf with correct `|   ` continuation
- Empty children list renders just the label line
- `cargo check -p treb-cli` passes

---

### P3-US-002: Tree Renderer - Styled Labels and Color Integration

**Description:** Extend `TreeNode` to accept an optional `Style` (from `owo-colors`) so labels can be rendered with color when color is enabled. Add a `render_styled(&self) -> Vec<String>` method that applies ANSI styling.

**Details:**
- Add `style: Option<owo_colors::Style>` field to `TreeNode`, with `with_style(style)` builder
- `render()` remains plain-text (no ANSI); `render_styled()` applies the style to the label portion only (not the tree prefix characters)
- When `owo_colors::is_supported()` returns false (due to `NO_COLOR` or override), `render_styled()` falls back to plain text automatically (owo-colors handles this internally)

**Acceptance Criteria:**
- `render_styled()` output contains ANSI escape codes around labels when color is enabled
- `render()` output never contains ANSI escape codes
- Tree prefix characters (`|-- `, `\-- `, `|   `) are never colored
- Unit test verifying styled vs plain output
- `cargo check -p treb-cli` passes

---

### P3-US-003: Deployment Color Palette

**Description:** Extend `ui/color.rs` with deployment-specific style constants matching the Go CLI palette. Keep the existing `STAGE`, `SUCCESS`, `WARNING`, `ERROR`, `MUTED` constants.

**Details:**
- Add constants:
  - `NAMESPACE`: yellow background (matches Go `fatih/color` `BgYellow`)
  - `CHAIN`: cyan background (matches Go `BgCyan`)
  - `TYPE_PROXY`: magenta text
  - `TYPE_LIBRARY`: blue text
  - `TYPE_SINGLETON`: green text
  - `TYPE_UNKNOWN`: default/dimmed text
  - `ADDRESS`: cyan text (for hex addresses)
  - `LABEL`: dimmed text
  - `FORK_BADGE`: yellow text bold
  - `VERIFIED`: green text
  - `FAILED`: red text
  - `UNVERIFIED`: dimmed text
- Add `fn style_for_deployment_type(dtype: &DeploymentType) -> Style` helper that maps enum variants to the corresponding type style
- Keep all constants as `pub const` using `Style::new().*()` chain (compile-time constructable)

**Acceptance Criteria:**
- `style_for_deployment_type(DeploymentType::Proxy)` returns `TYPE_PROXY`
- All new constants are `pub` and accessible from other modules
- Existing constants (`STAGE`, `SUCCESS`, etc.) are unchanged
- `cargo check -p treb-cli` passes
- Unit test verifying the mapping function covers all `DeploymentType` variants

---

### P3-US-004: Verification Badge Formatter

**Description:** Create a badge formatting module at `crates/treb-cli/src/ui/badge.rs` that renders compact verification status strings in the `e[V] s[-] b[-]` format used by the Go CLI.

**Details:**
- `fn verification_badge(verifiers: &HashMap<String, VerifierStatus>) -> String` produces a compact badge string
- Verifier keys: `"etherscan"` -> `e`, `"sourcify"` -> `s`, `"blockscout"` -> `b`
- Status indicators: verified -> `[V]`, failed -> `[X]`, unverified/missing -> `[-]`
- When `verifiers` map is empty, return the overall `VerificationStatus` as text (backward compat with current output)
- `fn verification_badge_styled(verifiers: &HashMap<String, VerifierStatus>) -> String` adds color: green for `[V]`, red for `[X]`, dimmed for `[-]`
- Register module in `ui/mod.rs`

**Acceptance Criteria:**
- Empty verifiers map returns `"UNVERIFIED"` (or the passed status)
- Map with etherscan=verified, sourcify=failed, blockscout=missing returns `"e[V] s[X] b[-]"`
- Styled version includes ANSI codes around each badge segment
- Order is always `e s b` regardless of HashMap iteration order
- `cargo check -p treb-cli` passes

---

### P3-US-005: Fork Indicator Badge

**Description:** Add a fork indicator badge formatter to `ui/badge.rs` that produces the `[fork]` badge for fork-namespace deployments.

**Details:**
- `fn fork_badge(namespace: &str) -> Option<String>` returns `Some("[fork]")` if namespace starts with `"fork/"`, else `None`
- `fn fork_badge_styled(namespace: &str) -> Option<String>` returns the badge with `FORK_BADGE` style applied
- These are simple helpers but centralizing them ensures consistent detection logic across `list`, `show`, `fork status`, etc.

**Acceptance Criteria:**
- `fork_badge("fork/42220")` returns `Some("[fork]")`
- `fork_badge("mainnet")` returns `None`
- Styled version includes yellow bold ANSI when color enabled
- `cargo check -p treb-cli` passes

---

### P3-US-006: Terminal Width Detection and Column Width Utilities

**Description:** Add a terminal module at `crates/treb-cli/src/ui/terminal.rs` with terminal width detection and a unicode-aware string width function for column calculations.

**Details:**
- `fn terminal_width() -> u16` returns the terminal column count; falls back to 80 for non-TTY (piped output, `CI=true`)
- Use `console::Term::stdout().size()` (already a dependency) for width detection
- `fn display_width(s: &str) -> usize` returns the visual column width of a string, accounting for:
  - ANSI escape sequences (zero width — strip before measuring)
  - Common Unicode: tree chars (`│`, `├`, `└`, `─`) are width 1; checkmarks, emoji may be width 2
- Add `unicode-width` crate to workspace dependencies for `UnicodeWidthStr::width()` (the standard solution)
- Strip ANSI codes before measuring using `console::strip_ansi_codes()` (already available via `console` crate)
- Register module in `ui/mod.rs`

**Acceptance Criteria:**
- `terminal_width()` returns a value > 0 in both TTY and non-TTY contexts
- `display_width("hello")` returns 5
- `display_width("\x1b[31mhello\x1b[0m")` returns 5 (ANSI stripped)
- `display_width("|-- label")` returns 9
- `cargo check -p treb-cli` passes

---

### P3-US-007: Integration Test Proving Framework Composes Correctly

**Description:** Write an integration-style unit test in `ui/mod.rs` (or a dedicated `ui/tests.rs`) that exercises all the framework pieces together: builds a tree with styled labels, adds verification and fork badges as node labels, and verifies the composed output.

**Details:**
- Build a `TreeNode` tree simulating a namespace > chain > deployment hierarchy:
  ```
  mainnet (yellow bg)
  |-- celo/42220 (cyan bg)
  |   |-- FPMM (green, singleton) e[V] s[-] b[-]
  |   \-- Proxy (magenta) [fork] e[-] s[-] b[-]
  \-- ethereum/1 (cyan bg)
      \-- Counter (blue, library) e[-] s[-] b[-]
  ```
- Verify that `render()` produces correct plain-text line count and prefix structure
- Verify that `render_styled()` produces ANSI-containing output
- Verify `display_width()` returns correct column widths for styled lines
- This is a unit test, not a golden file CLI test — no subprocess execution needed

**Acceptance Criteria:**
- Test builds a 3-level tree and calls both `render()` and `render_styled()`
- Assertions check line count, prefix characters, and label content
- `display_width()` produces consistent widths for plain and styled versions of the same label
- `cargo test -p treb-cli -- ui` passes
- `cargo check -p treb-cli` passes

## Functional Requirements

- **FR-1:** Tree renderer produces `|-- `, `\-- `, `|   `, `    ` prefixed lines matching the Go CLI hierarchy style.
- **FR-2:** Tree renderer supports arbitrary nesting depth (not limited to 3 levels).
- **FR-3:** Color palette includes all deployment-specific styles: namespace bg, chain bg, per-type text color, badge colors.
- **FR-4:** `style_for_deployment_type()` maps all four `DeploymentType` variants to distinct styles.
- **FR-5:** Verification badges render in fixed `e[V] s[-] b[-]` order with status-appropriate indicators.
- **FR-6:** Fork badge detection uses `namespace.starts_with("fork/")` consistently.
- **FR-7:** Terminal width detection gracefully falls back to 80 columns when stdout is not a TTY.
- **FR-8:** `display_width()` strips ANSI escape codes before measuring and handles Unicode correctly.
- **FR-9:** All styled output respects `NO_COLOR` env var and `TERM=dumb` via existing `owo-colors` override mechanism.
- **FR-10:** All new modules are registered in `ui/mod.rs` as public submodules.

## Non-Goals

- **No command output changes in this phase.** The `list`, `show`, `fork`, `run` commands will adopt the framework in Phases 4-8. This phase only builds and tests the reusable utilities.
- **No golden file updates.** Since no command output changes, no golden files need regeneration.
- **No table rendering replacement.** The existing `comfy-table` based `build_table()`/`print_table()` stays as-is; it may be augmented or replaced in Phase 4 when `list` output changes.
- **No interactive prompt changes.** The `dialoguer`-based prompts in `ui/prompt.rs` and `ui/selector.rs` are unaffected.
- **No JSON output changes.** The formatting framework is for human-readable output only; `--json` paths are unchanged.

## Technical Considerations

### Dependencies
- **`unicode-width`**: Add to workspace `Cargo.toml` — standard crate for `UnicodeWidthStr::width()`. Lightweight, no transitive deps.
- **`console` (existing)**: Already used for TTY detection in selectors. Provides `Term::stdout().size()` for terminal dimensions and `strip_ansi_codes()` for ANSI stripping.
- **`owo-colors` (existing)**: Already used for `Style` constants. The `supports-colors` feature is enabled, so `set_override(false)` works globally.
- **No new heavy dependencies.** All functionality builds on existing crates.

### Integration Points
- `ui/color.rs` depends on `treb_core::types::DeploymentType` for the `style_for_deployment_type()` function. This is the only cross-crate dependency introduced.
- `ui/badge.rs` depends on `treb_core::types::VerifierStatus` for reading per-verifier status.
- The tree renderer is generic and has no domain dependencies — it works with arbitrary string labels.

### File Organization
After this phase, `crates/treb-cli/src/ui/` will contain:
```
ui/
├── mod.rs       (updated: add tree, badge, terminal modules)
├── color.rs     (extended: deployment palette + style_for_deployment_type)
├── prompt.rs    (unchanged)
├── selector.rs  (unchanged)
├── tree.rs      (new: TreeNode struct and render methods)
├── badge.rs     (new: verification badge + fork indicator)
└── terminal.rs  (new: terminal width + display_width)
```

### Patterns from Previous Phases
- Style constants follow the existing `pub const NAME: Style = Style::new().*()` pattern from `color.rs`
- All new `pub fn` helpers are pure functions (no side effects, no stdout) — callers decide when to print
- Tests use the same `unsafe { std::env::set_var(...) }` pattern for env var manipulation as existing `color.rs` tests
