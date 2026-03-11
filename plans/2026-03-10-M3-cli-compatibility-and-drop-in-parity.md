# Master Plan: CLI Compatibility and Drop-in Parity with Go

Make the Rust `treb-cli` a true drop-in replacement for the Go `treb` by removing the incompatible registry meta/migration system, fixing registry compatibility, aligning command surface (names, flags, aliases, positional args), resolving broken env var handling, and adding the missing `addressbook` command. Based on side-by-side exploratory testing at `~/projects/mento-deployments-v2` with both CLIs installed.

**Discovery document:** `plans/2026-03-10-exploratory-testing-comparison.md`
**Reference codebase:** `../treb-cli` (Go CLI)
**Target codebase:** `treb-cli-rs` (Rust CLI)

**Scope:** This plan covers compatibility gaps where Rust is **behind** Go or **incompatible**. Features where Rust extends beyond Go (e.g., `--json` on more commands, `--dry-run` on prune, `version --json`) are intentional improvements and are **not** in scope for removal.

---

## Phase 1 -- Remove Registry Meta and Migration System

The Rust CLI has a `registry.json` metadata file (`RegistryMeta` with `version`, `createdAt`, `updatedAt`) and a migration runner (`migrations.rs`) that were built for future schema evolution. In practice, this system conflicts with the Go CLI which uses `registry.json` for a completely different purpose (Solidity Registry address map). The migration system adds complexity for a problem that doesn't exist yet.

**What to remove:**
- `RegistryMeta` struct and `MetaStore` from `registry.rs`
- `migrations.rs` module entirely (including `run_migrations`, `MigrationReport`, all migration functions)
- `REGISTRY_FILE` and `REGISTRY_VERSION` constants from `lib.rs`
- The `registry.json` file from `Registry::init()` creation
- The version check from `Registry::open()`
- The `treb migrate registry` CLI subcommand
- All tests related to registry meta and migrations

**What to add instead:**
Following Foundry's pattern (`foundry-compilers` uses a `"_format"` field in each cache file), add a `"_format": "treb-v1"` field to every store JSON file. This is written on save but **not checked on load** — if a future schema change makes deserialization fail, the code simply treats it as an empty/corrupt file (same as Foundry's implicit invalidation). No migration runner, no version comparison code.

Store files to update:
- `deployments.json` — currently a bare `HashMap<String, Deployment>`, wrap in `{"_format": "treb-v1", "entries": {...}}`
- `transactions.json` — same wrapping pattern
- `safe-txs.json` — same wrapping pattern
- `governor-txs.json` — same wrapping pattern
- `fork.json` — same wrapping pattern
- `lookup.json` — same wrapping pattern

**Backward compatibility:** On load, if the file is a bare map (no `_format` wrapper), read it as-is (pre-wrapper format). On save, always write the wrapped format. This ensures seamless upgrade from existing Rust CLI registries.

**Rust files to modify:**
- `crates/treb-registry/src/registry.rs` — remove `MetaStore`, version check from `open()`, `registry.json` creation from `init()`
- `crates/treb-registry/src/migrations.rs` — delete entirely
- `crates/treb-registry/src/types.rs` — remove `RegistryMeta`, add `VersionedStore<T>` wrapper
- `crates/treb-registry/src/lib.rs` — remove `REGISTRY_FILE`, `REGISTRY_VERSION`, `migrations` module
- `crates/treb-registry/src/store/deployments.rs` — use `VersionedStore` wrapper on save, accept both formats on load
- `crates/treb-registry/src/store/transactions.rs` — same
- `crates/treb-registry/src/store/safe_transactions.rs` — same
- `crates/treb-registry/src/store/governor_proposals.rs` — same
- `crates/treb-registry/src/store/fork_state.rs` — same
- `crates/treb-registry/src/io.rs` — potentially add versioned read/write helpers
- `crates/treb-cli/src/commands/migrate.rs` — remove `registry` subcommand

**Deliverables**
- `registry.json` no longer created or read by Rust CLI
- `migrations.rs` deleted, `treb migrate registry` subcommand removed
- All store files write `{"_format": "treb-v1", "entries": {...}}` on save
- All store files accept both bare map and wrapped format on load (backward compat)
- `Registry::open()` works in directories with Go's `registry.json` (simply ignores it)
- `Registry::init()` no longer creates `registry.json`
- All existing tests pass (update or remove migration tests)
- New tests: verify round-trip with bare map format (pre-wrapper), verify `_format` field is written

**User stories:** 7
**Dependencies:** none

**Learnings from implementation:**
- Store migration is mechanical: swap to `read_versioned_file()` on load, `write_versioned_file()` on save, and remove any extra `with_file_lock()` wrapper — `write_versioned_file()` owns the lock internally.
- Versioned I/O helpers live in `crates/treb-registry/src/io.rs` (`VersionedStore<T>`, `read_versioned_file()`, `write_versioned_file()`). New stores (e.g., addressbook in Phase 9) should use these directly.
- Deterministic store output requires two things: (1) sort map keys into `BTreeMap` before writing, and (2) for secondary indexes with `Vec<String>` values, sort those vectors too — hash-map iteration order causes nondeterministic JSON otherwise.
- `ForkStateStore` uses a separate serializable persistence view to sort `forks` while preserving `history` in recency order — the runtime `HashMap` is never mutated for persistence.
- `seed_registry()` in `crates/treb-cli/tests/helpers/mod.rs` intentionally writes legacy bare-map fixtures so that mutating CLI tests verify the automatic upgrade to wrapped `{"_format":"treb-v1","entries":...}` format. Do not "fix" these fixtures.
- Removing a Clap subcommand requires updating both the command enum in `commands/*` and any helper matchers in `main.rs` (e.g., JSON-mode flag detection), or the workspace won't compile.
- The CLI integration test framework has a default `.treb/` artifact list (`framework/cleanup.rs`, `framework/integration_test.rs`) — when store files are added or removed, this list must be updated or unrelated golden tests break.
- Registry-only changes validate fastest with `cargo check -p treb-registry` + `cargo test -p treb-registry` before running wider workspace checks.
- Golden migrate test coverage now belongs only to `migrate config`; command-removal behavior is better tested with direct `assert_cmd` assertions than golden snapshots.
- **Superseded by Phase 2:** The `{"_format":"treb-v1","entries":...}` wrapper was removed from the write path in favor of bare JSON for Go compatibility. `write_versioned_file()` now writes bare maps. The read path still accepts the legacy wrapper.

