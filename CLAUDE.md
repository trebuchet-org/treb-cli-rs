# CLAUDE.md

Guidance for Claude Code when working in this repository.

## Project

treb — deployment orchestration CLI for Foundry projects. Rust workspace with in-process forge integration (no subprocess calls to `forge`).

## Workspace Crates

| Crate | Purpose |
|-------|---------|
| `treb-cli` | CLI binary — clap parser, commands, output formatting, UI components |
| `treb-core` | Shared domain types (`TrebError`, `Deployment`, `Fork*`, primitives) |
| `treb-config` | Config parsing and resolution (treb.toml v1/v2, foundry.toml, local overrides) |
| `treb-registry` | JSON-backed registry with file locking (deployments, transactions, safe-txs, governor-txs, lookup, fork state) |
| `treb-forge` | In-process Foundry bridge — script execution, compilation, Anvil, senders, pipeline |
| `treb-verify` | Contract verification orchestration (Etherscan, Blockscout, Sourcify) |
| `treb-safe` | Safe Transaction Service client and EIP-712 signing |
| `treb-sol` | Rust bindings for treb Solidity interfaces via `sol!` macro |

## Key File Paths

- `crates/treb-cli/src/main.rs` — CLI entry point and clap command definitions
- `crates/treb-cli/src/commands/` — One module per command (run.rs, list.rs, fork.rs, etc.)
- `crates/treb-cli/src/output.rs` — Shared formatting: `print_json()`, `build_table()`, `print_kv()`, `format_stage()`
- `crates/treb-cli/src/ui/color.rs` — Color palette and NO_COLOR/TERM=dumb handling
- `crates/treb-cli/src/ui/interactive.rs` — Non-interactive detection (CLI flag, env vars, TTY)
- `crates/treb-cli/build.rs` — Shell completions generation, build metadata embedding
- `crates/treb-core/src/error.rs` — `TrebError` enum (Config, Registry, Forge, Safe, Governor, Fork, Cli, Io)
- `crates/treb-core/src/types/` — Domain types with `#[serde(rename_all = "camelCase")]`
- `crates/treb-config/src/resolver.rs` — Config merging: foundry + treb.toml + local + CLI overrides
- `crates/treb-registry/src/store/` — Store pattern: struct + PathBuf + HashMap, load/save with file_lock
- `crates/treb-registry/src/registry.rs` — `Registry` facade wrapping all stores
- `crates/treb-forge/src/script.rs` — In-process forge script execution
- `crates/treb-forge/src/anvil.rs` — `AnvilConfig` builder + `AnvilInstance` with explicit abort-handle cleanup
- `crates/treb-forge/src/pipeline/` — Deployment recording pipeline (hydration, duplicates, orchestrator)

## Testing

**Golden file tests**: 175 snapshots in `crates/treb-cli/tests/golden/`. CLI tests compare normalized output against `.expected` files. Update with `UPDATE_GOLDEN=1`.

**Golden command renames**: When a CLI command spelling changes, update both the invoking test cases in `crates/treb-cli/tests/integration_*.rs` and the matching `tests/golden/*/commands.golden` headers; the golden harness snapshots the argv list verbatim.

**CLI help snapshots**: Help coverage lives in `crates/treb-cli/tests/integration_help.rs`. Root `treb --help` output is custom-built in `build_grouped_help()`, while subcommand `--help` text still comes from clap, so command-tree changes often need both root and subcommand help snapshots refreshed.

**CLI command help coverage**: When a command gains or changes command-specific flags, prefer adding or refreshing a dedicated `help_<command>` golden in `crates/treb-cli/tests/integration_help.rs` instead of relying on root help; clap-generated subcommand help can drift even when `treb --help` stays unchanged.

**Addressbook integration coverage**: For `treb addressbook` behavior tests, use `TestContext::new("project")`, run `treb init`, pass a numeric `--network`, and set `NO_COLOR=1` for human-output assertions. That keeps config resolution realistic while making list/remove output byte-stable.

**Global clap flags**: When hoisting a flag to `Cli` in `crates/treb-cli/src/main.rs`, mirror it in `crates/treb-cli/build.rs` and refresh the affected `tests/golden/help_*` snapshots; clap will surface the global option in root help and subcommand help, not just on the original command.

