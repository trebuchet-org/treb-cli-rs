# PRD: Phase 9 - Addressbook Command

## Introduction

Phase 9 implements the `addressbook` command, which is entirely missing from the Rust CLI. The Go CLI provides `treb addressbook` (alias `ab`) with `list`, `set`, and `remove` subcommands for managing named addresses scoped by chain ID, stored in `.treb/addressbook.json`. This phase adds the complete addressbook feature — a new registry store module, the CLI command surface, and Go-compatible file format — bringing the Rust CLI to full parity on address management.

This phase depends on Phase 1 (store I/O patterns with `read_versioned_file`/`write_versioned_file`) and Phase 3 (network/config resolution for chain ID scoping).

## Goals

1. **Feature parity**: `treb addressbook list/set/remove` and `treb ab ls/set/remove` produce identical behavior and output to the Go CLI.
2. **Go format compatibility**: `.treb/addressbook.json` is readable and writable by both the Go and Rust CLIs without data loss.
3. **Chain-scoped entries**: All addressbook operations scope entries to the current network's chain ID, resolved from `--network` flag or project config.
4. **Consistent CLI surface**: The command follows established patterns — `ab` alias, `ls` alias for `list`, `--json` on `list`, `--namespace`/`--network` flags, default-to-`list` when no subcommand given.

## User Stories

### P9-US-001: Addressbook Store Module

**Description:** Create the `AddressbookStore` in `treb-registry` with a nested map structure (`chain_id_string -> name -> address`), CRUD methods, and wiring into the Registry facade.

**Acceptance Criteria:**
- New file `crates/treb-registry/src/store/addressbook.rs` with `AddressbookStore` struct following the established store pattern (PathBuf + nested HashMap + load/save)
- Internal data type: `HashMap<String, HashMap<String, String>>` — outer key is chain ID as string (e.g., `"42220"`), inner map is name→address
- `load()` uses `read_versioned_file()` to accept both bare JSON and legacy wrapped format
- `save()` converts to nested `BTreeMap<String, BTreeMap<String, String>>` before calling `write_versioned_file()` for deterministic key ordering
- CRUD methods: `list_entries(chain_id: &str) -> Vec<(String, String)>` (sorted by name), `set_entry(chain_id: &str, name: &str, address: &str)`, `remove_entry(chain_id: &str, name: &str) -> Result<()>`, `has_entry(chain_id: &str, name: &str) -> bool`
- `remove_entry` cleans up empty chain maps (removes the outer key when inner map becomes empty)
- `ADDRESSBOOK_FILE` constant (`"addressbook.json"`) added to `crates/treb-registry/src/lib.rs`
- Store module registered in `crates/treb-registry/src/store/mod.rs` with `pub use`
- `AddressbookStore` field added to `Registry` struct in `crates/treb-registry/src/registry.rs` with accessor methods (`load_addressbook`, `addressbook`, `addressbook_mut`, `set_addressbook_entry`, `remove_addressbook_entry`, `list_addressbook_entries`)
- `Registry::init()` does NOT create an empty `addressbook.json` — the file is created on first write (matching Go behavior)
- Unit tests for store load/save round-trip, BTreeMap ordering, and empty-chain cleanup
- `cargo check -p treb-registry` passes
- `cargo test -p treb-registry` passes

**Files to create/modify:**
- `crates/treb-registry/src/store/addressbook.rs` (new)
- `crates/treb-registry/src/store/mod.rs`
- `crates/treb-registry/src/registry.rs`
- `crates/treb-registry/src/lib.rs`

---

### P9-US-002: Addressbook Command Scaffolding with Set Subcommand

**Description:** Create the `addressbook` command module, wire it into the CLI with the `ab` alias, and implement the `set <name> <address>` subcommand including chain ID resolution and address validation.

**Acceptance Criteria:**
- New file `crates/treb-cli/src/commands/addressbook.rs` with `AddressbookSubcommand` enum (`Set`, `Remove`, `List`) and `pub async fn run(...)` dispatcher
- `Commands` enum in `crates/treb-cli/src/main.rs` gains an `Addressbook` variant with:
  - `#[command(alias = "ab")]` for the `ab` shorthand
  - `--namespace` / `-s` and `--network` / `-n` flags (matching Go persistent flags)
  - `#[command(subcommand)]` for subcommands
