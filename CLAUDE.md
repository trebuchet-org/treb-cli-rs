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

**Test framework** (`crates/treb-cli/tests/framework/`):
- `TrebRunner` — subprocess CLI execution
- `TestContext` — high-level harness with anvil + workdir
- `AnvilNode` — in-process anvil management with port pooling
- `TestWorkdir` — fixture isolation with temp directories
- Golden/snapshot comparison with output normalization (timestamps, ports, addresses)

**Shared E2E helpers** (`crates/treb-cli/tests/e2e/mod.rs`): Reusable helpers for multi-command workflow tests, including `setup_project()`, `run_deployment()`, `run_json()`, and `spawn_anvil_or_skip()`.

**Registry compat test seeding**: `crates/treb-cli/tests/helpers/mod.rs::seed_registry()` writes the legacy bare `deployments_map.json` fixture into `.treb/deployments.json`; use a mutating CLI command such as `tag --add` when you need to verify that a write path upgrades legacy store files to the wrapped `{"_format":"treb-v1","entries":...}` shape.

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
- **Non-interactive**: Detected via: `--non-interactive` flag, `TREB_NON_INTERACTIVE=1`, `CI=true`, stdin not TTY, stdout not TTY
- **Store pattern**: Each registry store has PathBuf + HashMap + load/save with fs2 file lock + CRUD + sorted list
- **Versioned store files**: Registry store JSON can be wrapped as `{"_format":"treb-v1","entries":...}`; use `read_versioned_file()` for backward-compatible reads and `write_versioned_file()` for locked atomic writes
- **Deterministic store writes**: Map-backed registry stores should sort into a `BTreeMap` before `write_versioned_file()` so wrapped JSON remains stable for tests and diffs
- **Fork state persistence**: `ForkStateStore` should serialize the `forks` map through a sorted persistence view before `write_versioned_file()`, but keep `history` in insertion order because newest entries are stored first
- **Secondary index ordering**: When a persisted registry index stores `Vec<String>` ID lists inside maps, sort those vectors before saving or returning rebuilt data so lookup file round-trips stay deterministic
- **Registry metadata file**: Rust registry code should not create or read `.treb/registry.json`; that filename is reserved for Go/Solidity registry data, so tests should assert only on actual store files plus `config.local.json`
- **Config ownership**: Only `treb-config` parses config files; other crates consume resolved config
