# PRD: Phase 4 - list Command Output

## Introduction

Phase 4 ports the most complex rendering in the CLI: the deployment list tree view. The current Rust implementation uses a generic `TreeNode`-based renderer that produces a hierarchical tree, but the Go CLI uses a fundamentally different approach — a table-based renderer with emoji-prefixed namespace/chain headers, type section headers, and fixed-width deployment table rows rendered via `go-pretty`.

This phase replaces the Rust `TreeNode`-based list rendering with the Go-exact format: `◎ namespace:` headers with yellow background, `⛓ chain:` headers with cyan background, bold white type section headers (PROXIES, IMPLEMENTATIONS, SINGLETONS, LIBRARIES), deployment table rows with type-colored contract names, full addresses, verification badges, timestamps, fork indicators, tags, and implementation rows. It also adds the namespace discovery hint (shown when current namespace is empty but others have deployments), the footer line (`Total deployments: N`), and the Go-schema JSON output (`{"deployments": [...], "otherNamespaces": {...}}`).

**Dependencies:** Phase 2 (tree/table renderer alignment) — specifically `render_table_with_widths()`, `calculate_column_widths()`, and `TableData` from `output.rs`.

## Goals

1. **Exact human-readable output parity**: `treb list` produces string-identical output to the Go CLI for all deployment configurations (single/multi namespace, single/multi chain, all four type categories, proxies with implementations, tags, fork badges).
2. **JSON schema parity**: `treb list --json` outputs `{"deployments": [...], "otherNamespaces": {...}}` matching Go's `listJSONOutput` struct, with correct field names, `omitempty` behavior, and fork flag logic.
3. **Implementation categorization**: Singletons whose addresses are referenced as proxy implementations are separated into an `IMPLEMENTATIONS` section, matching Go behavior.
4. **Namespace discovery hint**: When the current namespace has no deployments but other namespaces do, display the Go-format hint with namespace names and counts.
5. **All 10 golden tests pass** with updated expected output matching Go format.

## User Stories

### P4-US-001: Add Implementation Categorization to Deployment Grouping

**Description:** Update `group_deployments()` to separate singletons that serve as proxy implementations into a distinct `IMPLEMENTATIONS` type group, matching Go's categorization logic.

**Context:** The Go renderer (lines 114-162) builds an `implementationAddresses` map from all proxy `ProxyInfo.Implementation` addresses, then classifies singletons matching that set as "implementations" rather than "singletons". The current Rust `group_deployments()` does not do this — it groups by `deployment_type` only.

**Changes:**
- `crates/treb-cli/src/commands/list.rs` — Update `group_deployments()` to:
  1. Build a `HashSet<String>` of implementation addresses from all proxies' `proxy_info.implementation`
  2. When categorizing `Singleton` deployments, check if address is in the implementation set
  3. If yes, place in a new `Implementation` pseudo-group (displayed between PROXIES and SINGLETONS)
- Introduce a display-order enum or const for the four categories: Proxies (0), Implementations (1), Singletons (2), Libraries (3)
- Update `type_sort_key()` to handle the new category
- Update existing unit tests for `group_deployments()` to verify implementation separation

**Acceptance Criteria:**
- Given deployments where a proxy's `proxy_info.implementation` matches a singleton's address, `group_deployments()` places that singleton in an IMPLEMENTATIONS group
- The display order is: PROXIES → IMPLEMENTATIONS → SINGLETONS → LIBRARIES
- Singletons NOT referenced as implementations remain in SINGLETONS
- Existing unit tests pass; new test covers the implementation-separation case
- `cargo clippy --workspace --all-targets` passes

---

### P4-US-002: Add Namespace Discovery Hint and Network Name Resolution

**Description:** Add the namespace discovery hint (shown when current namespace is empty but other namespaces have deployments) and network name resolution for chain headers.

**Context:** The Go CLI's `renderNamespaceDiscoveryHint()` (lines 70-100) shows:
```
No deployments found in namespace "<name>" [on <network> (<chainID>)]

Other namespaces with deployments:

  <namespace>          <count> <deployments/deployment>

Use --namespace <name> or `treb config set namespace <name>` to switch.
```
The Go CLI also resolves chain IDs to network names via `NetworkNames` map for chain headers. The current Rust `run()` function doesn't have access to other namespace data or network names.