- `Set` subcommand accepts two positional args: `<name>` and `<address>` via `cobra.ExactArgs(2)` equivalent (clap required positional args)
- Address validation: must match `^0x[0-9a-fA-F]{40}$` — error message: `invalid address "<addr>": must be a 0x-prefixed 40-character hex string`
- Chain ID resolution: resolves `--network` value (or config network) to a chain ID using Alloy's `Chain` enum for well-known names, or parses numeric chain IDs directly. If no network is configured, error: `no network configured; set one with --network or 'treb config set network <name>'`
- On success, prints: `Set <name> = <address> (chain <chainId>)` to stdout
- `set` calls `Registry::set_addressbook_entry()` then `save()`
- Command registered in `crates/treb-cli/src/commands/mod.rs`
- Command dispatched in `main()` match arm, passing `cli.non_interactive` and `cli.no_color`
- `addressbook` and `ab` mirrored in `crates/treb-cli/build.rs` with all subcommands and flags
- `parse_cli_from(...)` unit tests: `["treb", "addressbook", "set", "Foo", "0x..."]` parses correctly, `["treb", "ab", "set", "Foo", "0x..."]` parses identically
- `cargo check --workspace` passes
- `cargo clippy --workspace --all-targets` passes

**Files to create/modify:**
- `crates/treb-cli/src/commands/addressbook.rs` (new)
- `crates/treb-cli/src/commands/mod.rs`
- `crates/treb-cli/src/main.rs`
- `crates/treb-cli/build.rs`

---

### P9-US-003: Addressbook Remove Subcommand

**Description:** Implement the `remove <name>` subcommand that deletes a named address from the addressbook for the current chain.

**Acceptance Criteria:**
- `Remove` subcommand accepts one positional arg: `<name>`
- Resolves chain ID using the same utility as `set` (from `--network` or config)
- Calls `Registry::remove_addressbook_entry(chain_id, name)` — errors if entry not found: `addressbook entry '<name>' not found on chain <chainId>`
- On success, prints: `Removed <name> (chain <chainId>)` to stdout
- Empty chain maps are cleaned up by the store (tested in P9-US-001)
- `parse_cli_from(...)` unit test: `["treb", "ab", "remove", "Foo"]` parses correctly
- `cargo check --workspace` passes

**Files to modify:**
- `crates/treb-cli/src/commands/addressbook.rs`

---

### P9-US-004: Addressbook List Subcommand and Default Behavior

**Description:** Implement the `list` subcommand with human and JSON output, add the `ls` alias, and make bare `addressbook` (no subcommand) default to `list`.