---

## Phase 2 -- Go Registry Coexistence Verification

With `registry.json` no longer blocking, verify that all store files (`deployments.json`, `transactions.json`, `safe-txs.json`) are actually compatible between Go and Rust CLIs. Test against a real Go-created `.treb/` directory.

**Go source files to reference:**
- `internal/registry/deployments.go` — Go deployments.json format
- `internal/registry/transactions.go` — Go transactions.json format
- `internal/registry/safe.go` — Go safe-txs.json format

**Rust files to modify:**
- `crates/treb-registry/src/store/deployments.rs` — fix any deserialization gaps
- `crates/treb-registry/src/store/transactions.rs` — fix any deserialization gaps
- `crates/treb-registry/src/store/safe_transactions.rs` — fix any deserialization gaps
- `crates/treb-core/src/types/` — fix any field mismatches in domain types

**Deliverables**
- Copy `.treb/` from `~/projects/mento-deployments-v2` as a test fixture
- `list`, `show`, `tag` all work against Go-created registry
- Rust writes remain compatible with Go CLI reads (no field additions that break Go)
- Document any fields Go writes that Rust ignores (use `#[serde(flatten)]` or `deny_unknown_fields` audit)
- Integration test with real Go registry data

**User stories:** 5
**Dependencies:** Phase 1

**Notes from Phase 1:**
- `registry.json` is now fully ignored by Rust code — Go coexistence for that file is already solved. Focus testing on the actual store files (`deployments.json`, `transactions.json`, `safe-txs.json`, etc.).
- Rust store files now write the `{"_format":"treb-v1","entries":...}` wrapper. Go does not write this wrapper. Verify Go CLI can still read Rust-written wrapped files (Go should ignore unknown top-level keys or treat the file as unparseable — test both directions).
- `seed_registry()` writes bare-map fixtures, which is the same format Go produces. Use this for Go-compatibility test baselines.
- The `.treb/` artifact list in the test framework was updated in Phase 1 to exclude `registry.json` — no further changes needed there for Phase 2.

**Learnings from implementation:**
- **Write path changed to bare JSON**: `write_versioned_file()` now writes bare `map[string]T` JSON instead of the `{"_format":"treb-v1","entries":...}` wrapper. This was necessary because Go cannot read the wrapped format. The `read_versioned_file()` and `read_versioned_file_compat()` functions still accept the legacy wrapper for backward compatibility with older Rust-written files.
- Go-compat fixtures live in `crates/treb-registry/tests/fixtures/go-compat/` as bare JSON maps sourced from `~/projects/mento-deployments-v2/.treb/`. Current fixture set: 13 deployments, 10 transactions, 8 safe transactions.
- The live Go fixture data does not include populated deployment tags or `executedAt` on safe transactions — those edge cases are covered via augmented entries and the existing `crates/treb-core/tests/fixtures/deployments_map.json`.
- `chrono::DateTime<Utc>` serialization normalizes offset timestamps (e.g., `+02:00`) to `Z` suffix without changing the instant. Round-trip tests must compare parsed `DateTime<Utc>` values or instant equivalence, never literal RFC3339 strings.
- Recursive JSON key-set comparison is the reliable assertion style for proving Rust writes preserve the Go JSON tag schema, while still allowing value updates.
- Serde model types must NOT use `#[serde(deny_unknown_fields)]` — a regression test in `go_compat_deserialize.rs` scans source files to enforce this, ensuring forward compatibility with any Go fields Rust does not yet model.
- CLI integration tests can seed Go-created registry data using `seed_go_compat_registry()` from `crates/treb-cli/tests/helpers/mod.rs`, which copies fixtures into `.treb/` and rebuilds `lookup.json`.
- When the write format changes, persisted artifact goldens under `crates/treb-cli/tests/golden/` will break — normalize those snapshots alongside the format change, not as separate follow-up work.
- `seed_registry()` now writes the legacy bare `deployments_map.json` fixture; use a mutating CLI command (e.g., `tag --add`) to verify that the write path produces Go-compatible bare JSON output.

---

## Phase 3 -- Environment Variable Resolution and Config Display

The Rust CLI fails to resolve `${VAR}` patterns in foundry.toml RPC endpoints, causing `networks` to show errors for all networks and `config show` to display empty sender addresses. The Go CLI resolves these via `.env` file loading and environment variable expansion.

**Go source files to reference:**
- `internal/config/foundry.go` — foundry.toml parsing with `${VAR}` resolution
- `internal/config/env.go` — `.env` file loading
- `internal/cli/render/config.go` — config show with resolved sender addresses

