# PRD: Phase 4 - list and show Command Output Parity

## Introduction

This phase transforms the `treb list` and `treb show` commands from their current flat table/key-value output into the Go CLI's hierarchical, color-styled format. The Go `list` command groups deployments by namespace, then chain, then type category (Proxies, Singletons, Libraries) with tree-style rendering showing proxy-implementation relationships. The Go `show` command displays structured sections with per-verifier verification status and URLs. Phase 3 built the reusable UI framework (TreeNode, color palette, badges, terminal utilities) — this phase wires those components into the actual commands and updates all golden files to match the new output format.

## Goals

1. **Hierarchical list output**: Replace the flat `comfy_table` rendering in `list` with tree-style output grouped by namespace > chain > deployment type category, matching the Go CLI's visual hierarchy.

2. **Proxy-implementation relationships**: Display proxy deployments with their implementation addresses as child nodes in the tree (`\-- <impl_address>`), making deployment architecture visible at a glance.

3. **Styled show output**: Enhance `show` with per-verifier verification detail (URL display for each verifier), colored section headers, and styled proxy upgrade history.

4. **Badge integration**: Wire `verification_badge()` and `fork_badge()` from `ui/badge.rs` into both commands, replacing verbose status text with compact indicators.

5. **Golden file parity**: Update all existing golden files for `list` and `show` commands and add new test cases covering the hierarchical output, fork badges, and verification detail.

## User Stories

### P4-US-001: List Command - Deployment Grouping Logic

**Description:** Add a grouping function that organizes a flat list of deployments into a namespace > chain > deployment type hierarchy. This is pure data transformation with no output — it produces the data structure that the tree renderer will consume.

**Details:**
- Create a `fn group_deployments(deployments: &[&Deployment]) -> BTreeMap<String, BTreeMap<u64, BTreeMap<DeploymentType, Vec<&Deployment>>>>` in `list.rs` (or a dedicated `list_format.rs` module)
- Use `BTreeMap` for deterministic ordering: namespaces alphabetically, chain IDs numerically, deployment types in a fixed order (Proxies, Singletons, Libraries, Unknown)
- The type category labels are plural: "Proxies", "Singletons", "Libraries" (matching Go CLI)
- Each leaf group contains deployments sorted by contract name

**Acceptance Criteria:**
- Given 3 deployments (2 singletons + 1 proxy, all `mainnet/42220`), grouping produces: `mainnet` > `42220` > `{Proxy: [TransparentUpgradeableProxy], Singleton: [FPMM, FPMMFactory]}`
- Empty input returns empty map
- Multiple namespaces sort alphabetically
- `cargo check -p treb-cli` passes
- Unit test covering grouping with mixed namespaces, chains, and types

---

### P4-US-002: List Command - Tree Rendering with Grouping

**Description:** Replace the `comfy_table` output in `list` with `TreeNode`-based hierarchical rendering using the grouping from P4-US-001. Each level of the hierarchy becomes a tree node: namespace > chain > type category > deployment entries.

**Details:**
- Build a `TreeNode` tree from the grouped data structure:
  - Root level: one node per namespace (styled with `color::NAMESPACE`)
  - Second level: one node per chain ID (styled with `color::CHAIN`)
  - Third level: one node per deployment type category ("Proxies", "Singletons", "Libraries")
  - Fourth level: one node per deployment showing `ContractName address` (or `ContractName:label address` when label is non-empty)
- Call `render()` for plain output (when color is disabled or piped)
- Address displayed in truncated `0xABCD...EFGH` format using `output::truncate_address()`
- Keep `--json` output path unchanged
- Keep "No deployments found." for empty results

**Acceptance Criteria:**
- Default `treb list` with 3 seeded deployments produces tree output grouped by namespace > chain > type
- `treb list --json` output is unchanged (JSON array)
- `treb list` with no deployments still shows "No deployments found."
- `cargo check -p treb-cli` passes

---

### P4-US-003: List Command - Badges and Proxy Relationship Display

**Description:** Enhance each deployment node in the list tree with verification badges, fork indicators, and proxy-implementation child nodes.

**Details:**
- Append `verification_badge()` output to each deployment label: `ContractName 0xABCD...EFGH UNVERIFIED`
- Append `fork_badge()` output to deployment labels in fork namespaces: `ContractName 0xABCD...EFGH [fork] UNVERIFIED`
- For proxy deployments with `proxy_info.implementation` set, add a child node showing the implementation: `\-- impl: 0x9595...79dF`
- Use `style_for_deployment_type()` to color the deployment type category label
- Style addresses with `color::ADDRESS`