**CLI alias compatibility coverage**: When a rename keeps backward-compatible spellings or shorthand forms, add or extend `crates/treb-cli/tests/cli_compatibility_aliases.rs` with byte-for-byte stdout comparisons across the canonical and legacy invocations. For registry-backed commands, seed a temp config project and compare raw stdout/stderr there instead of relying only on parser tests. Keep the feature-specific suites for richer behavior, but pin alias parity in one focused binary-level test file.

**Fork alias parity fixtures**: For fork compatibility checks in `cli_compatibility_aliases.rs`, run each invocation against its own identically seeded temp project and share a multi-request loopback JSON-RPC server when the command needs live RPC. That keeps stdout/stderr byte-stable without snapshot normalizers or Anvil.

**Test framework** (`crates/treb-cli/tests/framework/`):
- `TrebRunner` — subprocess CLI execution
- `TestContext` — high-level harness with anvil + workdir
- `AnvilNode` — in-process anvil management with port pooling
- `TestWorkdir` — fixture isolation with temp directories
- Golden/snapshot comparison with output normalization (timestamps, ports, addresses)

**Shared E2E helpers** (`crates/treb-cli/tests/e2e/mod.rs`): Reusable helpers for multi-command workflow tests, including `setup_project()`, `run_deployment()`, `run_json()`, and `spawn_anvil_or_skip()`.

**Registry compat test seeding**: `crates/treb-cli/tests/helpers/mod.rs::seed_registry()` writes the legacy bare `deployments_map.json` fixture into `.treb/deployments.json`; use a mutating CLI command such as `tag --add` when you need to verify that a write path preserves Go-compatible bare JSON output instead of reintroducing the legacy wrapper.

**Scoped CLI integration coverage**: For namespace/network/fork filter behavior in `crates/treb-cli/tests/integration_*.rs`, prefer direct `TestContext` subprocess assertions plus local `Registry::insert_deployment()` setup when the shared fixture does not provide the ambiguous deployments you need. Keep goldens for formatting-heavy output, and use focused assertions for scope-specific success/error cases.

**CLI registry artifact goldens**: Golden files that snapshot persisted `.treb/*.json` artifacts in `crates/treb-cli/tests/golden/` must match the current bare-map registry format. If an older snapshot still contains `_format`/`entries`, rewrite it to the bare object before trusting the test result.

**CLI RPC golden tests**: When a golden test needs a resolved RPC endpoint, prefer a tiny loopback JSON-RPC listener plus an `extra_normalizer` that rewrites `http://127.0.0.1:<port>` instead of depending on a fixed port or a full Anvil instance.

**Fork argument-shape integration tests**: For fork parsing behavior that only needs subprocess assertions, prefer direct `TestContext::run(...)` checks in `crates/treb-cli/tests/integration_fork.rs`; use the local `spawn_json_rpc_server()` helper there for `fork enter` cases that need `eth_chainId`, and keep goldens for formatting-heavy output only.

**Golden tests with gated fixtures**: If a golden test returns early when loopback sockets or Anvil are unavailable, `UPDATE_GOLDEN=1` will not rewrite its `commands.golden` header. When only the recorded argv changes, refresh that snapshot from an environment where the fixture runs or patch the header manually so the stored command stays aligned with the test case.

**CLI config dotenv tests**: For `config show` coverage that needs `${VAR}` sender resolution, overwrite the fixture `treb.toml` and `.env` in a `pre_setup_hook` and snapshot the human output; `treb_config::resolve_config()` already loads `.env` / `.env.local`, so the test should not inject process env separately.

**Go registry compat fixtures**: `crates/treb-registry/tests/fixtures/go-compat/` stores bare `map[string]T` JSON for cross-CLI registry tests. Prefer subsets from `/home/sol/projects/mento-deployments-v2/.treb/`; the current local snapshot does not include populated deployment tags or `executedAt` on safe transactions, so keep those edge cases explicit when refreshing the fixture set.