**Rust files to modify:**
- `crates/treb-config/src/` — env var resolution for `${VAR}` patterns in foundry.toml
- `crates/treb-cli/src/commands/networks.rs` — display resolved chain IDs
- `crates/treb-cli/src/commands/config.rs` — display resolved sender addresses

**Deliverables**
- `${VAR}` env var patterns in foundry.toml resolved from environment and `.env` files
- `networks` command resolves and displays chain IDs (matching Go output)
- `config show` displays resolved sender addresses (not empty)
- `config show` sender format matches Go: inline `role  type  address` (not comfy_table)
- `networks --json` includes resolved `chainId` field when available
- Golden file updates for networks and config show

**User stories:** 6
**Dependencies:** none

**Learnings from implementation:**
- `foundry_config::Config::load_with_root()` preserves raw `${VAR}` text in RPC endpoint strings — treb must call `expand_env_vars()` after loading the config, not before. The expansion happens inside `treb_config::rpc_endpoints()`.
- `extract_treb_senders_from_foundry()` parses raw TOML custom sections into `SenderConfig`, so env var expansion must run _after_ deserialization — reuse `trebfile::expand_sender_config_env_vars()` to stay aligned with `treb.toml` sender handling.
- CLI commands that bypass `treb_config::resolve_config()` and call `load_foundry_config()` directly (e.g., `networks`) must call `treb_config::load_dotenv(project_root)` first, otherwise `.env`-backed RPC and sender values stay unresolved. This is a recurring pattern — any new command that reads `foundry.toml` directly needs the same treatment.
- `config show` sender output uses Go-style plain aligned rows (not `comfy_table`), sorted by role. The formatter is a standalone function with unit tests in `commands/config.rs`.
- A lightweight loopback JSON-RPC server (binds port 0, responds to `eth_chainId`) is enough for `networks` golden tests — no Anvil dependency needed. Combine with an `extra_normalizer` that rewrites `http://127.0.0.1:<port>` to keep golden files deterministic.
- `output::print_json()` sorts object keys deterministically; golden snapshot expectations for `--json` output must match that sorted key order.
- Dotenv-backed integration tests work by overwriting fixture `treb.toml` and `.env` in a `pre_setup_hook`; `treb_config::resolve_config()` already loads `.env` / `.env.local`, so process env injection is unnecessary and should be avoided.
- The `project` fixture can be repurposed for config env-resolution coverage by overwriting `treb.toml` in a `pre_setup_hook` before `treb init`.

---

## Phase 4 -- Command Naming, Aliases, and Structure

Align command names, subcommand structure, and aliases with the Go CLI to ensure scripts and documentation targeting the Go CLI work unmodified with the Rust CLI.

| Current Rust | Go | Change needed |
|-------------|-----|---------------|
| `gen-deploy` | `gen deploy` | Restructure as nested subcommand |
| `completions` | `completion` | Rename (singular) |
| `config` (requires subcommand) | `config` (shows config) | Default to `show` when no subcommand |
| `list` (no alias) | `list` / `ls` | Add `ls` alias |
| No `gen` alias | `gen` / `generate` | Add `generate` alias |

**Rust files to modify:**
- `crates/treb-cli/src/main.rs` — command definitions, aliases, default subcommands
- `crates/treb-cli/src/commands/gen_deploy.rs` — restructure as `gen deploy` with gen parent
- `crates/treb-cli/src/commands/config.rs` — default behavior when no subcommand given

**Deliverables**
- `treb gen deploy <artifact>` works (nested subcommand, matching Go)
- `treb generate deploy <artifact>` works (alias, matching Go)
- `treb gen-deploy <artifact>` still works (backward compat via alias or hidden command)
- `treb completion <shell>` works (renamed from `completions`)
- `treb config` (no subcommand) shows current config (same as `config show`)
- `treb ls` alias for `treb list`
- Golden file updates for help output

**User stories:** 6
**Dependencies:** none

**Notes from Phase 1:**
- When restructuring Clap commands (e.g., `gen-deploy` → `gen deploy`), update both the command enum definition and all match arms in `main.rs` that reference it (JSON-mode detection, non-interactive detection, etc.) — partial updates cause compile failures.
- Golden test coverage for removed/renamed commands should use direct `assert_cmd` assertions rather than golden snapshots (learned from `migrate registry` removal).

**Notes from Phase 3:**
- Phase 3 changed `config show` sender rendering in `commands/config.rs` to inline aligned rows. When adding `config` default-to-`show` behavior, the output format is already Go-aligned — no further rendering changes needed.
- Several `config show` golden snapshots were refreshed in Phase 3 (`config_show_default`, `config_set_show_round_trip`, `config_remove_show_round_trip`, `config_show_resolves_dotenv_sender_address`). Account for these when updating golden files for help text changes.

---

## Phase 5 -- Global Non-Interactive Flag and Short Flags

The Go CLI has `--non-interactive` as a global flag inherited by every command. The Rust CLI only has it on `run` and `compose`. Several commands also lack the `-s`/`-n` short flags that Go provides.

**Go source files to reference:**
- `internal/cli/root.go` — global `--non-interactive` flag definition
- `internal/cli/list.go` — `-s`/`--namespace`, `-n`/`--network` short flags
- `internal/cli/tag.go` — `-s`/`--namespace`, `-n`/`--network` short flags