**Changes:**
- `crates/treb-cli/src/commands/list.rs` — Update `run()` to:
  1. When filtered results are empty but registry has deployments in other namespaces, compute `other_namespaces: BTreeMap<String, usize>` with counts
  2. Print the namespace discovery hint matching Go format exactly (sorted namespaces, `%-20s` alignment, singular/plural "deployment"/"deployments")
  3. Resolve chain IDs to network names using the config/registry network mapping (or pass through chain ID if no name available)
- Add a `ListResult` struct to hold: deployments, other_namespaces, current_namespace, current_network, current_chain_id, network_names, fork_deployment_ids
- Update `run()` to populate this struct and use it for both display and JSON output

**Acceptance Criteria:**
- When `--namespace staging` returns empty but "default" and "production" have deployments, output matches Go format exactly
- Network names appear in chain headers as `network_name (chain_id)` when available, or just `chain_id` when not
- Singular "deployment" for count=1, plural "deployments" for count>1
- `cargo clippy --workspace --all-targets` passes

---

### P4-US-003: Port Namespace and Chain Headers to Go Format

**Description:** Replace the `TreeNode`-based namespace/chain rendering with Go-exact formatted headers using emoji, background colors, and tree connectors.

**Context:** The Go renderer outputs:
- Namespace: `   ◎ namespace:   UPPERCASE                       ` with Yellow bg, Black text (nsHeader/nsHeaderBold styles)
- Chain: `├─ ⛓ chain:       network_name (chain_id)           ` with Cyan bg, Black text (chainHeader/chainHeaderBold styles), or `└─` for last chain
- After each chain header: a blank continuation line (`│ ` or `  `)

The exact format uses `%-12s` for the label ("namespace:" / "chain:") and `%-30s` for the value.

**Changes:**
- `crates/treb-cli/src/commands/list.rs` — Replace the `TreeNode`-based display loop with direct formatted output:
  1. Namespace header: `nsHeader("   ◎ %-12s ", "namespace:")` + `nsHeaderBold("%-30s", UPPERCASE_NS)`
  2. Chain header: `treePrefix` + `chainHeader(" ⛓ %-12s ", "chain:")` + `chainHeaderBold("%-30s", "network (chainid)")`
  3. Tree prefix: `├─` for non-last chains, `└─` for last chain
  4. Continuation prefix: `│ ` for non-last chains, `  ` for last chain
  5. Blank continuation line after each chain header
- Use `color::NS_HEADER` / `color::NS_HEADER_BOLD` / `color::CHAIN_HEADER` / `color::CHAIN_HEADER_BOLD` styles (add if not already present)
- Use `emoji::CIRCLE` (◎) and `emoji::CHAIN_EMOJI` (⛓)

**Acceptance Criteria:**
- Namespace header line matches Go output exactly: `   ◎ namespace:   UPPERCASE                       ` with yellow bg
- Chain header line matches Go output: `├─ ⛓ chain:       network (chainid)           ` with cyan bg
- Last chain uses `└─` connector, others use `├─`
- Blank continuation line appears after each chain header
- `cargo clippy --workspace --all-targets` passes

---

### P4-US-004: Port Type Section Headers and Deployment Table Rows

**Description:** Replace the `TreeNode`-based type/deployment rendering with Go-exact type section headers and table-formatted deployment rows using `render_table_with_widths()`.

**Context:** The Go renderer outputs:
- Type section headers: `continuationPrefix` + bold white `PROXIES`/`IMPLEMENTATIONS`/`SINGLETONS`/`LIBRARIES`
- Deployment table rows via `renderTableWithWidths()` with 4 columns: [contract_name, address, verification_badges, timestamp]
- Blank continuation lines between sections
- Global column widths calculated across ALL tables in the entire output

**Changes:**
- `crates/treb-cli/src/commands/list.rs` — Add a two-pass rendering approach:
  1. **First pass**: Build all `TableData` for all namespace/chain/type combinations (for global column width calculation)
  2. **Second pass**: Render namespace headers, chain headers, type section headers, and table rows using `render_table_with_widths()`