**Go registry store-load tests**: For `treb-registry` compatibility coverage, seed a temp registry directory with the raw go-compat fixture under the real store filename (`deployments.json`, `transactions.json`, `safe-txs.json`) and call `Store::load()`; compare offset timestamps as `DateTime<Utc>` values rather than raw strings.

**Go registry round-trip tests**: When Rust rewrites Go-created registry data, compare recursive JSON key sets and timestamp instants instead of literal RFC3339 strings. The schema should stay identical, but `DateTime<Utc>` serialization normalizes offset timestamps to `Z`.

**Anvil spawning references**:
- `crates/treb-cli/tests/e2e/mod.rs` — `spawn_anvil_or_skip()` for workflow tests that need a transient node and must skip cleanly in restricted environments
- `crates/treb-cli/tests/framework/anvil_node.rs` — `AnvilNode::spawn()` / `spawn_with_config()` for integration tests that manage named nodes through `TestContext`
- `crates/treb-cli/tests/framework/context.rs` — `TestContext::with_anvil()` / `with_anvil_mapped()` for composing workdirs, runners, and Anvil instances

**Async tests**: Use `#[tokio::test(flavor = "multi_thread")]` + `tokio::task::spawn_blocking` when calling blocking CLI from async context.

**E2E tests**: Full workflow tests (deployment, fork, prune/reset, register, registry consistency) in `crates/treb-cli/tests/e2e_*.rs`.

**Running tests**:
```bash
cargo test --workspace --all-targets          # all tests
cargo test -p treb-cli                        # CLI tests only (includes golden)
cargo clippy --workspace --all-targets        # lint
```

## Build & Version Pinning

- **Rust**: edition 2024, rust-version 1.85
- **Foundry**: pinned to git tag v1.5.1 in workspace `[dependencies]`
- **Alloy**: all 1.x crates pinned to v1.1.1 via `[patch.crates-io]` — foundry v1.5.1 needs exactly this; without the pins cargo resolves to 1.7.x which breaks alloy-evm
- **Build metadata**: `build.rs` embeds git commit, build date, foundry version, treb-sol commit, rust version via `cargo:rustc-env`

## Conventions