**Rust files to modify:**
- `crates/treb-cli/src/main.rs` — add `--non-interactive` as global CLI option
- `crates/treb-cli/src/commands/list.rs` — add `-s` for namespace, `-n` for network
- `crates/treb-cli/src/commands/tag.rs` — add `-s` for namespace, `-n` for network
- `crates/treb-cli/src/commands/show.rs` — add namespace, network flags (see Phase 6)
- All command run() functions — read global non-interactive flag

**Deliverables**
- `--non-interactive` accepted on every command (global flag)
- Global non-interactive detection: flag OR `TREB_NON_INTERACTIVE=1` OR `CI=true` OR non-TTY
- `-s` short flag for `--namespace` on `list`, `tag`, and other commands matching Go
- `-n` short flag for `--network` on `list`, `tag`, and other commands matching Go
- Verify no clashes between `-s`/`-n` and existing short flags (e.g., `-s` on verify = `--sourcify`)
- Golden file updates for help output

**User stories:** 5
**Dependencies:** none

**Learnings from implementation:**
- Global clap flags must be defined in both `src/main.rs` (derive parser) and `build.rs` (completion builder); promoting a flag to global propagates it into root help and every subcommand help, causing multiple `help_*` golden snapshots to change.
- Shared UI helpers (`ui::selector::*`) are intentionally generic and do not see clap state — the command must pass `non_interactive` explicitly. Searching for `is_non_interactive(false)` only catches direct detector calls; selector helper signatures must be updated too.
- `TREB_NON_INTERACTIVE` and `CI` have different truth tables: only the former accepts `"1"`, while both are case-insensitive for `"true"`. These env semantics are pinned in unit tests in `crates/treb-cli/src/ui/interactive.rs`.
- `tag` can accept parser-only filter fields (e.g., `-s`/`-n`) without runtime behavior changes — add `..` to the match arm destructuring so new clap fields are silently ignored until wired.
- Targeted clap regression coverage is cheap: `parse_cli_from(...)` in `src/main.rs` tests pins flag/alias parsing, and `find_subcommand_mut(...).write_long_help(...)` asserts exact help rows without snapshot churn.
- For CLI shorthand compatibility on stateful commands, compare canonical and shorthand invocations at the subprocess level after seeding real registry state in `cli_compatibility_aliases.rs`; parser-only tests miss output drift.
- `integration_prune.rs` is the strongest place to prove `TREB_NON_INTERACTIVE=1` works end-to-end because it exercises actual confirmation bypass and validates registry mutation afterward.
- Help-surface changes that are intentionally scoped can be covered with a dedicated `integration_help` golden (e.g., `help_list`) without touching the broader root help baselines.

---

## Phase 6 -- Deployment Query Flags (show, list, tag)

The Go CLI's `show`, `list`, and `tag` commands have filtering/scoping flags that the Rust CLI lacks. These are needed for multi-namespace and multi-network workflows.

**Go source files to reference:**
- `internal/cli/show.go` — `--namespace`, `--network`, `--no-fork` flags
- `internal/cli/list.go` — `-s`/`-n` short flags, `--tag` filter (Rust already has this)
- `internal/cli/tag.go` — `-s`/`--namespace`, `-n`/`--network` flags

**Rust files to modify:**
- `crates/treb-cli/src/commands/show.rs` — add `--namespace`, `--network`, `--no-fork`
- `crates/treb-cli/src/commands/tag.rs` — add `--namespace`, `--network` (if not already present)
- Command execution paths — wire new flags into registry queries

**Deliverables**
- `show --namespace <NS>` scopes lookup to specific namespace
- `show --network <NET>` scopes lookup to specific network/chain
- `show --no-fork` skips fork-added deployments
- `tag --namespace <NS>` and `tag --network <NET>` scope tag operations
- All new flags properly filter registry queries
- Golden file updates

**User stories:** 5
**Dependencies:** Phase 2 (registry must be loadable with Go data)

**Notes from Phase 2:**
- Go-compat test infrastructure is available: use `seed_go_compat_registry()` from `crates/treb-cli/tests/helpers/mod.rs` to seed `.treb/` with Go-created fixture data for testing new query flags against realistic registry content.
- The `cli_go_registry_compat.rs` test file demonstrates the pattern for verifying CLI commands against Go-created data — follow the same approach for new flag tests.

**Notes from Phase 5:**
- `tag` already has parser-only `--namespace`/`-s` and `--network`/`-n` fields added in Phase 5 (with `..` in the match arm). Phase 6 needs to wire these into the runtime handler to actually filter tag operations — no clap definition changes required.
- Short flags for `list` (`-s`/`-n`) are already defined in both `src/main.rs` and `build.rs` from Phase 5. Phase 6 should focus on runtime filtering behavior, not parser surface.
- When adding `--no-fork` to `show`, mirror in `build.rs` for completions — established pattern from Phase 5.
- Non-interactive plumbing for `show`, `tag`, `verify` was already wired in Phase 5 (global `non_interactive` passed through handlers). New flag additions should not regress this wiring.
- For testing new query flags against seeded data, follow the `cli_compatibility_aliases.rs` subprocess pattern: seed real registry state and compare stdout, not just parser acceptance.

