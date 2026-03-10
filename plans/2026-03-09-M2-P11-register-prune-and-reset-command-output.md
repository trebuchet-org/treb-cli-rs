# PRD: Phase 11 - register, prune, and reset Command Output

## Introduction

Phase 11 ports the human-readable output of three registry management commands (`treb register`, `treb prune`, `treb reset`) to match the Go CLI exactly. The register command currently uses a comfy_table plus a summary line; the Go version uses green bold `✓ Successfully registered N deployment(s)` with indented per-deployment details. The prune command currently uses a comfy_table for candidates; the Go version uses emoji-staged output with per-section item listings (Deployments, Transactions, Safe Transactions). The reset command currently uses a warning banner with per-type counts; the Go version uses a `Found N items to reset` header with aligned per-type count lines, a confirmation prompt, and a plain success message.

This phase depends on Phase 1 (shared color palette, emoji constants, formatting helpers) and builds on patterns established in Phases 3-10.

## Goals

1. **Register output matches Go `register.go` lines 383-396 exactly** — green bold `✓ Successfully registered N deployment(s)` header, per-deployment indented details (Deployment N, Deployment ID, Address, Contract, Label).
2. **Prune output matches Go `render/prune.go` + `prune.go` exactly** — `🔍 Checking registry entries...` header, `✅ All registry entries are valid` if clean, `🗑️ Found N items to prune` with per-section listings, `⚠️` confirmation, `🔧 Pruning registry entries...`, `✅ Successfully pruned N items`.
3. **Reset output matches Go `reset.go` lines 48-96 exactly** — `Found N items to reset for namespace 'NS' on network 'NET' (chain CHAINID)` header, aligned per-type count lines, confirmation prompt, plain `Successfully reset N items` success.
4. **All golden tests pass** — register, prune, and reset golden expected files updated, no cross-command golden drift.
5. **JSON output unchanged** — no changes to `--json` output schema or behavior for any command.

## User Stories

### P11-US-001: Port register Command Human Output to Go Indented Format

**Description:** Replace the current register human output (comfy_table with [Contract, Address, Namespace, Chain] columns + `"N deployment(s) registered from transaction 0x..."` summary) with the Go format: green bold `✓` success header + indented per-deployment details.

**File to modify:** `crates/treb-cli/src/commands/register.rs` (lines 596-616, the human output block after `if json { ... } else { ... }`)

**Changes:**
1. Remove the comfy_table creation (`output::build_table(&["Contract", "Address", "Namespace", "Chain"])`) and `output::print_table()` call
2. Print green bold success header: `"✓ Successfully registered {N} deployment(s)\n\n"` — uses `emoji::CHECK_MARK` (✓ U+2713) with `color::SUCCESS` (green+bold), matching Go `color.New(color.FgGreen, color.Bold)`
3. For each deployment (0-indexed `i`), print:
   - `"  Deployment {i+1}:\n"` (2-space indent)
   - `"    Deployment ID: {id}\n"` (4-space indent)
   - `"    Address: {address}\n"` (4-space indent)
   - `"    Contract: {contract_name}\n"` (4-space indent)
   - `"    Label: {label}\n"` (4-space indent, only when label is non-empty)
4. Print blank line between deployments (not after the last one) — matching Go `if i < len-1 { fmt.Println() }`
5. Remove the `output::truncate_address()` summary line

**Acceptance criteria:**
- `cargo clippy --workspace --all-targets` passes with no new warnings
- `cargo test -p treb-cli -- register` compiles (some tests may fail until P11-US-004/US-006)
- Human output for single deployment: green bold `✓ Successfully registered 1 deployment(s)` + indented details
- Human output for multiple deployments: details separated by blank lines
- Label line omitted when empty
- JSON output (`--json`) is completely unchanged
- `output::print_stage()` calls for tracing/registering progress lines remain unchanged

---

### P11-US-002: Port prune Command Human Output to Go Section/Emoji Format

**Description:** Replace the current prune human output (comfy_table with [ID, Kind, Reason, Chain ID] columns for both dry-run and destructive modes) with the Go format: emoji-staged section output with per-section item listings.

