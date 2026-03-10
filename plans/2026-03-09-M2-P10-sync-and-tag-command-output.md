# PRD: Phase 10 - sync and tag Command Output

## Introduction

Phase 10 ports the human-readable output of the `treb sync` and `treb tag` commands to match the Go CLI exactly. The sync command currently uses a comfy_table detail table and aligned key-value metrics; the Go version uses a section-based bullet format with "Syncing registry..." header, bullet metrics, and a `✓` footer. The tag command currently uses vertical bullet lists for tags; the Go version uses comma-separated inline cyan tags with deployment context headers (address, namespace/chain/name).

This phase depends on Phase 1 (shared color palette, emoji constants, formatting helpers) and builds on patterns established in Phases 3-9.

## Goals

1. **Sync output matches Go `render/sync.go` exactly** — "Syncing registry..." header, `Safe Transactions:` section with `•` bullet metrics, `Cleanup:` section, yellow `Warnings:` section, green `✓` footer.
2. **Tag show output matches Go `render/tag.go`** — cyan bold `Deployment:` header with NS/CHAINID/NAME, white bold `Address:` with green bold address, `Tags:` with cyan comma-separated list or faint "No tags".
3. **Tag add/remove output matches Go** — green `✅` success message with deployment path, `Current tags:` / `Remaining tags:` with cyan comma-separated inline tags.
4. **All golden tests pass** — sync and tag golden expected files updated, no cross-command golden drift.
5. **JSON output unchanged** — no changes to `--json` output schema or behavior.

## User Stories

### P10-US-001: Port sync Command Human Output to Go Section/Bullet Format

**Description:** Replace the current sync human output (comfy_table detail table + aligned key-value metrics + "✅ Sync complete." header) with the Go `render/sync.go` format: "Syncing registry..." header, `Safe Transactions:` section with `•` bullet metrics, `Cleanup:` section, yellow `Warnings:` section, and green `✓` footer.

**File to modify:** `crates/treb-cli/src/commands/sync.rs` (lines 557-626, the human output block)

**Changes:**
1. Print `"Syncing registry..."` as the first line of human output (replaces `"✅ Sync complete."` stage header)
2. Remove the `SyncDetailRow` struct and comfy_table detail table rendering (lines 37-44, 572-584)
3. Replace aligned metric lines with Go bullet sections:
   - When safe txs synced > 0: `"\nSafe Transactions:\n"` (plain text, NOT cyan bold) followed by:
     - `"  • Checked: {synced_count}\n"` (always shown)
     - `"  • Executed: {newly_executed_count}\n"` in green (only when > 0)
     - `"  • Transactions updated: {updated_count}\n"` (only when > 0)
   - When safe txs synced == 0: `"No pending Safe transactions found\n"`
4. Governor proposals section (Rust extension, adapted to Go style):
   - When gov synced > 0: `"\nGovernor Proposals:\n"` followed by same bullet pattern:
     - `"  • Checked: {gov_synced_count}\n"`
     - `"  • Executed: {gov_newly_executed_count}\n"` in green (only when > 0)
     - `"  • Proposals updated: {gov_updated_count}\n"` (only when > 0)
5. Cleanup section: when `removed_count + gov_removed_count > 0`: `"\nCleanup:\n"` + `"  • Entries removed: {total_removed}\n"`
6. Warnings section: when errors non-empty: yellow `"\nWarnings:\n"` + `"  • {err}\n"` per error
7. Footer: blank line, then green `"✓ Registry synced successfully"` (no errors) or green `"✓ Registry sync completed with warnings"` (with errors) — uses `✓` (U+2713 `emoji::CHECK_MARK`), NOT `✅` (U+2705)
8. Move the `"🏛️ Syncing governor proposals..."` progress line to print before the governor sync operation (if not already), keeping it as a progress indicator separate from the result rendering