- Build deployment table rows with:
  - Column 0: Type-colored contract display name (`ContractName:Label`), + fork indicator `[fork]` in yellow, + first tag `(tag)` in cyan
  - Column 1: Full address in white (not truncated — Go uses full address)
  - Column 2: Verification badges (`e[status] s[status] b[status]`) or pending status (`⏳ queued`/`⏳ simulated`)
  - Column 3: Timestamp in faint style (`YYYY-MM-DD HH:MM:SS`)
- Implementation rows: `└─ impl_display_name` in faint style, with empty columns 1-3
- Type section header: `continuationPrefix` + `sectionHeaderStyle.Sprint("PROXIES")` etc.
- Add blank continuation lines between type sections (matching Go's `sectionsDisplayed > 0` logic)

**Acceptance Criteria:**
- Type section headers are bold white, prefixed with continuation prefix
- Deployment rows show: type-colored name, full address (white), verification badges, timestamp (faint)
- Implementation rows show: `└─ impl_name` in faint, with empty address/verification/timestamp columns
- Fork indicator `[fork]` appears in yellow after contract name for fork deployments
- First tag appears as `(tag_name)` in cyan after contract name (after fork badge if present)
- Column widths are globally consistent across all tables
- `cargo clippy --workspace --all-targets` passes

---

### P4-US-005: Port Footer Line and Empty-State Messages

**Description:** Add the `Total deployments: N` footer line and update empty-state messages to match Go format.

**Context:**
- Go outputs `Total deployments: N` after the last namespace/chain group (line 309)
- Go outputs `No deployments found` (no period) when empty with no other namespaces
- The current Rust outputs `No deployments found.` (with period)

**Changes:**
- `crates/treb-cli/src/commands/list.rs`:
  1. After rendering all namespace groups, print `Total deployments: N` where N is the total deployment count
  2. Change empty message from `"No deployments found."` to `"No deployments found"` (no period)
  3. Ensure the footer does NOT appear when there are 0 deployments (only the empty message or namespace hint)

**Acceptance Criteria:**
- `Total deployments: N` appears as the last line when deployments exist
- Empty registry shows `No deployments found` (no period) — matches Go exactly
- Footer line count matches the total number of deployments across all namespaces/chains
- `cargo clippy --workspace --all-targets` passes

---

### P4-US-006: Port JSON Output to Go Schema

**Description:** Update `--json` output from raw deployment array to Go's `listJSONOutput` schema: `{"deployments": [...], "otherNamespaces": {...}}`.

**Context:** The Go CLI (lines 110-162) outputs:
```json
{
  "deployments": [
    {
      "id": "...",
      "contractName": "...",
      "address": "...",
      "namespace": "...",
      "chainId": 42220,
      "label": "...",        // omitted if empty
      "type": "SINGLETON",
      "fork": true           // omitted if false
    }
  ],
  "otherNamespaces": {       // omitted if deployments found OR no other namespaces
    "default": 1,
    "production": 3
  }
}
```

The current Rust outputs the full deployment objects as a raw JSON array `[{...}, ...]`.

**Changes:**
- `crates/treb-cli/src/commands/list.rs`:
  1. Define `ListJsonEntry` struct with fields: `id`, `contract_name`, `address`, `namespace`, `chain_id`, `label` (skip_serializing_if empty), `type`, `fork` (skip_serializing_if false)
  2. Define `ListJsonOutput` struct with fields: `deployments: Vec<ListJsonEntry>`, `other_namespaces: Option<BTreeMap<String, usize>>` (skip_serializing_if None)
  3. Convert deployments to `ListJsonEntry`, setting `fork: true` only for deployment IDs in fork_deployment_ids set
  4. Include `other_namespaces` only when current namespace has 0 deployments AND other namespaces exist
  5. Output via `print_json()` for deterministic key sorting

**Acceptance Criteria:**
- JSON output wraps deployments in `{"deployments": [...]}` object, not a raw array
- Each entry has exactly the 8 Go fields (id, contractName, address, namespace, chainId, label, type, fork)
- `label` is omitted when empty string
- `fork` is omitted when false
- `otherNamespaces` is included only when deployments array is empty AND other namespaces exist
- `cargo clippy --workspace --all-targets` passes

---

### P4-US-007: Add New Golden Test Variants

**Description:** Add golden tests for scenarios not currently covered: namespace discovery hint, multiple namespaces, tags display, implementation categorization, and the all-categories view.

**Context:** The existing 10 golden tests cover basic scenarios but several Go-specific rendering features need dedicated test coverage. The Go test suite includes: `list_with_all_categories`, `list_with_tags`, `list_namespace_discovery_hint`, `list_with_multiple_namespaces_and_chains`, and `list_with_proxy_relationships` (showing IMPLEMENTATIONS section).

**Changes:**
- `crates/treb-cli/tests/integration_list.rs` — Add new test functions
- `crates/treb-cli/tests/helpers/mod.rs` — May need additional fixture seeding helpers for:
  - Deployments across multiple namespaces (for discovery hint)
  - Library deployments (for all-categories test)
  - Tagged deployments (tags display in output)
- `crates/treb-cli/tests/golden/` — New golden directories with expected output

**New test cases:**
1. `list_namespace_discovery_hint` — Filter by non-existent namespace when others have deployments → shows hint
2. `list_with_tags` — Deployments with tags → first tag shown as `(tag_name)` in cyan
3. `list_json_wrapped` — JSON output → `{"deployments": [...]}` wrapper (replaces/updates `list_json`)

**Acceptance Criteria:**
- Each new test has a golden expected file matching Go format exactly
- Tests pass with `cargo test -p treb-cli --test integration_list`
- All new tests use `IntegrationTest` builder pattern consistently

---

### P4-US-008: Update Existing Golden Files for New Output Format

**Description:** Update all 10 existing list golden test expected files to match the new Go-format output.

**Context:** The current golden files use `TreeNode`-based output (e.g., `fork/42220\n└─ 42220\n   └─ SINGLETON\n      └─ MockToken 0xDeaD...beeF UNVERIFIED [fork]`). After the rendering rewrite, all expected output must change to Go format (emoji headers, table-formatted rows, full addresses, timestamps, footer line).

**Changes:**
- Run `UPDATE_GOLDEN=1 cargo test -p treb-cli --test integration_list` to regenerate all list golden files
- Manually verify each regenerated file matches Go output format:
  - `list_table`: Full table format with namespace/chain headers, type sections, table rows, footer
  - `list_empty`: `No deployments found` (no period)
  - `list_json`: Updated to `{"deployments": [...]}` wrapper
  - `list_ls_alias`: Same as list_table
  - `list_filter_namespace`: Namespace-filtered output in Go format
  - `list_filter_namespace_no_match`: `No deployments found` (no period) or namespace hint
  - `list_filter_contract`: Contract-filtered output in Go format
  - `list_filter_type`: Type-filtered output in Go format
  - `list_with_fork_badge`: Fork badge in table format
  - `list_uninitialized`: Error message (unchanged)
- Verify diff of each golden file shows expected format transition

**Acceptance Criteria:**
- All 10 existing golden tests pass after update
- `list_empty` shows `No deployments found` (no period)
- `list_table` shows namespace headers, chain headers, type sections, table rows, and `Total deployments: N` footer
- `list_json` shows `{"deployments": [...]}` wrapper
- `list_with_fork_badge` shows `[fork]` in table-formatted row
- `cargo test -p treb-cli --test integration_list` passes all tests
- `cargo clippy --workspace --all-targets` passes

## Functional Requirements

- **FR-1**: Namespace headers render as `   ◎ namespace:   UPPERCASE` with Yellow background, Black text
- **FR-2**: Chain headers render as `├─ ⛓ chain:       network (chainid)` with Cyan background, Black text; `└─` for last chain
- **FR-3**: Type section headers render as bold white: `PROXIES`, `IMPLEMENTATIONS`, `SINGLETONS`, `LIBRARIES`
- **FR-4**: Deployment table rows use fixed-width columns: Contract (type-colored), Address (full, white), Verification badges, Timestamp (faint)
- **FR-5**: Contract display name format: `ContractName:Label` (with label) or `ContractName` (without)
- **FR-6**: Fork indicator: ` [fork]` in yellow appended to contract name for fork deployments
- **FR-7**: Tags display: ` (first_tag)` in cyan appended after fork indicator (if any)
- **FR-8**: Implementation rows: `└─ impl_display_name` in faint style, empty columns 1-3
- **FR-9**: Singletons whose addresses match a proxy implementation are categorized as IMPLEMENTATIONS
- **FR-10**: Global column widths calculated across all tables before rendering
- **FR-11**: Footer: `Total deployments: N` after all groups (only when N > 0)
- **FR-12**: Empty state: `No deployments found` (no period) when no deployments and no other namespaces
- **FR-13**: Namespace discovery hint when current namespace empty but others have deployments
- **FR-14**: JSON output uses `{"deployments": [...], "otherNamespaces": {...}}` schema
- **FR-15**: JSON `fork` field true only for IDs in fork_deployment_ids; omitted when false
- **FR-16**: JSON `label` field omitted when empty string
- **FR-17**: JSON `otherNamespaces` included only when deployments empty AND other namespaces exist
- **FR-18**: Verification badges: `e[✔︎] s[-] b[⏳]` format with per-status colors; pending states show `⏳ queued`/`⏳ simulated`
- **FR-19**: Deployments within each type section sorted alphabetically by contract display name; same name sorted by timestamp (newest first)
- **FR-20**: Tree connectors: `├─`/`└─` for chains, `│ `/`  ` for continuation prefixes

## Non-Goals

- **Not porting filter logic changes**: The existing `filter_deployments()` function is adequate; only display logic changes.
- **Not adding new CLI flags**: The Go CLI's `--fork`/`--no-fork` flags already exist in Rust. No new flags are added.
- **Not modifying core domain types**: `Deployment`, `DeploymentType`, and related types in `treb-core` remain unchanged.
- **Not porting interactive features**: The list command has no interactive prompts.
- **Not updating non-list golden files**: Only list-related golden files are updated in this phase.
- **Not removing `TreeNode`**: The `TreeNode` renderer is still used by other commands; it is not removed.
- **Not modifying `build_table()` (comfy_table)**: The old table builder remains for commands that haven't been ported yet.

## Technical Considerations

### Dependencies
- **Phase 2 utilities**: `render_table_with_widths()`, `calculate_column_widths()`, `TableData` from `output.rs` — these are the core table rendering functions matching Go's go-pretty output
- **Phase 1 utilities**: Color palette (`color::NS_BG`, `color::CHAIN_BG`, `color::NS_HEADER`, `color::CHAIN_HEADER`, type colors), emoji constants (`emoji::CIRCLE`, `emoji::CHAIN_EMOJI`), verification badge functions (`badge::verification_badge_styled()`, `badge::fork_badge_styled()`)

### Key Implementation Notes
- **Two-pass rendering**: Must build ALL `TableData` across all namespaces/chains/types BEFORE rendering, so `calculate_column_widths()` can compute global widths. Then render in a second pass.
- **Full addresses**: Go renders full addresses (not truncated) in the table. The current Rust uses `truncate_address()` — this must change to full addresses in column 1.
- **Timestamp column**: Go formats timestamps as `YYYY-MM-DD HH:MM:SS` in faint style. The current Rust tree view doesn't show timestamps.
- **`#[allow(dead_code)]` removal**: The `calculate_column_widths` and `render_table_with_widths` functions in `output.rs` currently have `#[allow(dead_code)]` — this phase makes them live.
- **Color styles needed**: `NS_HEADER` (yellow bg, black fg), `NS_HEADER_BOLD` (yellow bg, black fg, bold), `CHAIN_HEADER` (cyan bg, black fg), `CHAIN_HEADER_BOLD` (cyan bg, black fg, bold), `SECTION_HEADER` (bold, bright white), `IMPL_PREFIX` (faint). Check `color.rs` for existing definitions; add any missing ones.
- **Network name resolution**: The Rust CLI may not currently have a `NetworkNames` map. If not available, fall back to showing just the chain ID (which matches Go behavior when network name is unknown). Consider reading from config or registry.
- **`fork_deployment_ids`**: The Go CLI computes this from fork state. The Rust CLI detects fork deployments via namespace prefix `fork/`. Need to determine if a proper `ForkDeploymentIDs` set needs to be computed or if namespace-based detection suffices.
- **Golden test targeting**: Use `cargo test -p treb-cli --test integration_list` to run only list tests. Use `UPDATE_GOLDEN=1` to regenerate expected files. Always verify the diff carefully.
- **Existing unit tests**: The `list.rs` module has extensive unit tests for `filter_deployments()` and `group_deployments()`. These must continue to pass after changes.
