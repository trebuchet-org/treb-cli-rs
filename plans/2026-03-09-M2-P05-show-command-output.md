# PRD: Phase 5 - show Command Output

## Introduction

Phase 5 ports the detailed single-deployment view (`treb show`) from the Go CLI to the Rust CLI, achieving exact 1:1 output parity. The Go version (`render/deployment.go`) uses a cyan bold `Deployment: ID` header, an 80-character `=` divider, plain-text section headers (`Basic Information:`, `Deployment Strategy:`, etc.), and 2-space-indented key-value fields with per-field coloring. The current Rust implementation uses a different structure (`── Title ──` headers, `print_kv()` right-padded layout, different section names and field ordering).

This phase depends only on Phase 1 (color palette, emoji, format helpers) which is complete. No table rendering is needed — the show command uses simple indented key-value output.

## Goals

1. **Exact header and divider parity**: `Deployment: ID [fork]` in cyan bold + 80-char `=` divider, matching Go `render/deployment.go` lines 28-35.
2. **Section-for-section match**: All 8 Go sections (Basic Information, Deployment Strategy, Proxy Information, Artifact Information, Verification Status, Transaction Information, Tags, Timestamps) rendered with identical headers, field order, conditional display, and coloring.
3. **JSON schema parity**: `--json` output wrapped as `{"deployment": {...}, "fork": true}` matching Go `show.go` lines 73-85.
4. **Timestamp format parity**: All human-readable timestamps use `YYYY-MM-DD HH:MM:SS` format (Go's `2006-01-02 15:04:05`), not RFC3339.
5. **All 9 golden tests pass** with updated `.expected` files matching Go output format.

## User Stories

### P5-US-001: Replace Section Structure with Go Header and Divider Format

**Description:** Replace the current `── Title ──` section header style and `print_kv()` layout with the Go format: a `Deployment: ID` header line, 80-char `=` divider, and plain-text `\nSection Name:` headers with 2-space-indented `  Key: Value\n` fields.

**Changes:**
- `crates/treb-cli/src/commands/show.rs`: Remove `print_header()` helper. Replace `print_kv()` calls with direct `println!("  Key: Value")` format. Add `Deployment: {id}` header in cyan bold + 80-char `=` divider. Rename sections: Identity+On-Chain → "Basic Information", Transaction → "Deployment Strategy", Artifact → "Artifact Information", Verification → "Verification Status", Proxy Info → "Proxy Information", Tags → "Tags", Timestamps → "Timestamps".

**Acceptance Criteria:**
- Header line reads `Deployment: {id}` styled with cyan bold, followed by `\n` and 80 `=` characters
- Each section starts with `\nSection Name:\n` (plain text, not styled)
- Fields use `  Key: Value\n` format (2-space indent, no right-padding alignment)
- No more `── Title ──` headers anywhere in show output
- `cargo clippy --workspace --all-targets` passes
- `cargo test -p treb-cli --test integration_show` compiles (golden tests may fail until US-006 updates expected files)

**Reference:** Go `render/deployment.go` lines 28-35 (header+divider), lines 38, 54, 71, 107, 121, 138, 155, 162 (section headers)

---

### P5-US-002: Port Basic Information and Deployment Strategy Sections

**Description:** Port the first two sections to match Go field ordering and coloring exactly. Basic Information shows Contract (yellow, using `ContractDisplayName` format: `Name:Label` or just `Name`), Address, Type, Namespace, Network (chain ID string), and Label (magenta, conditional). Deployment Strategy shows Method, Factory (conditional), Salt (conditional — skip zero-hash `0x0000...0000`), Entropy (conditional), InitCodeHash (conditional).

**Changes:**
- `crates/treb-cli/src/commands/show.rs`: Implement `contract_display_name()` helper returning `"{name}:{label}"` when label is non-empty, else `"{name}"`. Basic Information section: Contract in yellow, Address plain, Type plain, Namespace plain, Network as chain_id string, Label in magenta (only if non-empty). Deployment Strategy section: Method always shown, Factory/Salt/Entropy/InitCodeHash only when non-empty (Salt also skips zero-hash `0x0000000000000000000000000000000000000000000000000000000000000000`).

**Acceptance Criteria:**
- Contract field shows `ContractDisplayName` format (e.g., `FPMM:v3.0.0`) in yellow
- Label line only appears when label is non-empty, colored magenta
- Network field shows chain ID as string (e.g., `42220`), not a labeled network name
- Salt field is hidden when empty OR when equal to 64-char zero hash
- Entropy and InitCodeHash fields are hidden when empty
- Factory field is hidden when empty
- `cargo clippy --workspace --all-targets` passes

**Reference:** Go `render/deployment.go` lines 37-67

---

### P5-US-003: Port Proxy Information and Artifact Information Sections

**Description:** Port the Proxy Information section to match Go: Type, Implementation (yellow bold contract name + address when resolved implementation is available, else just address), Implementation ID (cyan, when resolved), Admin (conditional), Upgrade History (numbered list with `YYYY-MM-DD HH:MM:SS` timestamps). Port Artifact Information with conditional field display (BytecodeHash, ScriptPath, GitCommit hidden when empty).

**Changes:**
- `crates/treb-cli/src/commands/show.rs`: Proxy Information section — implementation display uses yellow bold for resolved contract name if available (requires registry lookup of implementation deployment by address or ID). Implementation ID shown in cyan when resolved. Upgrade History uses `    N. impl_id (upgraded at YYYY-MM-DD HH:MM:SS)` format. Artifact Information — Path and Compiler always shown; BytecodeHash, Script, GitCommit only when non-empty. Reorder Proxy Information before Artifact Information to match Go section order.

**Acceptance Criteria:**
- Proxy Information section appears only for proxy deployments (when `proxy_info` is `Some`)
- Proxy Information section appears before Artifact Information (matching Go order)
- Implementation line shows address; if implementation deployment can be resolved, shows `ContractDisplayName` in yellow bold + `at address`
- Implementation ID line appears in cyan when implementation is resolved
- Admin line only appears when admin is non-empty
- Upgrade History uses numbered list: `    1. impl_id (upgraded at 2025-01-20 08:00:00)`
- Artifact fields BytecodeHash, Script, GitCommit are hidden when empty
- `cargo clippy --workspace --all-targets` passes

**Reference:** Go `render/deployment.go` lines 69-118

---

### P5-US-004: Port Verification Status, Tags, and Timestamps Sections

**Description:** Port Verification Status to match Go: overall status colored green (verified) or red (other), EtherscanURL conditional, VerifiedAt conditional with `YYYY-MM-DD HH:MM:SS` format. Port Tags as bullet list (`  - tag\n` per tag). Port Timestamps with `YYYY-MM-DD HH:MM:SS` format instead of RFC3339.

**Changes:**
- `crates/treb-cli/src/commands/show.rs`: Verification Status section — show overall `verification.status` colored green/red (not per-verifier breakdown for the Go-matching format), EtherscanURL when non-empty (`  Etherscan: URL`), VerifiedAt when present. Tags section — each tag on its own line as `  - tag\n` (not comma-separated). Timestamps — `Created: YYYY-MM-DD HH:MM:SS`, `Updated: YYYY-MM-DD HH:MM:SS` using `format!("{}", dt.format("%Y-%m-%d %H:%M:%S"))`.

**Acceptance Criteria:**
- Verification status uses `color::VERIFIED` (green) for "VERIFIED", `color::NOT_VERIFIED`/`color::FAILED` (red) for others
- EtherscanURL line hidden when `etherscan_url` is empty
- VerifiedAt line hidden when `verified_at` is None; when present, formatted as `YYYY-MM-DD HH:MM:SS`
- Tags section uses bullet list: one `  - tag` line per tag
- Tags section hidden when tags is None or empty
- Timestamps use `YYYY-MM-DD HH:MM:SS` format (not RFC3339)
- Timestamp labels are `Created:` and `Updated:` (not `Created At:` / `Updated At:`)
- `cargo clippy --workspace --all-targets` passes

**Reference:** Go `render/deployment.go` lines 120-165

---

### P5-US-005: Port JSON Output Schema and Fork Detection

**Description:** Port the JSON output to match Go schema: `{"deployment": {...}, "fork": true}` wrapper. The `fork` field is only present when the deployment is fork-added. Add fork detection to the show command (namespace starts with `"fork/"`). Add `[fork]` badge to the header line for fork deployments.

**Changes:**
- `crates/treb-cli/src/commands/show.rs`: Add `is_fork_deployment()` helper that checks `namespace.starts_with("fork/")`. In JSON mode, wrap output as `{"deployment": <deployment>, "fork": true}` (fork field only when true, matching Go's conditional `if result.IsForkDeployment { output["fork"] = true }`). In human mode, append ` [fork]` badge (yellow styled) to header line when fork deployment. Use `serde_json::json!` macro or a small struct for the wrapper.

**Acceptance Criteria:**
- `--json` output wraps deployment in `{"deployment": {...}}` object
- `fork` field present (value `true`) only when namespace starts with `"fork/"`
- `fork` field absent (not `false`) for non-fork deployments
- Human output header shows `Deployment: ID [fork]` with `[fork]` in yellow for fork deployments
- Human output header shows `Deployment: ID` without badge for non-fork deployments
- JSON output uses `print_json()` for deterministic sorted keys
- `cargo clippy --workspace --all-targets` passes

**Reference:** Go `show.go` lines 72-85, `render/deployment.go` lines 30-32

---

### P5-US-006: Update All Golden Test Expected Files

**Description:** Regenerate all 9 show golden test `.expected` files to match the new Go-parity output format. Verify all golden tests pass and no other test suites are broken.

**Changes:**
- Run `UPDATE_GOLDEN=1 cargo test -p treb-cli --test integration_show` to regenerate all show golden files
- Verify regenerated files match expected Go format (spot-check header, dividers, section names, field format)
- Run full test suite to confirm no regressions

**Acceptance Criteria:**
- All 9 golden tests in `integration_show.rs` pass: `show_full_id`, `show_json`, `show_proxy`, `show_non_proxy`, `show_by_contract_name`, `show_with_verifiers`, `show_with_tags`, `show_nonexistent`, `show_uninitialized`
- `show_full_id` golden file contains: `Deployment: mainnet/42220/FPMM:v3.0.0` header, 80-char `=` divider, `Basic Information:` section, 2-space indented fields
- `show_json` golden file contains `"deployment"` wrapper key
- `show_proxy` golden file contains `Proxy Information:` section
- `show_with_tags` golden file contains bullet-list tags (`  - core`, `  - v3-release`)
- `cargo test -p treb-cli` passes (all CLI tests, not just show)
- `cargo test --workspace --all-targets` passes
- `cargo clippy --workspace --all-targets` passes

---

## Functional Requirements

- **FR-1**: Header line: `Deployment: {id}` in cyan bold, followed by newline and 80 `=` characters
- **FR-2**: Fork badge: ` [fork]` in yellow appended to header when `namespace.starts_with("fork/")`
- **FR-3**: Section headers: `\n{Section Name}:\n` as plain text (not styled)
- **FR-4**: Field format: `  {Key}: {Value}\n` with 2-space indent
- **FR-5**: Basic Information fields in order: Contract (yellow), Address, Type, Namespace, Network (chain ID), Label (magenta, conditional)
- **FR-6**: Contract display name: `Name:Label` when label non-empty, else `Name`
- **FR-7**: Deployment Strategy fields in order: Method, Factory (conditional), Salt (conditional, skip zero-hash), Entropy (conditional), InitCodeHash (conditional)
- **FR-8**: Proxy Information section (conditional): Type, Implementation (with resolved name if available), Implementation ID (cyan, conditional), Admin (conditional), Upgrade History (numbered, `YYYY-MM-DD HH:MM:SS`)
- **FR-9**: Artifact Information fields: Path, Compiler (always); BytecodeHash, Script, GitCommit (conditional on non-empty)
- **FR-10**: Verification Status: Status colored green/red, EtherscanURL (conditional), VerifiedAt (conditional, `YYYY-MM-DD HH:MM:SS`)
- **FR-11**: Tags: bullet list `  - tag\n` per tag; section hidden when no tags
- **FR-12**: Timestamps: `Created:` and `Updated:` in `YYYY-MM-DD HH:MM:SS` format
- **FR-13**: JSON output: `{"deployment": {...}, "fork": true}` wrapper; `fork` field only present when true
- **FR-14**: Section order matches Go: Basic Information, Deployment Strategy, Proxy Information, Artifact Information, Verification Status, Transaction Information, Tags, Timestamps

## Non-Goals

- **Transaction Information section with linked transaction data**: The Go renderer shows Hash, Status, Sender, Block, and Safe context from a resolved `Transaction` object (a runtime-linked field via `json:"-"`). The Rust `Deployment` struct does not have this linked field, and adding transaction resolution to the show command is a larger change. This section will be omitted for now (the current Rust show command already omits it) and can be added in a follow-up when transaction lookup is integrated into the show path.
- **Network name resolution**: Go shows chain ID as a string (`fmt.Sprintf("%d", deployment.ChainID)`), not a resolved network name. No network name resolution is needed.
- **Implementation resolution from registry**: Full registry-backed implementation name resolution (looking up the implementation deployment by address to get its `ContractDisplayName`) is a nice-to-have. If not straightforward, showing just the implementation address is acceptable (matches Go behavior when `deployment.Implementation` is nil).
- **Changes to `--help` text or command flags**: Flag descriptions and help text are Phase 14 scope.
- **Changes to the `Deployment` struct or other core types**: All changes are in the CLI display layer only.

## Technical Considerations

### Dependencies
- **Phase 1 (complete)**: Color constants (`color::VERIFIED`, `color::FAILED`, `color::UNVERIFIED`, `color::FORK_BADGE`, `color::YELLOW`, `color::STAGE`), emoji constants, `format_success()`/`format_warning()`/`format_error()` helpers
- **chrono**: Already a dependency; use `.format("%Y-%m-%d %H:%M:%S")` for timestamp formatting

### Key Differences from Current Rust Implementation
| Aspect | Current Rust | Go Target |
|--------|-------------|-----------|
| Header | `── Identity ──` | `Deployment: ID [fork]` + 80 `=` |
| Section headers | `── Title ──` styled | `\nTitle:` plain text |
| Field layout | `print_kv()` right-padded | `  Key: Value\n` 2-space indent |
| Section names | Identity, On-Chain, Transaction, Artifact, Verification | Basic Information, Deployment Strategy, Proxy Information, Artifact Information, Verification Status |
| Contract display | `d.contract_name` | `ContractDisplayName()` (Name:Label) |
| Tags | Comma-separated | Bullet list `  - tag` |
| Timestamps | RFC3339 | `YYYY-MM-DD HH:MM:SS` |
| JSON | Raw `Deployment` | `{"deployment": {...}, "fork": bool}` |
| Salt filtering | Empty string check | Empty string + zero-hash check |

### Golden Test Targeting
- Use `cargo test -p treb-cli --test integration_show` to target only show golden tests
- Use `UPDATE_GOLDEN=1` to regenerate expected files
- Verify with `git diff` that regenerated files match expected Go format
- Error tests (`show_nonexistent`, `show_uninitialized`) should be unaffected by display changes

### Existing Patterns from Previous Phases
- `styled()` helper for conditional color application (already in show.rs)
- `color::is_color_enabled()` returns true in golden tests (checks `NO_COLOR`/`TERM=dumb` only)
- `output::print_json()` for deterministic JSON with sorted keys
- Fork detection pattern: `namespace.starts_with("fork/")` (established in Phase 4)