**File to modify:** `crates/treb-cli/src/commands/prune.rs` (lines 355-468, the output sections in `run()`)

**Changes to empty-result output (line 359):**
1. Replace `"Nothing to prune."` with `"✅ All registry entries are valid. Nothing to prune."` — uses `emoji::CHECK` (✅) matching Go `"✅ All registry entries are valid. Nothing to prune."`

**Changes to dry-run output (lines 365-391):**
1. Remove `output::print_stage("\u{1f50d}", "Scanning registry...")` call
2. Remove comfy_table creation and `output::print_table()` call
3. Print `"🔍 Checking registry entries against on-chain state..."` using `emoji::MAGNIFYING_GLASS`
4. Print `"\n🗑️  Found {N} items to prune:\n\n"` using `emoji::WASTEBASKET`
5. Group candidates by type and render per-section:
   - Deployment candidates: `"Deployments ({N}):\n"` header + `"  - {id} at {address} (reason: {reason})\n"` per item + blank line
   - Transaction candidates: `"Transactions ({N}):\n"` header + `"  - {id} [{status}] (reason: {reason})\n"` per item + blank line
   - Safe transaction candidates are not currently tracked as prune candidates in the Rust CLI; render section if present in future
6. Remove the old `"{N} prune candidate(s) found. Re-run without --dry-run to remove."` summary