**Acceptance Criteria:**
- `List` subcommand has `#[command(alias = "ls")]` for the `ls` shorthand
- `--json` flag on `List` subcommand
- Human output format: entries sorted alphabetically by name, each line `  %-24s  %s\n` with name in bold yellow and address in default color (matching Go's `abNameStyle`)
- Empty state: prints `No addressbook entries found` (no error, exit 0)
- JSON output: array of `{"name": "...", "address": "..."}` objects, sorted by name, printed via `output::print_json()`
- JSON output struct: `#[derive(Serialize)] #[serde(rename_all = "camelCase")] struct AddressbookEntryJson { name: String, address: String }`
- Default subcommand: bare `treb addressbook` and `treb ab` default to `list` behavior — use argv normalization in `normalize_cli_args()` (same pattern as `config` → `config show`), keeping `--help` unnormalized so parent help remains stable
- `list` alias `ls` mirrored in `build.rs`
- `parse_cli_from(...)` unit tests: `["treb", "addressbook"]` normalizes to list, `["treb", "ab", "ls", "--json"]` parses with json=true
- `cargo check --workspace` passes
- `cargo clippy --workspace --all-targets` passes

**Files to modify:**
- `crates/treb-cli/src/commands/addressbook.rs`
- `crates/treb-cli/src/main.rs` (argv normalization for default subcommand)
- `crates/treb-cli/build.rs` (add `ls` alias)

---

### P9-US-005: Update Golden Tests, Help Snapshots, and Add Integration Tests

**Description:** Add help golden tests for the addressbook command and integration tests covering set, remove, list, JSON output, and error cases.

**Acceptance Criteria:**
- New `help_addressbook` golden snapshot in `crates/treb-cli/tests/integration_help.rs` for `treb addressbook --help`
- Root help golden (`help_root`) refreshed to include `addressbook` in the Management Commands group
- New integration test file `crates/treb-cli/tests/integration_addressbook.rs` with test cases:
  - `addressbook_set_and_list`: set an entry, list it, verify human output format
  - `addressbook_set_and_list_json`: set entries, list with `--json`, verify JSON structure
  - `addressbook_remove`: set an entry, remove it, verify removal message and empty list
  - `addressbook_remove_not_found`: remove nonexistent entry, verify error message
  - `addressbook_invalid_address`: set with invalid address, verify validation error
  - `addressbook_no_network`: run without `--network` in a project with no configured network, verify error
  - `addressbook_default_to_list`: bare `treb addressbook --network <N>` behaves like `treb addressbook list --network <N>`
  - `addressbook_empty_list`: list when no entries exist, verify "No addressbook entries found"
- Integration tests use `TestContext` with the `project` fixture and `--network` flag with a known chain ID (e.g., `--network 31337` or `--network anvil`)
- Tests use `NO_COLOR=1` for stable human output assertions
- `addressbook.json` included in the test framework artifact cleanup list (`crates/treb-cli/tests/framework/cleanup.rs`) — verify `.treb/` directory removal already covers this
- `UPDATE_GOLDEN=1 cargo test -p treb-cli --test integration_help` refreshes help snapshots
- All golden tests pass
- `cargo test -p treb-cli` passes

**Files to create/modify:**
- `crates/treb-cli/tests/integration_addressbook.rs` (new)
- `crates/treb-cli/tests/integration_help.rs`
- `crates/treb-cli/tests/golden/help_addressbook/` (new, auto-generated)
- `crates/treb-cli/tests/golden/help_root/` (refreshed)

---

### P9-US-006: CLI Compatibility Alias Tests

**Description:** Add alias parity tests verifying that `addressbook` vs `ab` and `list` vs `ls` produce byte-identical output.

**Acceptance Criteria:**
- New test cases in `crates/treb-cli/tests/cli_compatibility_aliases.rs`:
  - `addressbook_vs_ab_list`: `["addressbook", "list", "--network", "31337"]` vs `["ab", "list", "--network", "31337"]` produce identical stdout/stderr after seeding entries
  - `addressbook_list_vs_ls`: `["addressbook", "list", "--network", "31337"]` vs `["addressbook", "ls", "--network", "31337"]` produce identical output
  - `addressbook_vs_ab_set`: `["addressbook", "set", ...]` vs `["ab", "set", ...]` produce identical output
  - `addressbook_default_vs_list`: bare `["addressbook", "--network", "31337"]` vs `["addressbook", "list", "--network", "31337"]` produce identical output
- Each invocation uses a separate identically seeded temp project (since `set`/`remove` mutate state)
- Tests follow established `assert_matching_command_output` pattern with seed function
- `cargo test -p treb-cli --test cli_compatibility_aliases` passes

**Files to modify:**
- `crates/treb-cli/tests/cli_compatibility_aliases.rs`

---

### P9-US-007: Go Compatibility Store Tests

**Description:** Add Go-compatible addressbook fixture and store-level tests verifying the Rust store can read Go-written addressbook data and produce Go-readable output.

**Acceptance Criteria:**
- New Go-compat fixture at `crates/treb-registry/tests/fixtures/go-compat/addressbook.json` containing a representative multi-chain addressbook in bare JSON format (manually created to match Go's `json.MarshalIndent` output)
- Store-load test: `AddressbookStore::load()` correctly deserializes the Go-compat fixture
- Round-trip test: load Go fixture → save → reload → verify entries unchanged
- Key-set comparison: verify Rust-written JSON has identical recursive key structure to Go fixture
- Confirm `addressbook.json` is bare JSON (not wrapped in `{"_format":"treb-v1","entries":...}`)
- Addressbook types do NOT use `#[serde(deny_unknown_fields)]` — verified by existing regression scan in `go_compat_deserialize.rs`
- `cargo test -p treb-registry` passes

**Files to create/modify:**
- `crates/treb-registry/tests/fixtures/go-compat/addressbook.json` (new)
- `crates/treb-registry/tests/go_compat_deserialize.rs` (add addressbook test)

## Functional Requirements

- **FR-1:** `treb addressbook set <name> <address>` adds or overwrites a named address entry scoped to the current chain ID. Address must match `^0x[0-9a-fA-F]{40}$`.
- **FR-2:** `treb addressbook remove <name>` removes a named address entry from the current chain. Returns an error if the entry does not exist.
- **FR-3:** `treb addressbook list` displays all entries for the current chain, sorted alphabetically by name, in `  %-24s  %s` aligned column format.
- **FR-4:** `treb addressbook list --json` outputs entries as a JSON array of `{"name": "...", "address": "..."}` objects, sorted by name.
- **FR-5:** `treb ab` is accepted as an alias for `treb addressbook` on all subcommands.
- **FR-6:** `treb addressbook ls` is accepted as an alias for `treb addressbook list`.
- **FR-7:** Bare `treb addressbook` (no subcommand) defaults to `list` behavior.
- **FR-8:** `--network` / `-n` flag specifies the network whose chain ID scopes entries. Falls back to configured network from `config.local.json` or `treb.toml`.
- **FR-9:** `--namespace` / `-s` flag is accepted for consistency but only affects network resolution through config.
- **FR-10:** Store file `.treb/addressbook.json` uses bare JSON format `{"<chainId>": {"<name>": "<address>"}}` compatible with Go CLI reads and writes.
- **FR-11:** When no network is configured and `--network` is not provided, commands error with: `no network configured; set one with --network or 'treb config set network <name>'`.
- **FR-12:** `set` output format: `Set <name> = <address> (chain <chainId>)`.
- **FR-13:** `remove` output format: `Removed <name> (chain <chainId>)`.
- **FR-14:** Empty list output: `No addressbook entries found`.

## Non-Goals

- **No RPC chain ID resolution**: Chain ID is resolved from Alloy's `Chain` enum or parsed as a numeric string from the `--network` value. Live RPC `eth_chainId` calls are not required for addressbook operations.
- **No interactive selection**: Unlike `show` or `tag`, addressbook commands do not offer interactive fuzzy-select prompts — all arguments are positional.
- **No deployment cross-referencing**: The addressbook is a standalone name→address store. It does not resolve or cross-reference against the deployment registry.
- **No bulk import/export**: Only single-entry `set`/`remove` operations. Batch operations are out of scope.
- **No address checksumming**: Addresses are stored as provided. EIP-55 checksum validation or normalization is not performed.

## Technical Considerations

### Dependencies
- **Phase 1 (store patterns)**: `read_versioned_file()`/`write_versioned_file()` from `crates/treb-registry/src/io.rs` are used for load/save. BTreeMap sorting pattern from `DeploymentStore`.
- **Phase 3 (network resolution)**: `treb_config::resolve_config()` provides the configured network name. `load_dotenv()` is called automatically by `resolve_config()` — no explicit call needed unless reading `foundry.toml` directly.

### Chain ID Resolution
The `--network` flag value (or configured network from `ResolvedConfig.network`) is resolved to a chain ID string via:
1. Parse as `u64` directly (e.g., `--network 42220`)
2. Parse via Alloy's `Chain` enum for well-known names (e.g., `--network celo` → `42220`)
3. Error if neither works: the network name is not recognized

This reuses the `resolve_chain_id()` pattern from `crates/treb-cli/src/commands/list.rs`. Consider extracting it to a shared utility in `commands/mod.rs` or a dedicated `commands/chain.rs` if it doesn't already exist as a shared function.

### Default Subcommand Pattern
Bare `treb addressbook` → `treb addressbook list` uses the same argv normalization approach as `config` → `config show` in `normalize_cli_args()`. A new `addressbook_list_insertion_index()` function checks if the subcommand position is `addressbook` or `ab`, and if the next token is not a known subcommand (`set`, `remove`, `list`, `ls`) or a help flag, inserts `"list"`.

### Store File Lifecycle
`addressbook.json` is NOT created by `Registry::init()`. It is created on first `set` operation when `write_versioned_file()` writes the file. `load()` returns an empty map when the file does not exist (matching Go behavior).

### Test Framework
The `.treb/` directory removal in `framework/cleanup.rs` already covers `addressbook.json` cleanup — no changes needed to the artifact list. Integration tests should use `--network <chainId>` with a numeric chain ID (e.g., `31337`) to avoid dependency on Alloy's `Chain` enum coverage for test network names.