**Acceptance criteria:**
- `cargo clippy --workspace --all-targets` passes with no new warnings
- `cargo test -p treb-cli -- sync` compiles (unit tests may fail until P10-US-005)
- Human output for sync with safe txs matches Go bullet format
- Human output for sync with no pending txs shows "No pending Safe transactions found"
- Warnings section is yellow
- Footer uses `✓` character in green
- JSON output (`--json`) is completely unchanged

---

### P10-US-002: Port tag show Human Output to Go Deployment Context Format

**Description:** Replace the current tag show output ("No tags on '...'" / "Tags for '...': - tag") with Go `render/tag.go` format showing deployment context with address and comma-separated inline tags.

**File to modify:** `crates/treb-cli/src/commands/tag.rs` (function `show_tags`, lines 80-101)

**Changes to `show_tags()`:**
1. Add blank line before output (`println!()`)
2. Print cyan bold `"Deployment: {deployment_id}\n"` — uses `color::STAGE`
3. Print white bold `"Address: "` + green bold `"{address}\n"` — access `dep.address` from the deployment object
4. Print white bold `"Tags:    "` (8-char alignment with Address label) followed by:
   - If tags empty: faint "No tags" using `color::GRAY`
   - If tags non-empty: sort tags alphabetically, print cyan comma-separated inline (e.g., `"tag1, tag2, tag3"`) using `color::CYAN` for each tag with `", "` separator in plain text
5. Add trailing blank line (`println!()`)
6. Remove old format: "No tags on '...'" and "Tags for '...':  - tag" patterns

**Acceptance criteria:**
- `cargo clippy --workspace --all-targets` passes
- Tag show with no tags displays: blank line, "Deployment: ..." cyan bold, "Address: ..." green bold, "Tags:    No tags" faint, blank line
- Tag show with tags displays comma-separated cyan tags inline after "Tags:    "
- Tags are sorted alphabetically for consistent display
- JSON output unchanged

---

### P10-US-003: Port tag add Human Output to Go Format

**Description:** Replace the current tag add output ("📝 Updating tags..." + "Added tag '...' to '...'" + "Tags: - tag") with Go format: green `✅` success line with deployment path, then "Current tags:" with inline comma-separated cyan tags.

**File to modify:** `crates/treb-cli/src/commands/tag.rs` (function `add_tag`, lines 103-143)

**Changes to `add_tag()`:**
1. Remove the `"📝 Updating tags..."` stage line (`output::print_stage(...)` call at line 117)
2. Change success message to green `"✅ Added tag '{tag}' to {deployment_id}\n"` — uses `color::GREEN` (plain green, matching Go `color.FgGreen`)
3. Replace vertical "Tags:" list with: `"\nCurrent tags: "` (plain text) followed by cyan comma-separated sorted tags inline
4. Sort tags alphabetically before display (matching Go `sort.Strings(allTags)`)

**Acceptance criteria:**
- `cargo clippy --workspace --all-targets` passes
- No "📝 Updating tags..." line in output
- Success line is green with ✅ emoji and deployment path
- Tags displayed as "Current tags: tag1, tag2" with cyan tag names, comma-separated on one line
- Tags are sorted alphabetically
- JSON output unchanged

---

### P10-US-004: Port tag remove Human Output to Go Format

**Description:** Replace the current tag remove output ("📝 Updating tags..." + "Removed tag '...' from '...'" + "Tags: - tag" or "No tags remaining") with Go format: green `✅` success line, then "Remaining tags:" with inline comma-separated cyan tags or faint "No tags".

**File to modify:** `crates/treb-cli/src/commands/tag.rs` (function `remove_tag`, lines 145-194)

**Changes to `remove_tag()`:**
1. Remove the `"📝 Updating tags..."` stage line (`output::print_stage(...)` call at line 159)
2. Change success message to green `"✅ Removed tag '{tag}' from {deployment_id}\n"` — uses `color::GREEN` (plain green)
3. Replace vertical tag list / "No tags remaining" with: `"\nRemaining tags: "` (plain text) followed by:
   - If no tags left: faint "No tags" using `color::GRAY` (matching Go `color.Faint`)
   - If tags remain: cyan comma-separated sorted tags inline
4. Sort remaining tags alphabetically before display