**Learnings from implementation:**
- Scoped deployment resolution requires a dedicated `resolve_deployment_in_scope()` function in `commands::resolve` — the existing `resolve_deployment()` resolves against the whole registry and stays ambiguous even when filters should narrow to one result.
- The canonical pattern for scoped commands: (1) build `DeploymentFilters` from clap flags, (2) call `filter_deployments()` from `commands::list` to get the candidate set, (3) resolve the user query with `resolve_deployment_in_scope()` against that filtered set, (4) pass the same filtered set to `fuzzy_select_deployment_id()` for interactive selection.
- Scoped interactive selection must use the filtered deployment slice — if you pass unfiltered deployments to `fuzzy_select_deployment_id()`, the interactive prompt surfaces out-of-scope deployments even though typed queries are filtered correctly.
- The shared seeded registry fixture does not include ambiguous namespace/network variants; scoped resolution tests often need extra deployments inserted directly into a temp registry via `Registry::insert_deployment()`.
- Focused binary test files (e.g., `cli_list_show.rs`, `cli_tag.rs`) are sufficient for scoped runtime filtering coverage without introducing new golden snapshots — reserve goldens for formatting-heavy output.
- `integration_*.rs` suites can mix golden-backed scenarios with direct `TestContext` assertions, which is cleaner for scope-specific success/error checks than snapshotting every filtered branch.
- `NO_COLOR=1` keeps scoped human-output assertions stable in subprocess tests, while `--json` assertions can read raw stdout without goldens.
- `UPDATE_GOLDEN=1 cargo test -p treb-cli --test integration_help help_show` is sufficient to create or refresh a single help snapshot without broader golden churn.

---

## Phase 7 -- Fork Command Positional Arguments

The Go CLI accepts network as a positional argument for fork subcommands (`fork enter <network>`, `fork exit [network]`, etc.). The Rust CLI requires `--network` flag. For drop-in parity, Rust should accept both forms.

Also, Go uses `--url` for external fork connection while Rust uses `--rpc-url`. Both should be accepted.

**Go source files to reference:**
- `internal/cli/fork.go` — all fork subcommand definitions with positional network arg

**Rust files to modify:**
- `crates/treb-cli/src/commands/fork.rs` — update clap definitions for all fork subcommands
- Fork enter: accept positional `<network>` OR `--network <NETWORK>`
- Fork exit/revert/restart/history/diff: accept positional `[network]` OR `--network <NETWORK>`

**Deliverables**
- `fork enter <network>` works (positional, matching Go)
- `fork enter --network <network>` still works (backward compat)
- `fork exit [network]` works (optional positional, matching Go)
- Same for `fork revert`, `fork restart`, `fork history`, `fork diff`
- `fork enter --url <URL>` accepted as alias for `--rpc-url`
- Conflict detection: error if both positional and `--network` provided
- Golden file updates for help output

**User stories:** 5
**Dependencies:** none

**Notes from Phase 5:**
- When adding `--url` as an alias for `--rpc-url`, mirror it in `build.rs` for shell completions — all subcommand flag changes must stay in sync across both files.
- Non-interactive plumbing is already global from Phase 5 — fork subcommands automatically inherit the flag via `Cli.non_interactive`. No per-subcommand wiring needed for `--non-interactive`.

**Learnings from implementation:**
- Fork clap surfaces are defined twice — in the derive parser (`commands/fork.rs`) and in `build.rs` — so positional args, aliases, and ArgGroup wiring must be mirrored in both places or shell completions drift.
- When the clap field name differs from the displayed long flag (e.g., a renamed field), missing `value_name` leaks the internal field id into usage text and conflict errors. Always set `value_name` explicitly on renamed flag fields.
- Fork subcommands that accept both positional and `--network` forms are normalized in `commands::fork::run(...)` before dispatching to handlers, which keeps handler signatures stable while the clap surface evolves.
- If a dual-form subcommand also supports `--all`, the positional arg, `--network` long flag, and `--all` must share one ArgGroup in both the derive parser and `build.rs` so bypass behavior and conflict errors stay aligned.
- Fast parser-shape coverage lives in the `commands::fork::tests` block using `parse_cli_from(...)`; story-level validation does not need the heavier golden suites unless help text changes.
- `crates/treb-cli/tests/integration_fork.rs` is the right place for fork subprocess assertions that care about parser/runtime behavior; it has a local `spawn_json_rpc_server()` fixture sufficient for commands that only need `eth_chainId`.
- Missing/conflicting fork network arguments are rejected by clap before command dispatch — subprocess tests should assert on clap stderr text, not the later resolver helpers.
- Fork parity tests in `cli_compatibility_aliases.rs` cannot reuse one workdir for both invocations because fork commands mutate fork state; clone the setup into separate temp projects and compare raw outputs.
- A single nonblocking loopback JSON-RPC fixture can satisfy `eth_chainId`, `evm_revert`, `evm_snapshot`, `anvil_reset`, and `anvil_setCode` across both parity runs, keeping tests fast and deterministic.
- Golden tests that early-return before exercising their fixture will not rewrite `commands.golden` under `UPDATE_GOLDEN=1`; if only the recorded argv changed, refresh that snapshot from a runnable environment or patch the header manually.

---

## Phase 8 -- Verify Command Flag Parity

The verify command is missing several flags needed for production verification workflows. The Go CLI supports namespace/network scoping, debug mode, and manual contract path specification.

**Go source files to reference:**
- `internal/cli/verify.go` — full flag set including `--namespace`, `--network`, `--debug`, `--contract-path`, `--blockscout-verifier-url`
- Short flags: `-e` (etherscan), `-b` (blockscout), `-s` (sourcify), `-n` (network)

**Rust files to modify:**
- `crates/treb-cli/src/commands/verify.rs` — add missing flags and short flags