**Acceptance Criteria:**
- List output shows `UNVERIFIED` or `e[V] s[X] b[-]` badges on each deployment line
- Fork-namespace deployments show `[fork]` badge
- Proxy entry has an implementation child node with the truncated implementation address
- `cargo check -p treb-cli` passes
- Unit test verifying proxy child node is added when `proxy_info` exists

---

### P4-US-004: Show Command - Per-Verifier Detail in Verification Section

**Description:** Enhance the Verification section in `show` to display per-verifier status with URLs, replacing the current single-line status.

**Details:**
- When `verification.verifiers` map is non-empty, display each verifier on its own line:
  ```
  Etherscan:  VERIFIED  https://etherscan.io/address/0x...
   Sourcify:  FAILED
  Blockscout:  -
  ```
- Use `verification_badge()` for the compact summary line and add per-verifier detail lines below
- Display verifier URL when present (from `VerifierStatus.url`)
- Display verifier failure reason when present (from `VerifierStatus.reason`)
- When verifiers map is empty, keep the existing single "Status: UNVERIFIED" line
- Order: etherscan, sourcify, blockscout (matching `VERIFIER_ORDER` in badge.rs)

**Acceptance Criteria:**
- Show with empty verifiers map displays `Status:  UNVERIFIED`
- Show with populated verifiers map displays per-verifier lines with status and URL
- Verifier order is always etherscan, sourcify, blockscout
- `cargo check -p treb-cli` passes

---

### P4-US-005: Show Command - Styled Section Headers and Labels

**Description:** Apply ANSI color styling to the `show` command's section headers and key labels, using the Phase 3 color palette.

**Details:**
- Section headers (`-- Identity --`, `-- On-Chain --`, etc.) styled with `color::STAGE`
- Address value styled with `color::ADDRESS`
- Deployment type value colored with `style_for_deployment_type()`
- Verification status colored: VERIFIED with `color::VERIFIED`, FAILED with `color::FAILED`, UNVERIFIED with `color::UNVERIFIED`
- Fork badge in namespace display when applicable
- Proxy upgrade history entries styled with `color::MUTED` for timestamps
- All styling skipped when `should_use_color()` returns false (NO_COLOR, TERM=dumb)
- Plain output path (render without styles) must be the default when piped/non-TTY — use `color_enabled()` check

**Acceptance Criteria:**
- Show output in TTY includes ANSI codes for section headers and values
- Show output when piped (non-TTY) or with `NO_COLOR` set contains no ANSI codes
- Golden file tests use plain output (no ANSI) since test runner captures stdout
- `cargo check -p treb-cli` passes

---

### P4-US-006: Update Golden Files for List Command

**Description:** Update all existing list golden files and add new test cases to cover the tree-style output format, fork badges, and filter behavior with the new grouping.

**Details:**
- Regenerate golden files: `list_table`, `list_filter_namespace`, `list_filter_contract`, `list_filter_type`, `list_ls_alias`
- Keep unchanged: `list_empty`, `list_filter_namespace_no_match`, `list_uninitialized`, `list_json`
- Add new test fixture data: expand `deployments_map.json` OR create a separate richer fixture with:
  - A fork-namespace deployment (to test fork badge)
  - A deployment with populated verifiers (to test verification badge)
  - Multiple namespaces/chains (to test grouping across boundaries)
- Add new golden test: `list_with_fork_badge` — list with fork-namespace deployment showing `[fork]` badge
- Update integration test file `integration_list.rs` to use updated golden files
- Run `UPDATE_GOLDEN=1 cargo test -p treb-cli -- list` to auto-regenerate

**Acceptance Criteria:**
- All list golden file tests pass with `cargo test -p treb-cli -- list`
- New `list_with_fork_badge` test validates fork indicator in tree output
- Existing filter tests produce correct tree-grouped output
- `cargo check -p treb-cli` passes

---

### P4-US-007: Update Golden Files for Show Command

**Description:** Update all existing show golden files and add new test cases covering per-verifier detail, styled proxy info, and fork badge display.

**Details:**
- Regenerate golden files: `show_full_id`, `show_proxy`, `show_non_proxy`, `show_by_contract_name`
- Keep unchanged: `show_json`, `show_nonexistent`, `show_uninitialized`
- Add new golden test: `show_with_verifiers` — show a deployment with populated verifiers map displaying per-verifier status and URLs
- Add new golden test: `show_with_tags` — show a deployment with tags to verify tags section rendering
- To support the new verifier test, either:
  - Add a new deployment to the fixture with verifier data, OR
  - Use a separate fixture/seed function for verifier-specific tests