**Changes to destructive-mode output (lines 394-468):**
1. Replace `output::print_stage("\u{1f50d}", "Scanning registry...")` and `output::print_warning_banner(...)` with the same section listing as dry-run (items 3-5 above)
2. Replace custom confirmation message with Go format: `"⚠️  Are you sure you want to prune these items? This cannot be undone. [y/N]: "` — uses `emoji::WARNING`
3. Replace cancellation message `"Cancelled."` with `"❌ Prune cancelled."` — uses `emoji::CROSS`
4. Replace non-interactive mode message to match Go: `"⚠️  Running in non-interactive mode. Proceeding with prune..."`
5. Replace `output::print_stage("\u{1f4be}", "Creating backup...")` + backup output with Go format: `"\n🔧 Pruning registry entries..."` — uses `emoji::WRENCH`
6. Replace the post-removal comfy_table and summary with: `"✅ Successfully pruned {N} items from the registry.\n"` — uses `emoji::CHECK`
7. Remove the `"Backup created at: ..."` line from human output (Go doesn't show it; backup still created internally)

**Helper to add:** A function to group `PruneCandidate`s by type (deployment vs transaction) and extract display fields (address for deployments, status for transactions). Candidates with `BrokenTransactionRef` or `DestroyedOnChain` kind target deployments; `BrokenDeploymentRef` or `OrphanedPendingEntry` target transactions.

**Acceptance criteria:**
- `cargo clippy --workspace --all-targets` passes
- Empty result: `"✅ All registry entries are valid. Nothing to prune."`
- Dry-run with candidates: `🔍` checking header, `🗑️` found header, per-section listings with `"  - "` prefix
- Destructive mode: same section listing + `⚠️` confirmation + `🔧` pruning + `✅` success
- Cancellation shows `"❌ Prune cancelled."`
- JSON output (`--json`) completely unchanged
- Backup still created internally before destructive removal

---

### P11-US-003: Port reset Command Human Output to Go Aligned-Count Format

**Description:** Replace the current reset human output (warning banner with per-type counts + styled "Reset complete." + "Removed N..." summary + backup path) with the Go format: `Found N items to reset` header with namespace/network/chain, aligned per-type count lines, confirmation prompt, and plain success message.

**File to modify:** `crates/treb-cli/src/commands/reset.rs` (lines 162-258, the output and removal sections in `run()`)

**Changes to empty-result output (line 164):**
1. Replace `"Nothing to reset."` (styled with `color::SUCCESS`) with plain `"Nothing to reset. No registry entries found for the current namespace and network."` — no color styling, matching Go exactly

**Changes to summary header (lines 170-183):**
1. Remove `output::print_stage("\u{1f50d}", "Scanning registry...")` and `output::print_warning_banner(...)` calls
2. Print `"Found {total} items to reset for namespace '{ns}' on network '{net}' (chain {chain_id}):\n\n"` — plain text, no color, matching Go format
   - `ns`: the effective namespace (from `--namespace` or default)
   - `net`: the effective network name (from `--network` or default)
   - `chain_id`: the resolved chain ID
   - Note: The Rust CLI uses `--network` as a chain ID, not a network name. Use the chain ID value for both the `network` and `chain` fields (e.g., `"Found N items to reset for namespace 'default' on network '31337' (chain 31337):"`)

**Changes to per-type count lines:**
1. Print aligned count lines (only when count > 0), matching Go fixed-width label alignment:
   - `"  Deployments:        {N}\n"` (8 spaces after colon)
   - `"  Transactions:       {N}\n"` (7 spaces after colon)
   - `"  Safe Transactions:  {N}\n"` (2 spaces after colon)
   - Governor proposals (Rust extension): `"  Governor Proposals: {N}\n"` (1 space after colon) — added to match alignment pattern
2. Print blank line after counts

**Changes to confirmation prompt (lines 186-193):**
1. Replace current `confirm("Remove {N} total entry(s)?", false)` with Go-matching prompt printed directly: `"Are you sure you want to reset the registry for namespace '{ns}' on network '{net}'? This cannot be undone. [y/N]: "` — matching Go format
2. Replace cancellation text `"Cancelled."` with `"Reset cancelled."` — matching Go exactly
3. Add non-interactive mode message: `"Running in non-interactive mode. Proceeding with reset..."` when `--yes` or non-interactive context

**Changes to backup and removal (lines 196-258):**
1. Remove `output::print_stage("\u{1f4be}", "Creating backup...")` (backup still created internally)
2. Remove `output::print_stage("\u{2705}", "Reset complete")` stage marker
3. Replace styled success messages with plain: `"Successfully reset {total} items from the registry.\n"` — no color, no emoji, matching Go
4. Remove `"Backup created at: ..."` line from human output (Go doesn't show it)

**Acceptance criteria:**
- `cargo clippy --workspace --all-targets` passes
- Empty result: `"Nothing to reset. No registry entries found for the current namespace and network."`
- Items found: `"Found N items to reset for namespace '...' on network '...' (chain ...):"` + aligned per-type counts
- Count lines use fixed-width label alignment matching Go spacing
- Confirmation prompt matches Go wording exactly
- Success message is plain text with no color/emoji
- JSON output (`--json`) completely unchanged
- Backup still created internally before removal

---

### P11-US-004: Update Unit Tests for register, prune, and reset Output Changes

**Description:** Update unit tests in the command modules and the CLI assertion-based integration tests that assert on human output strings to match the new Go-matching format.

**Files to modify:**
- `crates/treb-cli/src/commands/register.rs` (test module) — review for output assertions; current tests focus on RPC URL resolution and utility functions, likely no changes needed
- `crates/treb-cli/src/commands/prune.rs` (test module, lines 473+) — review for output assertions on candidate display format
- `crates/treb-cli/src/commands/reset.rs` (test module, lines 263+) — review for output assertions on reset summary format
- `crates/treb-cli/tests/cli_prune_reset_migrate.rs` — update assertion-based tests that check for old output strings:
  - `prune_dry_run_outputs_candidates_and_does_not_modify_files()` — was checking for comfy_table format; update to check for section listing format
  - `prune_yes_removes_broken_entry_and_creates_backup()` — was checking for removal output; update to check for `"✅ Successfully pruned"` format
  - `prune_dry_run_on_clean_registry_outputs_nothing_to_prune()` — update to check for `"✅ All registry entries are valid. Nothing to prune."`
  - `reset_yes_empties_all_stores_and_creates_backup()` — update to check for `"Successfully reset"` format
  - `reset_network_removes_only_matching_chain()` — update assertions
  - `reset_namespace_removes_only_matching_namespace()` — update assertions

**Changes:**
1. Review all tests in the listed files for `contains()`, `assert!`, or other assertions on stdout/stderr output strings
2. Update expected output strings to match new Go-matching format:
   - Prune empty: `"✅ All registry entries are valid. Nothing to prune."`
   - Prune dry-run: section headings like `"Deployments ("`, item prefix `"  - "`, `"(reason: "`
   - Prune destructive: `"✅ Successfully pruned"`, no `"Removed N entry(s)."` or `"Backup created at:"`
   - Reset: `"Found"`, `"items to reset"`, `"Successfully reset"`, no styled messages
3. Remove assertions that reference removed elements (comfy_table headers, `"Scanning registry..."` stage markers, `"Backup created at:"` lines)
4. Add `NO_COLOR=1` env where tests do plain-text `contains()` assertions on styled output (if not already set)

**Acceptance criteria:**
- `cargo test -p treb-cli --lib -- prune` passes (all prune unit tests)
- `cargo test -p treb-cli --lib -- reset` passes (all reset unit tests)
- `cargo test -p treb-cli --test cli_prune_reset_migrate` passes (all CLI assertion tests)
- `cargo clippy --workspace --all-targets` passes
- No dead code warnings from removed helpers

---

### P11-US-005: Update E2E Workflow Test Assertions for register, prune, and reset

**Description:** Update the E2E workflow tests that assert on human output strings for register, prune, and reset commands.

**Files to modify:**
- `crates/treb-cli/tests/e2e_register_workflow.rs` — update `e2e_register_from_tx_hash()` and `e2e_register_tag_show_roundtrip()` assertions to match new register output format
- `crates/treb-cli/tests/e2e_prune_reset_workflow.rs` — update `e2e_prune_onchain_clean_registry()`, `e2e_prune_detects_selfdestructed()`, `e2e_reset_scoped_by_namespace()`, `e2e_deploy_reset_redeploy()` assertions to match new output formats

**Changes:**
1. Review all e2e tests for assertions on stdout/stderr output strings referencing old format
2. Update register test assertions:
   - Replace table-format checks with `"✓ Successfully registered"` and `"Deployment ID:"` assertions
   - Remove assertions on `"deployment(s) registered from transaction"` summary line
3. Update prune test assertions:
   - Replace table-format checks with section listing checks (`"Deployments ("`, `"  - "`, `"(reason: "`)
   - Update success checks to `"✅ Successfully pruned"` or `"✅ All registry entries are valid"`
4. Update reset test assertions:
   - Replace `"Reset complete."` and `"Removed N deployment(s)"` checks with `"Successfully reset N items from the registry."`
   - Update `"Nothing to reset."` checks to include full Go message
5. Add `NO_COLOR=1` env where tests do plain-text `contains()` on styled output

**Acceptance criteria:**
- `cargo test -p treb-cli --test e2e_register_workflow` passes
- `cargo test -p treb-cli --test e2e_prune_reset_workflow` passes
- `cargo clippy --workspace --all-targets` passes
- All assertions reflect the new Go-matching output format

---

### P11-US-006: Update All Golden Test Expected Files

**Description:** Regenerate golden test expected files for register, prune, and reset commands, then verify no cross-command golden drift.

**Files to update:** Golden expected files in:
- `crates/treb-cli/tests/golden/register_*/` (5 directories)
- `crates/treb-cli/tests/golden/prune_*/` (8 directories)
- `crates/treb-cli/tests/golden/reset_*/` (6 directories)

**Steps:**
1. Run `UPDATE_GOLDEN=1 cargo test -p treb-cli --test integration_register --test integration_prune --test integration_reset` to regenerate golden files
2. Review the diffs to confirm they match expected format changes:
   - Register: table replaced with `✓ Successfully registered` + indented details
   - Prune: table replaced with emoji-staged section listings
   - Prune nothing: `"Nothing to prune."` → `"✅ All registry entries are valid. Nothing to prune."`
   - Prune destructive: new emoji messages, no `"Backup created at:"` in output
   - Reset: warning banner replaced with `"Found N items..."` + aligned counts + plain success
   - Reset empty: `"Nothing to reset."` → `"Nothing to reset. No registry entries found for the current namespace and network."`
   - Error golden files (uninitialized, bad prefix, tx not found) should be unchanged
3. Run full `cargo test -p treb-cli` to check for cross-command golden drift
4. If other golden files broke, run targeted `UPDATE_GOLDEN=1` for affected test binaries and verify diffs are benign
5. Run `cargo test --workspace --all-targets` for full workspace validation

**Acceptance criteria:**
- `UPDATE_GOLDEN=1 cargo test -p treb-cli --test integration_register --test integration_prune --test integration_reset` passes
- All golden file diffs match expected output format changes (no unexpected regressions)
- `cargo test -p treb-cli` passes (full CLI test suite, no cross-command failures)
- `cargo test --workspace --all-targets` passes (full workspace)

## Functional Requirements

- **FR-1:** Register human output prints green bold `"✓ Successfully registered N deployment(s)"` header followed by indented per-deployment details (Deployment N, Deployment ID, Address, Contract, Label).
- **FR-2:** Register per-deployment Label field is omitted when empty.
- **FR-3:** Register deployments are separated by blank lines, with no trailing blank line after the last.
- **FR-4:** Prune empty result prints `"✅ All registry entries are valid. Nothing to prune."`.
- **FR-5:** Prune with candidates prints `"🔍 Checking registry entries against on-chain state..."` header followed by `"🗑️  Found N items to prune:"` and per-section listings.
- **FR-6:** Prune per-section format: `"Deployments (N):"` header + `"  - {id} at {address} (reason: {reason})"` per deployment item; `"Transactions (N):"` header + `"  - {id} [{status}] (reason: {reason})"` per transaction item.
- **FR-7:** Prune destructive mode shows `"⚠️  Are you sure...? [y/N]:"` confirmation, `"🔧 Pruning registry entries..."` execution message, and `"✅ Successfully pruned N items from the registry."` success.
- **FR-8:** Prune cancellation shows `"❌ Prune cancelled."`.
- **FR-9:** Reset empty result prints `"Nothing to reset. No registry entries found for the current namespace and network."` in plain text.
- **FR-10:** Reset with items prints `"Found N items to reset for namespace 'NS' on network 'NET' (chain CHAINID):"` header with aligned per-type count lines using fixed-width label spacing.
- **FR-11:** Reset per-type count alignment: `"  Deployments:        N"`, `"  Transactions:       N"`, `"  Safe Transactions:  N"`, `"  Governor Proposals: N"` (Rust extension).
- **FR-12:** Reset confirmation matches Go: `"Are you sure you want to reset the registry for namespace 'NS' on network 'NET'? This cannot be undone. [y/N]:"`.
- **FR-13:** Reset success message is plain text: `"Successfully reset N items from the registry."` — no color, no emoji.
- **FR-14:** JSON output (`--json`) for all three commands remains completely unchanged.
- **FR-15:** Prune backup is still created internally before destructive removal, but the backup path is no longer displayed in human output.
- **FR-16:** Reset backup is still created internally before removal, but the backup path is no longer displayed in human output.

## Non-Goals

- **No JSON schema changes** — this phase only modifies human-readable output, not `--json` output. JSON schema parity is handled in Phase 14.
- **No register interactive prompting** — the Go CLI has interactive `Contract N of N:` prompts during registration; the Rust CLI operates non-interactively via flags. No interactive prompting will be added.
- **No new command features** — no new flags, no new prune candidate types, no new reset scoping options.
- **No prune/reset behavioral logic changes** — candidate detection, backup creation, and removal logic remain unchanged; only display format changes.
- **No comfy_table replacement for other commands** — only register, prune, and reset tables are affected.
- **No changes to error output** — error messages (bad prefix, tx not found, uninitialized) remain unchanged.

## Technical Considerations

### Dependencies
- **Phase 1 artifacts:** `emoji::CHECK_MARK` (✓ U+2713), `emoji::CHECK` (✅), `emoji::MAGNIFYING_GLASS` (🔍), `emoji::WASTEBASKET` (🗑️), `emoji::WRENCH` (🔧), `emoji::WARNING` (⚠️), `emoji::CROSS` (❌), `color::SUCCESS` (green+bold), `color::MUTED` (dimmed)
- **Existing helpers:** `styled()` pattern in prune.rs and reset.rs, `output::print_stage()`, `output::format_success()`

### Key Patterns from Previous Phases
- `styled(text, style)` for conditional coloring — already present in prune.rs and reset.rs
- `emoji::CHECK_MARK` (✓ U+2713) vs `emoji::CHECK` (✅ U+2705) — register uses CHECK_MARK (matching Go `✓`), prune uses CHECK (matching Go `✅`)
- `color::SUCCESS` (green+bold) for register success header, matching Go `color.New(color.FgGreen, color.Bold)`
- Indented key-value format with aligned labels — proven in fork enter/status (Phase 9) and sync/tag (Phase 10)
- Golden test framework trims trailing blank lines from `println!()` — always finalize with `UPDATE_GOLDEN=1`
- Run full `cargo test -p treb-cli` after golden updates to catch cross-command drift
- Always search for ALL test files covering a command before changing output — there are golden, CLI assertion, and e2e test files

### Go Output Reference

**Register output:**
```
✓ Successfully registered 2 deployment(s)      ← green bold

  Deployment 1:
    Deployment ID: default/31337/Token
    Address: 0x5FbDB2315678afecb367f032d93F642f64180aa3
    Contract: Token
    Label: v1

  Deployment 2:
    Deployment ID: default/31337/Proxy
    Address: 0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512
    Contract: Proxy
```

**Prune output (nothing to prune):**
```
✅ All registry entries are valid. Nothing to prune.
```

**Prune output (items found + destructive):**
```
🔍 Checking registry entries against on-chain state...

🗑️  Found 3 items to prune:

Deployments (2):
  - dep-1 at 0xABCD...1234 (reason: deployment 'dep-1' has no on-chain bytecode)
  - dep-2 at 0xEFGH...5678 (reason: deployment 'dep-2' references missing transaction 'tx-1')

Transactions (1):
  - tx-orphan [Executed] (reason: transaction 'tx-orphan' references missing deployment 'dep-gone')

⚠️  Are you sure you want to prune these items? This cannot be undone. [y/N]: y

🔧 Pruning registry entries...
✅ Successfully pruned 3 items from the registry.
```

**Reset output:**
```
Found 7 items to reset for namespace 'default' on network 'mainnet' (chain 1):

  Deployments:        3
  Transactions:       2
  Safe Transactions:  2

Are you sure you want to reset the registry for namespace 'default' on network 'mainnet'? This cannot be undone. [y/N]: y
Successfully reset 7 items from the registry.
```

### Prune Candidate Grouping

The current Rust prune candidates are flat (`Vec<PruneCandidate>`) with a `kind` field. To render per-section output matching Go, group by target type:
- **Deployment section:** candidates with `BrokenTransactionRef` or `DestroyedOnChain` kind — display as `"  - {id} at {address} (reason: {reason})"`. The `address` field is not currently on `PruneCandidate`; it must be looked up from the registry or added to the struct.
- **Transaction section:** candidates with `BrokenDeploymentRef` or `OrphanedPendingEntry` kind — display as `"  - {id} [{status}] (reason: {reason})"`. The `status` field is not currently on `PruneCandidate`; it may need lookup or struct extension.

Consider adding optional `address` and `status` fields to `PruneCandidate` during candidate collection, or looking them up at render time from the registry.

### Potential Risks
- `PruneCandidate` struct may need an `address` field for Go-matching deployment item display — this is a minor struct extension, not a behavioral change
- The Rust reset command uses `--network` as a chain ID integer, while Go uses a network name. The `Found N items to reset...` message will show the chain ID where Go shows a network name — acceptable difference since Rust doesn't have network name resolution in reset
- Removing `"Backup created at:"` from human output is a deliberate match to Go behavior — users can find backups at `.treb/backups/` by convention
- The prune dry-run mode in Rust (`--dry-run` flag) doesn't exist in Go — Go always collects then confirms. The dry-run output should show the same section listing without confirmation/execution messages