**Deliverables**
- `--namespace <NS>` filter for verification scope
- `--network <NET>` / `-n` for network selection
- `--debug` flag for showing forge verify commands
- `--contract-path <PATH>` for manual contract path specification (e.g., `./src/Counter.sol:Counter`)
- `-e` short for `--etherscan`, `-b` for `--blockscout`, `-s` for `--sourcify`
- `--blockscout-verifier-url` accepted as alias for `--verifier-url` when using blockscout
- Golden file updates

**User stories:** 5
**Dependencies:** none

**Notes from Phase 5:**
- When adding short flags (`-e`, `-b`, `-s`, `-n`) to `verify`, mirror them in `build.rs` — established pattern from Phase 5. Use `parse_cli_from(...)` unit tests and `find_subcommand_mut(...).write_long_help(...)` assertions for cheap regression coverage without golden snapshot churn.
- Phase 5 confirmed `-s` on `verify` is already used for `--sourcify`. If Go also uses `-s` for `--sourcify`, no conflict. If Go uses `-s` for `--namespace` on `verify`, that's a clash — check Go's `internal/cli/verify.go` for the actual short flag mapping before implementing.
- Non-interactive plumbing for `verify` was already wired in Phase 5 (global flag passed to handler).

**Notes from Phase 6:**
- When `verify` adds `--namespace`/`--network` scoping, reuse the established scoped resolution pattern: `filter_deployments()` → `resolve_deployment_in_scope()` → pass filtered slice to interactive selection. This is now available in `commands::resolve` and `commands::list`.
- For scoped behavior tests, prefer direct `TestContext` subprocess assertions with `Registry::insert_deployment()` setup over golden snapshots — the Phase 6 pattern in `integration_show.rs` and `integration_tag.rs` demonstrates this cleanly.

**Notes from Phase 7:**
- When adding flag aliases (e.g., `--blockscout-verifier-url` as alias for `--verifier-url`), set `value_name` explicitly on any renamed field so help and error output show the user-facing placeholder instead of the internal field id — this gotcha was discovered during fork alias work.
- Mirror all new flags and aliases in `build.rs` alongside `src/main.rs` — the dual-definition requirement is confirmed across fork, list, tag, and show; verify will be no different.

**Learnings from implementation:**
- For namespace/network-scoped commands, encapsulating filter construction and scoped messaging in a dedicated `*Scope` struct (e.g., `VerifyScope`) keeps single-item resolution, batch candidate selection, and queryless interactive selection all consistent — the pattern from Phase 6's `resolve_deployment_in_scope()` generalizes cleanly when centralized in a command-specific scope helper.
- Verify runtime flags should terminate in `treb_verify::build_verify_args()` unit tests because that function is the single translation layer into Foundry's `VerifyArgs` — testing at the CLI command level alone misses incorrect forge-arg mappings.
- Adding a new verify execution option requires threading it through both single and batch verification loops in `commands/verify.rs`; updating only one path compiles cleanly but produces inconsistent behavior.
- `treb_verify::format_verify_command()` centralizes the `forge verify-contract` debug surface so single and batch flows print the same stderr output. Debug output must sit immediately before `verify_args.run().await` in both loops — adding it in only one misses multi-verifier batch attempts.
- `--blockscout-verifier-url` renders as a clap alias under `--verifier-url` in `verify --help`, not as a separate standalone option line — golden snapshots must match the alias rendering.
- An already-verified seeded deployment is a stable integration fixture for flag-surface tests because it proves parsing/dispatch without requiring explorer network access or Foundry verification side effects.
- Verify/help compatibility work is best locked down with both fast assertion tests (`cli_verify.rs`) and golden-backed subprocess coverage (`integration_help.rs`, `integration_verify.rs`) — clap rendering and runtime argv acceptance regressions are caught separately.
- The package bin target is `treb-cli`, so targeted binary test runs use `cargo test -p treb-cli --bin treb-cli ...`.

---

## Phase 9 -- Addressbook Command

Implement the `addressbook` command that is entirely missing from the Rust CLI. The addressbook manages named addresses scoped to chain ID, stored in `.treb/addressbook.json`. The Go CLI provides `list`, `set`, `remove` subcommands with `ab` alias.

**Go source files to reference:**
- `internal/cli/addressbook.go` — command definitions and handlers
- `internal/cli/render/addressbook.go` — rendering (name-address pairs, scoped by chain)
- `internal/registry/addressbook.go` — store implementation

**Rust files to create/modify:**
- `crates/treb-registry/src/store/addressbook.rs` — new store module
- `crates/treb-registry/src/store/mod.rs` — register addressbook store
- `crates/treb-registry/src/registry.rs` — add addressbook to Registry facade
- `crates/treb-cli/src/commands/addressbook.rs` — new command module
- `crates/treb-cli/src/commands/mod.rs` — register addressbook command
- `crates/treb-cli/src/main.rs` — add `addressbook` command with `ab` alias in Management Commands group

**Deliverables**
- `treb addressbook list` / `treb ab ls` — lists entries for current chain (name-address pairs)
- `treb addressbook set <name> <address>` / `treb ab set` — adds/updates entry
- `treb addressbook remove <name>` / `treb ab remove` — removes entry
- Entries scoped to chain ID (from `--network` flag or config)
- `--json` output on `list`
- `--namespace` and `--network` flags on all subcommands
- Human output matches Go format: `  NAME  ADDRESS` aligned columns
- `addressbook` (no subcommand) defaults to `list` (matching Go)
- Store reads/writes `.treb/addressbook.json` compatible with Go format
- Golden file tests

**User stories:** 7
**Dependencies:** Phase 1 (store patterns), Phase 3 (network resolution)