- Update integration test file `integration_show.rs` to use updated golden files
- Run `UPDATE_GOLDEN=1 cargo test -p treb-cli -- show` to auto-regenerate

**Acceptance Criteria:**
- All show golden file tests pass with `cargo test -p treb-cli -- show`
- New `show_with_verifiers` test validates per-verifier detail lines
- New `show_with_tags` test validates tags section
- Proxy show test includes implementation address display
- `cargo check -p treb-cli` passes

## Functional Requirements

- **FR-1:** `treb list` groups deployments by namespace > chain ID > deployment type category using tree-style rendering with `|--` and `\--` prefixes.
- **FR-2:** Deployment type categories use plural labels: "Proxies", "Singletons", "Libraries".
- **FR-3:** Proxy deployments in the list tree show implementation address as a child node.
- **FR-4:** Each deployment line in the list tree includes: contract name (with label if non-empty), truncated address, verification badge, and fork badge (when applicable).
- **FR-5:** `treb list --json` output is unchanged from pre-Phase-4 behavior.
- **FR-6:** `treb show` Verification section displays per-verifier status with URLs when the verifiers map is populated.
- **FR-7:** `treb show` Proxy Info section displays implementation address and upgrade history.
- **FR-8:** All existing `treb list` and `treb show` filter flags (`--namespace`, `--network`, `--type`, `--tag`, `--contract`, `--label`, `--fork`, `--no-fork`) continue to work with the new output format.
- **FR-9:** Color styling in both commands respects `NO_COLOR` and `TERM=dumb` via `color_enabled()`.
- **FR-10:** List grouping uses `BTreeMap` for deterministic sort order (namespaces alphabetical, chain IDs ascending, types in fixed order).

## Non-Goals

- **No new CLI flags.** This phase only changes output formatting, not command behavior or flag surface.
- **No JSON output changes.** The `--json` path for both commands remains identical to the current schema.
- **No changes to `filter_deployments()` logic.** The filtering function in `list.rs` is unchanged; only the output rendering after filtering changes.
- **No changes to `resolve_deployment()` logic.** The show command's deployment resolution mechanism is unchanged.
- **No changes to registry types or storage.** Deployment, VerificationInfo, ProxyInfo structs are unchanged.
- **No interactive selector changes.** The `fuzzy_select_deployment_id()` fallback in show remains as-is.

## Technical Considerations

### Dependencies
- **Phase 3 UI framework**: All rendering uses `TreeNode`, `color::*`, `badge::*`, and `terminal::*` from `crates/treb-cli/src/ui/`
- **No new crate dependencies.** Everything builds on existing workspace crates and Phase 3 additions.
- **`comfy_table` may become unused** after list switches to tree rendering — remove it from `treb-cli/Cargo.toml` if no other command uses it, or keep for potential use in other phases.

### Integration Points
- `list.rs` imports `ui::{tree::TreeNode, color, badge}` for tree construction and styling
- `show.rs` imports `ui::{color, badge}` for styled section headers and verification detail
- `output.rs` functions (`print_kv`, `truncate_address`, `print_json`) remain in use — `build_table`/`print_table` may be removed from list's code path
- Test fixture `deployments_map.json` may need new entries for fork/verifier test cases — coordinate with `helpers::seed_registry()` which reads this fixture

### Golden File Regeneration
- Use `UPDATE_GOLDEN=1 cargo test -p treb-cli -- <test_name>` to auto-regenerate golden files after output changes
- Golden files capture plain-text output (no ANSI codes) since test stdout is captured in non-TTY mode
- The `ShortHexNormalizer` handles address normalization in golden files — no changes needed for truncated addresses
- Version normalizer handles `v<VERSION>` placeholders — existing normalizers should work with tree output

### Patterns from Previous Phases
- `TreeNode::new(label).with_style(style).child(child)` builder pattern from Phase 3
- `verification_badge(&d.verification.verifiers)` for compact status badges
- `fork_badge(&d.namespace)` returns `Some("[fork]")` for fork namespaces
- `style_for_deployment_type(d.deployment_type)` for per-type coloring
- `output::truncate_address(&d.address)` for `0xABCD...EFGH` format
- Golden file tests use `IntegrationTest::new("name").setup(&["init"]).post_setup_hook(|ctx| helpers::seed_registry(ctx.path())).test(&["list"])` pattern