- **Serde**: All types use `#[serde(rename_all = "camelCase")]` for Go registry compatibility
- **Errors**: `TrebError` variants as `#[error("category error: {0}")] Category(String)`
- **Color**: Respects `NO_COLOR` env and `TERM=dumb`; per-command override via `--no-color`; palette in `ui/color.rs`
- **JSON output**: `--json` flag on read commands; deterministic via recursive key sorting in `print_json()`
- **CLI command tree sync**: When renaming or nesting commands in `crates/treb-cli/src/main.rs`, update both `build_grouped_help()` there and `crates/treb-cli/build.rs`; grouped root help and generated shell completions are maintained separately from the derive parser
- **CLI backward-compat aliases**: For spelling-only command renames, prefer keeping the new canonical subcommand in `crates/treb-cli/src/main.rs` with a hidden `alias`, then mirror that alias in `crates/treb-cli/build.rs` so generated shell completions still accept the legacy spelling
- **CLI short flags**: When adding or changing subcommand short options in `crates/treb-cli/src/main.rs`, mirror the shorthand in `crates/treb-cli/build.rs` and pin it with a focused `parse_cli_from(...)` or subcommand help assertion in `src/main.rs` tests so parser/help regressions are caught without relying on broader golden churn
- **Verify option plumbing**: When adding a new `treb verify` execution option, thread it through both the single-deployment and batch paths in `crates/treb-cli/src/commands/verify.rs`, then cover the forge-arg mapping in `crates/treb-verify/src/lib.rs` unit tests because `build_verify_args()` is the only place that translates CLI/runtime flags into Foundry verification inputs
- **Verify debug diagnostics**: When `treb verify` needs to show the underlying Foundry invocation, format it through `treb_verify::format_verify_command()` so single and batch flows print the same `forge verify-contract` surface instead of drifting through ad-hoc stderr strings
- **CLI dual-form args**: When a clap surface accepts both a positional argument and a long flag for the same concept, keep the derive parser and `crates/treb-cli/build.rs` in sync with matching conflicts/groups, and set `value_name` explicitly on any renamed flag field so help and error output show the user-facing placeholder instead of the internal field id. If `--all` can replace the network entirely, put the positional arg, long flag, and `--all` in the same exclusivity group so the bypass and conflict behavior stay aligned across both definitions.
- **CLI default subcommands**: When one command should behave like an existing subcommand without changing that command's help/completion tree, normalize argv in `crates/treb-cli/src/main.rs` before clap parsing instead of reshaping the clap enum; keep `--help` unnormalized so parent help remains stable. If the parent command also owns flags, insert the synthetic subcommand after those parent-scoped flags but before subcommand-only flags like `--json`, or clap will route the options to the wrong level.
- **Non-interactive**: Detected via: `--non-interactive` flag, `TREB_NON_INTERACTIVE=1|true` (case-insensitive), `CI=true` (case-insensitive), stdin not TTY, stdout not TTY
- **Non-interactive plumbing**: When a command reaches interactive helpers through `ui::selector` or destructive confirmation branches, thread `Cli.non_interactive` down explicitly; calling `is_non_interactive(false)` in those paths only preserves env/TTY fallbacks and drops the global clap flag.
- **Scoped deployment queries**: For command-specific namespace/network/fork filtering, pre-filter `Registry::list_deployments()` with `commands::list::filter_deployments()` and then resolve the user query with `commands::resolve::resolve_deployment_in_scope()` so scoped lookups can disambiguate shared names without changing the global resolution rules used elsewhere.
- **Scoped interactive deployment picks**: When a scoped command allows omitting the deployment query, pass the same filtered deployment slice used for scoped resolution into `ui::selector::fuzzy_select_deployment_id()`; otherwise interactive selection can still surface out-of-scope deployments even when direct queries are filtered correctly.
- **Store pattern**: Each registry store has PathBuf + HashMap + load/save with fs2 file lock + CRUD + sorted list
- **Versioned store files**: Registry store JSON is now written as bare JSON, but `read_versioned_file()` and `read_versioned_file_compat()` must continue to accept legacy wrapped `{"_format":"treb-v1","entries":...}` payloads for backward compatibility
- **Deterministic store writes**: Map-backed registry stores should sort into a `BTreeMap` before `write_versioned_file()` so bare JSON remains stable for tests and diffs
- **Nested map store writes**: Registry stores backed by nested maps must sort both the outer and inner maps into `BTreeMap`s before `write_versioned_file()` so per-scope JSON files stay deterministic
- **Fork state persistence**: `ForkStateStore` should serialize the `forks` map through a sorted persistence view before `write_versioned_file()`, but keep `history` in insertion order because newest entries are stored first
- **Secondary index ordering**: When a persisted registry index stores `Vec<String>` ID lists inside maps, sort those vectors before saving or returning rebuilt data so lookup file round-trips stay deterministic
- **Registry metadata file**: Rust registry code should not create or read `.treb/registry.json`; that filename is reserved for Go/Solidity registry data, so tests should assert only on actual store files plus `config.local.json`
- **Config ownership**: Only `treb-config` parses config files; other crates consume resolved config
- **Config-backed network fallbacks**: For commands that accept both `--network` and `--rpc-url`, resolve the config network only when neither flag is provided, then feed that value through the existing RPC URL resolver so explicit overrides keep their current precedence and URL passthrough behavior
- **Env var expansion reuse**: When `foundry.toml` or `treb.toml` strings need `${VAR}` resolution, reuse `treb_config::expand_env_vars()` instead of duplicating expansion logic so unset vars and mixed literals behave consistently
- **Sender config env expansion**: When custom sender tables are parsed into `SenderConfig` from either `treb.toml` or raw `foundry.toml`, run `trebfile::expand_sender_config_env_vars()` on the parsed struct so every supported sender string field stays in sync
- **Dotenv before direct foundry reads**: Commands that bypass `treb_config::resolve_config()` and call `load_foundry_config()` directly must call `treb_config::load_dotenv(cwd)` first, otherwise `${VAR}` RPC endpoints and sender fields defined only in `.env` stay unresolved
- **Config show sender rendering**: Human `treb config show` output should render senders as sorted inline `role  type  address` rows instead of `comfy_table`; keep `--json` output unchanged when touching that command