**Notes from Phase 1:**
- The new addressbook store should follow the established store pattern: `read_versioned_file()` on load, `write_versioned_file()` on save, `BTreeMap` sorting before persistence. See `crates/treb-registry/src/io.rs` for the helpers and any existing store (e.g., `store/deployments.rs`) for the pattern.
- Register the new store module in `store/mod.rs` and expose through `registry.rs` facade, following the same wiring as other stores.

**Notes from Phase 2 (supersedes Phase 1 wrapper advice):**
- `write_versioned_file()` now writes **bare JSON**, not the `{"_format":"treb-v1","entries":...}` wrapper. The addressbook store should write bare `map[string]T` JSON for Go compatibility. The read path still accepts both bare and legacy wrapped format.
- If Go also writes `addressbook.json`, use the same Go-compat fixture pattern: copy a production slice into `crates/treb-registry/tests/fixtures/go-compat/` and add store-load + round-trip tests in `go_compat_deserialize.rs`.
- Do NOT use `#[serde(deny_unknown_fields)]` on addressbook types — the existing regression test in `go_compat_deserialize.rs` enforces this across all registry/core model sources.

**Notes from Phase 3:**
- If the `addressbook` command resolves network/chain from config (for scoping entries), it goes through `treb_config::resolve_config()` which already loads `.env` / `.env.local` — no extra dotenv call needed. But if it reads `foundry.toml` directly (e.g., for chain ID lookup), call `treb_config::load_dotenv(cwd)` first.
- Human output should use inline aligned columns (matching the Go-style `NAME  ADDRESS` format) — follow the same pattern as the `config show` sender renderer in `commands/config.rs`, not `comfy_table`.
- Integration tests covering chain-scoped addressbook entries can reuse the `pre_setup_hook` pattern to overwrite `.env` with a test chain ID, same as the `config_show_resolves_dotenv_sender_address` golden test.

**Notes from Phase 5:**
- When adding `addressbook` command with `ab` alias, define both in `src/main.rs` and mirror in `build.rs` for shell completions — established pattern from Phase 5.
- Non-interactive is now a global flag on `Cli` — addressbook subcommands automatically inherit it. If any subcommand needs destructive confirmation (e.g., `remove`), pass `cli.non_interactive` explicitly to the confirmation helper, not `is_non_interactive(false)`.

**Notes from Phase 6:**
- If addressbook needs scoped deployment lookups (e.g., to resolve addresses from deployments within a namespace), reuse `filter_deployments()` → `resolve_deployment_in_scope()` from `commands::resolve` and `commands::list`.
- For integration tests covering `--namespace`/`--network` scoping, use direct `TestContext` subprocess assertions with `NO_COLOR=1` for stable human output — reserve goldens for the formatting-heavy `list` output.

**Notes from Phase 7:**
- If `addressbook` subcommands accept both positional arguments and `--flag` forms (e.g., `addressbook set <name>` vs `--name`), normalize them in the command dispatcher before reaching the handler — the fork pattern in `commands::fork::run(...)` keeps handler signatures stable while clap evolves.
- For `ab` alias parity tests in `cli_compatibility_aliases.rs`, use separate identically seeded temp projects per invocation if the command mutates state, and reuse a loopback JSON-RPC server when live RPC is needed — this keeps byte-for-byte output comparison clean without snapshot normalizers.

**Notes from Phase 8:**
- If addressbook adds namespace/network scoping, consider the `VerifyScope` pattern: a dedicated scope struct that centralizes filter construction, scoped resolution, and scoped error messages. This avoids duplicating filter/message logic across list, set, and remove subcommands.
- For addressbook help coverage, add both fast assertion tests (like `cli_verify.rs`) and a dedicated `help_addressbook` golden in `integration_help.rs` — the dual-layer approach catches clap rendering and runtime argv regressions separately.

---

## Phase 10 -- Register and Sync Flag Alignment

Minor flag differences on `register` and `sync` that affect production workflows. The Go CLI derives network/namespace from config while the Rust CLI may require explicit flags.

**Go source files to reference:**
- `internal/cli/register.go` — `--contract-name` separate from `--contract`, no `--rpc-url` (uses config)
- `internal/cli/sync.go` — no `--network` flag (uses config), `--clean` removes invalid entries

**Rust files to modify:**
- `crates/treb-cli/src/commands/register.rs` — verify flag behavior matches Go
- `crates/treb-cli/src/commands/sync.rs` — verify network resolution from config

**Deliverables**
- `register` falls back to configured network when `--network` not provided
- `register --contract-name` behavior matches Go (separate from `--contract`)
- `sync` uses configured network when `--network` not provided (matching Go)
- `sync --clean` flag description matches Go ("Remove invalid entries while syncing")
- Verify `register` and `sync` work without explicit `--network` in Go-created projects

**User stories:** 4
**Dependencies:** Phase 2 (registry coexistence), Phase 3 (config resolution)

**Notes from Phase 2:**
- Go-compat test infrastructure is available: `seed_go_compat_registry()` seeds `.treb/` with Go-created fixture data. Use this to verify `register` and `sync` against realistic Go-originated registry content.
- Registry write path now produces bare JSON — `register` and `sync` mutations will output Go-readable files without any wrapper.

**Notes from Phase 3:**
- `register` already had a unit test that was updated in Phase 3 to match `${VAR}` expansion semantics when env vars are unset (empty-string expansion). If `register` derives network from config, ensure `load_dotenv()` runs before `load_foundry_config()` so `.env`-backed RPC endpoints resolve correctly — follow the same pattern applied to `networks` in Phase 3.
- `sync` likely reads foundry.toml for network resolution. If it bypasses `resolve_config()`, it needs the explicit `load_dotenv(cwd)` → `load_foundry_config()` sequence established in Phase 3.