**Acceptance criteria:**
- `cargo clippy --workspace --all-targets` passes
- No "📝 Updating tags..." line in output
- Success line is green with ✅ emoji and deployment path
- When tags remain: "Remaining tags: tag1, tag2" with cyan names
- When no tags: "Remaining tags: No tags" with faint styling
- JSON output unchanged

---

### P10-US-005: Update Unit Tests for sync and tag Output Changes

**Description:** Update the unit tests in `sync.rs` and `tag.rs` that assert on human output strings to match the new Go-matching format.

**Files to modify:**
- `crates/treb-cli/src/commands/sync.rs` (test module, lines 633+) — update any tests that assert on human output format
- `crates/treb-cli/src/commands/tag.rs` (test module, lines 196+) — update tests that assert on human output strings (e.g., `show_tags_empty`, `show_tags_with_existing`, `add_tag_success`, `remove_tag_success`, `remove_last_tag_sets_none`)

**Changes:**
1. Review all unit tests in both files for assertions on stdout/stderr output strings
2. Update expected output strings to match the new Go-matching format:
   - Sync: bullet format with `•`, section headers, `✓` footer
   - Tag show: deployment header, address, comma-separated tags
   - Tag add: `✅ Added...` + `Current tags:` inline
   - Tag remove: `✅ Removed...` + `Remaining tags:` inline
3. Add `NO_COLOR=1` env where tests do plain-text `contains()` assertions on styled output (if not already set)
4. Remove any assertions that reference removed elements (detail table, "📝 Updating tags...", "Tags:" bullet list)

**Acceptance criteria:**
- `cargo test -p treb-cli --lib -- sync` passes (all sync unit tests)
- `cargo test -p treb-cli --lib -- tag` passes (all tag unit tests)
- `cargo clippy --workspace --all-targets` passes
- No dead code warnings from removed helpers (e.g., `SyncDetailRow` if removed)

---

### P10-US-006: Update All Golden Test Expected Files

**Description:** Regenerate golden test expected files for sync and tag commands, then verify no cross-command golden drift.

**Files to update:** Golden expected files in `crates/treb-cli/tests/golden/sync_*/` and `crates/treb-cli/tests/golden/tag_*/` directories.

**Steps:**
1. Run `UPDATE_GOLDEN=1 cargo test -p treb-cli --test integration_sync --test integration_tag` to regenerate sync and tag golden files
2. Review the diffs to confirm they match expected format changes:
   - Sync golden files: "Syncing registry..." header, bullet metrics, `✓` footer
   - Tag golden files: deployment context headers, comma-separated tags, no "📝 Updating tags..."
3. Run full `cargo test -p treb-cli` to check for cross-command golden drift
4. If other golden files broke, run targeted `UPDATE_GOLDEN=1` for affected test binaries and verify diffs are benign

**Acceptance criteria:**
- `UPDATE_GOLDEN=1 cargo test -p treb-cli --test integration_sync --test integration_tag` passes
- All golden file diffs match expected output format changes (no unexpected regressions)
- `cargo test -p treb-cli` passes (full CLI test suite, no cross-command failures)
- `cargo test --workspace --all-targets` passes (full workspace)

## Functional Requirements

- **FR-1:** Sync human output prints "Syncing registry..." as the first line, followed by section-based bullet metrics, and a green `✓` footer.
- **FR-2:** Sync `Safe Transactions:` section uses `•` bullets with conditional display: Checked (always), Executed in green (when > 0), Transactions updated (when > 0).
- **FR-3:** When no safe transactions are pending, sync outputs "No pending Safe transactions found".
- **FR-4:** Sync governor proposals section follows same bullet pattern as safe transactions (Rust extension of Go format).
- **FR-5:** Sync cleanup section shows removed entry count when applicable.
- **FR-6:** Sync warnings section renders in yellow with `•` bullet per warning.
- **FR-7:** Sync footer uses `✓` (U+2713) character in green — "Registry synced successfully" or "Registry sync completed with warnings".
- **FR-8:** Tag show displays deployment context: cyan bold `Deployment: NS/CHAINID/NAME`, white bold `Address:` + green bold address, `Tags:` with cyan comma-separated sorted list or faint "No tags".
- **FR-9:** Tag add displays green `✅ Added tag 'TAG' to NS/CHAINID/NAME` followed by `Current tags:` with cyan comma-separated sorted tags.
- **FR-10:** Tag remove displays green `✅ Removed tag 'TAG' from NS/CHAINID/NAME` followed by `Remaining tags:` with cyan comma-separated sorted tags or faint "No tags".
- **FR-11:** Tag add and remove no longer print `"📝 Updating tags..."` progress line.
- **FR-12:** All tag displays sort tags alphabetically for deterministic output.
- **FR-13:** JSON output (`--json`) for both sync and tag commands remains completely unchanged.