**Notes from Phase 6:**
- If `register` or `sync` need scoped deployment resolution, reuse the established `filter_deployments()` → `resolve_deployment_in_scope()` pattern. The scoped resolution helpers are in `commands::resolve` and handle both typed queries and interactive selection consistently.

---

## Phase 11 -- Error Messages, Version Format, and Help Text

Polish remaining surface-level differences: error message format, version string format, and help text consistency.

**Differences to address:**
- Go: `Error: unknown command "foo" for "treb"` — Rust: clap default `error: unrecognized subcommand 'foo'`
- Go: `treb nightly-41-gc72d1b1` — Rust: `treb 0.1.0` (version string scheme)
- Go: `Use "treb [command] --help"` footer — Rust: clap default footer
- Go: full help on `-h` — Rust: summary on `-h`, full on `--help`

**Rust files to modify:**
- `crates/treb-cli/src/main.rs` — custom error handler for clap errors, version string format
- `crates/treb-cli/src/commands/version.rs` — version string format alignment
- `crates/treb-cli/build.rs` — version string construction

**Deliverables**
- Version format matches Go: `treb <version-tag>` with git-describe style versioning
- Error messages for unknown commands match Go format
- Help footer `Use "treb [command] --help"` consistent across all commands
- `-h` behavior review: document whether summary vs full help is intentional (clap convention vs Go cobra convention)
- Unknown flag errors match Go format where feasible

**User stories:** 4
**Dependencies:** none

**Notes from Phase 5:**
- Phase 5 established that `--non-interactive` appears in root help and every subcommand help as a global flag. Any help text reformatting in Phase 11 must account for this global option row appearing in all subcommand `--help` output.
- `build_grouped_help()` in `main.rs` is the custom root help builder — it is maintained separately from clap's derive-generated subcommand help. Changes to help footer or `-h` behavior must update both paths.
- Phase 5 added focused help goldens (e.g., `help_list`) that lock individual subcommand help without touching root baselines. Phase 11 help text changes will likely require refreshing all `help_*` golden snapshots, including these Phase 5 additions.

**Notes from Phase 6:**
- Phase 6 added a `help_show` golden snapshot in `integration_help.rs` covering `--namespace`, `--network`, and `--no-fork` options. This is another per-command help golden that Phase 11 must refresh if help text formatting changes.

**Notes from Phase 7:**
- Phase 7 added a `help_fork_enter` golden snapshot in `integration_help.rs` covering positional `<NETWORK>`, `--network`, and `--rpc-url`/`--url` alias surface. This is another per-command help golden that Phase 11 must refresh if help text formatting changes.
- When clap field names differ from the displayed long flag, missing `value_name` leaks the internal field id into clap-generated usage and error text. Phase 11 error message polishing should audit existing commands for this issue — it was discovered and fixed during fork work.
- Golden tests that early-return before running their fixture will not rewrite `commands.golden` under `UPDATE_GOLDEN=1`. When Phase 11 changes affect argv formatting in golden headers, some snapshots may need manual patching if the test cannot run in the current environment.

**Notes from Phase 8:**
- Phase 8 added a `help_verify` golden snapshot in `integration_help.rs` covering `-e`, `-b`, `-s`, `--namespace`, `--network`/`-n`, `--contract-path`, `--debug`, and the `--blockscout-verifier-url` alias. This is another per-command help golden that Phase 11 must refresh if help text formatting changes.
- Clap alias flags (e.g., `--blockscout-verifier-url` as alias for `--verifier-url`) render inline under the primary option in `--help` output, not as separate lines. Phase 11 help text formatting should preserve this clap convention rather than trying to split them.

---

## Dependency Graph (ASCII)

```
Phase 1 (remove registry meta) ──> Phase 2 (Go coexistence verify)
                                                 │
                                                 ├──> Phase 6 (show/list/tag flags)
                                                 └──> Phase 10 (register/sync flags)

Phase 3 (env var resolution) ──┬──> Phase 9 (addressbook)
                               └──> Phase 10 (register/sync flags)

Phase 1 (store patterns) ──> Phase 9 (addressbook)

Phase 4 (naming/aliases)         [independent]
Phase 5 (global flags)           [independent]
Phase 7 (fork positional args)   [independent]
Phase 8 (verify flags)           [independent]
Phase 11 (error/version format)  [independent]
```

---

## Summary Table

| Phase | Title | Stories | Depends On |
|------:|-------|--------:|------------|
| 1 | Remove Registry Meta and Migration System | 7 | -- |
| 2 | Go Registry Coexistence Verification | 5 | 1 |
| 3 | Environment Variable Resolution and Config Display | 6 | -- |
| 4 | Command Naming, Aliases, and Structure | 6 | -- |
| 5 | Global Non-Interactive Flag and Short Flags | 5 | -- |
| 6 | Deployment Query Flags (show, list, tag) | 5 | 2 |
| 7 | Fork Command Positional Arguments | 5 | -- |
| 8 | Verify Command Flag Parity | 5 | -- |
| 9 | Addressbook Command | 7 | 1, 3 |
| 10 | Register and Sync Flag Alignment | 4 | 2, 3 |
| 11 | Error Messages, Version Format, and Help Text | 4 | -- |
| **Total** | | **57** | |