## Non-Goals

- **No JSON schema changes** — this phase only modifies human-readable output, not `--json` output.
- **No new sync features** — no new metrics, no verbose mode changes, no new flags.
- **No tag command logic changes** — tag add/remove/show logic stays the same, only display format changes.
- **No comfy_table replacement for other commands** — only sync's detail table is affected.
- **No integration test logic changes** — only golden expected files and unit test assertions are updated.
- **No changes to governor proposal sync behavior** — only the output format of governor results changes.

## Technical Considerations

### Dependencies
- **Phase 1 artifacts:** `emoji::CHECK_MARK` (✓), `emoji::CHECK` (✅), `color::GREEN`, `color::SUCCESS`, `color::STAGE`, `color::GRAY`, `color::WARNING`, `color::CYAN`
- **Existing helpers:** `styled()` pattern (conditional color application based on `color::is_color_enabled()`) — already present in tag.rs, may need addition in sync.rs

### Key Patterns from Previous Phases
- `styled(text, style)` for conditional coloring — established in show.rs (P5), verify.rs (P7), tag.rs
- `color::GREEN` (plain green, Go `FgGreen`) vs `color::SUCCESS` (green+bold) — use GREEN for tag add/remove success, match Go reference
- `color::CYAN` (plain) for tag text, `color::STAGE` (cyan+bold) for section headers — don't confuse
- Sort dynamic lists for deterministic output (established in P7 verify)
- Golden test framework trims trailing blank lines from `println!()` — always finalize with `UPDATE_GOLDEN=1`
- Run full `cargo test -p treb-cli` after golden updates to catch cross-command drift

### Go Output Reference

**Sync output structure:**
```
Syncing registry...

Safe Transactions:
  • Checked: N
  • Executed: N          ← green, only when > 0
  • Transactions updated: N   ← only when > 0

Cleanup:                 ← only when entries removed
  • Invalid entries removed: N

Warnings:                ← yellow, only when errors exist
  • error message

✓ Registry synced successfully       ← green
  OR
✓ Registry sync completed with warnings  ← green
```

**Tag show output structure:**
```
                         ← blank line
Deployment: NS/CHAINID/NAME    ← cyan bold
Address: 0xADDR                 ← "Address:" white bold, addr green bold
Tags:    tag1, tag2, tag3       ← "Tags:" white bold, tags cyan comma-separated
                         ← blank line
```

**Tag add output:**
```
✅ Added tag 'TAG' to NS/CHAINID/NAME    ← green

Current tags: tag1, tag2, tag3           ← tags in cyan
```

**Tag remove output:**
```
✅ Removed tag 'TAG' from NS/CHAINID/NAME    ← green

Remaining tags: tag1, tag2        ← tags in cyan
  OR
Remaining tags: No tags           ← "No tags" in faint
```

### Potential Risks
- The `SyncDetailRow` struct and comfy_table usage in sync.rs may be referenced by unit tests — removing it requires updating all references
- Tag show now needs `dep.address` which is already available via `registry.get_deployment()` — no schema change needed
- The Go sync `Safe Transactions:` header is plain text (not styled) — the master plan description says "cyan bold" but the actual Go code uses plain `fmt.Fprintf`, so follow the Go code as ground truth
