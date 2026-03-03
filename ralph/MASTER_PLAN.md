# treb-cli-rs: Master Reimplementation Plan

Full-parity reimplementation of treb-cli (Go) in Rust, built directly on top of
foundry's Rust crates. **No subprocess/CLI wrapping** — all forge functionality
is accessed through foundry's library APIs.

## Design Decisions

These decisions apply across all phases:

- **Exact registry compatibility**: Read and write the same `.treb/` JSON format
  as the Go version. Users must be able to switch Go → Rust with zero migration.
  Use golden file tests with fixtures extracted from the Go version.
- **Async from the start**: Use `tokio` runtime throughout. `#[tokio::main]`
  entry point.
- **Full feature parity**: Reimplement everything — Governor, Trezor, legacy v1
  config, compose, all of it. No features dropped.
- **Multi-crate workspace**: Fine-grained crates for faster incremental builds:
  `treb-cli`, `treb-core`, `treb-config`, `treb-registry`, `treb-forge`,
  `treb-verify`, `treb-safe`. Crates are added as needed per phase.
- **Config compatibility**: `.treb/config.local.json` (JSON, matching Go).
  `treb.toml` v1 read support from Phase 3 so existing projects work immediately.
- **No CLI wrapping**: All forge functionality accessed through foundry Rust
  crates — `foundry-config`, `foundry-common`, `forge-script`,
  `forge-script-sequence`, `foundry-wallets`, `forge-verify`, `anvil`. No
  `std::process::Command` calls to `forge` anywhere.
- **Foundry version pinning**: Pin to a specific foundry git commit. Match their
  exact alloy/revm versions. Use `path` deps during dev, `git` deps with pinned
  `rev` for releases.
- **Parasitic CI**: GitHub Actions that track foundry releases, attempt automated
  merge/update via AI, and flag breaking changes.

## Foundry Crate Dependencies

| Foundry Crate | What We Use It For | Added In |
|---|---|---|
| `foundry-config` | `Config::load()`, RPC endpoints, etherscan config, project paths | Phase 1 |
| `foundry-common` | `ProjectCompiler`, `ContractsByArtifact`, `ProviderBuilder` | Phase 1 |
| `forge-script-sequence` | `ScriptSequence`, `TransactionWithMetadata`, `BroadcastReader` | Phase 5 |
| `forge-script` | `ScriptArgs` state machine for in-process script execution | Phase 5 |
| `foundry-wallets` | `WalletSigner` (Local/Ledger/Trezor/AWS/GCP) | Phase 6 |
| `foundry-linking` | `Linker` for CREATE2/nonce-based library linking | Phase 7 |
| `forge-verify` | `VerificationProvider`, Etherscan/Sourcify clients | Phase 13 |
| `anvil` | `NodeConfig`, `anvil::spawn()` for programmatic Anvil | Phase 18 |
| `foundry-evm` | `Executor`, `Backend` — only if needed for custom EVM calls | Phase 14 |
| `foundry-compilers` | Artifact types, compiler output (re-exported via foundry-common) | Phase 1 |

---

## Phase 1 -- Repository Scaffold and Workspace Layout

Bootstrap the Rust workspace, CI, foundry crate integration, and project skeleton.
Establish the foundry dependency chain from day one so all subsequent phases build
on real foundry types.

**Deliverables**
- Cargo workspace with initial crates: `treb-cli` (binary), `treb-core` (library)
- Foundry monorepo as git submodule or path dependency; `foundry-config` and
  `foundry-common` wired up and compiling
- `clap` CLI skeleton that prints version/help and dispatches to stub subcommands
- `thiserror` / `anyhow` error strategy established in `treb-core`
- `tokio` runtime wired up (`#[tokio::main]`)
- Alloy primitive re-exports (`Address`, `B256`, `U256`) pinned to foundry's versions
- GitHub Actions CI: `cargo check`, `cargo test`, `cargo clippy`, `cargo fmt --check`
- `.gitignore`, `rust-toolchain.toml` (matching foundry's minimum), `rustfmt.toml`, `clippy.toml`
- Placeholder README with build instructions

**User stories:** 5
**Dependencies:** none

---

## Phase 2 -- Core Domain Types and Serialization

Define the central data models that every other module depends on: deployments,
transactions, contracts, networks, and their serde representations. **All types
must serialize to the exact same JSON format as the Go version.** Use alloy types
from foundry's pinned versions for primitives.

**Deliverables**
- `Deployment` struct (id, address, type/strategy, proxy info, artifact info, verification status, tags)
- `Transaction` struct (hash, status, sender, deployments, operations, safe context)
- `SafeTransaction` struct (safe tx hash, safe address, status, confirmations)
- `Contract` struct (name, path, ABI via `alloy-json-abi::JsonAbi`, bytecode, compiler version)
- `Network` / `ChainId` types using `alloy::primitives::ChainId`
- `DeploymentId`, `Address`, `TxHash` newtypes wrapping alloy primitives
- Serde round-trip tests for every model using **golden JSON fixtures from the Go version**

**User stories:** 6
**Dependencies:** Phase 1

---

## Phase 3 -- Configuration System

Parse and merge every configuration source. Use `foundry-config::Config` directly
for `foundry.toml` — no custom parser. Build only the treb-specific config layers
on top.

**Deliverables**
- `treb-config` workspace crate
- `foundry-config::Config::load()` integration for all `foundry.toml` parsing
  (profiles, `[rpc_endpoints]`, `[etherscan]`, solc settings, remappings, EVM options)
- `treb.toml` v2 parser (project-level and local overrides) — treb-specific
- `treb.toml` v1 parser (legacy `[ns.*]` format — read-only for backwards compat)
- `.env` loading via `dotenvy` with override semantics
- Layered config resolution: foundry-config → treb.toml → env → CLI flag
- Config validation with actionable error messages
- Local config store (`.treb/config.local.json`) read/write — JSON, matching Go
- Sender config parsing from `[profile.*.treb.senders.*]` and `[profile.*.treb.accounts.*]`
- Unit tests with fixture files for each config source

**User stories:** 7
**Dependencies:** Phase 2

---

## Phase 4 -- Registry System (Read/Write)

Implement the JSON-backed deployment registry. This is treb-specific — foundry
has no equivalent. **Must read/write the exact same JSON schema as the Go version.**

**Deliverables**
- `treb-registry` workspace crate
- Registry file layout: `.treb/deployments.json`, `transactions.json`, `safe-txs.json`, `lookup.json`, `registry.json`
- `Registry` facade with CRUD for deployments, transactions, safe transactions
- Lookup index build/rebuild (name → id, address → id, tag → ids)
- File-level locking to prevent concurrent corruption
- Atomic writes (write-to-temp then rename)
- Registry migration detection (version field)
- Comprehensive round-trip and query tests **using Go-generated fixture files**

**User stories:** 6
**Dependencies:** Phase 2

---

## Phase 5 -- In-Process Forge Integration

Access forge's compilation and script execution pipeline entirely through Rust
crate APIs. No subprocess calls. Use `foundry-common::ProjectCompiler` for
compilation and `forge-script::ScriptArgs` state machine for script execution.

**Deliverables**
- `treb-forge` workspace crate
- In-process compilation via `foundry-common::ProjectCompiler` + `Config::project()`
- Guard against `ProjectCompiler`'s `std::process::exit(0)` on empty input
- `ScriptArgs` construction from treb config (not clap parsing — build programmatically)
- Script execution pipeline: `preprocess() → compile() → link() → prepare_execution() → execute()`
- `ScriptResult` extraction: logs, traces, transactions, return values, labeled addresses
- Console.log decoding from `ScriptResult.logs` via `decode_console_logs()`
- `ContractsByArtifact` for artifact lookup by name/bytecode
- Broadcast file parsing via `forge-script-sequence::{ScriptSequence, BroadcastReader}`
  (for reading historical broadcasts, not our primary path)
- Forge version detection via `foundry-config` version info
- Integration tests against a small embedded Foundry project fixture

**User stories:** 7
**Dependencies:** Phase 3

---

## Phase 6 -- Sender System

Implement the sender abstraction using `foundry-wallets` directly. All signer
types (private key, Ledger, Trezor, etc.) come from foundry's wallet crate.

**Deliverables**
- `foundry-wallets::WalletSigner` integration: Local, Ledger, Trezor, AWS KMS, GCP KMS
- Sender resolution: map treb config sender definitions → `WalletSigner` instances
- `MultiWalletOpts` / `RawWalletOpts` integration for CLI flag handling
- Keystore file support and mnemonic support (via foundry-wallets)
- `InMemory` sender (deterministic test accounts)
- Safe sender stub (full Safe integration deferred to Phase 17)
- Governor sender stub (full Governor integration deferred to Phase 17)
- Wire sender selection into `ScriptArgs` construction for in-process execution
- Unit tests for resolution logic

**User stories:** 5
**Dependencies:** Phase 3

---

## Phase 7 -- Event Parsing and ABI Bindings

Parse `ITrebEvents` and `ICreateX` events from execution results to detect
deployments, labels, and proxy relationships. These are treb-specific events —
foundry has no knowledge of them. Use alloy's `sol!` macro for type generation.

**Deliverables**
- ABI definitions for `ITrebEvents` and `ICreateX` via alloy `sol!` macro
- Event log decoder: extract treb events from `ScriptResult.logs`
- Deployment extraction: `ContractDeployed`, `ProxyDeployed`, `Create2Deployed`, etc.
- Label/tag extraction from events
- Proxy relationship linking (implementation → proxy)
- `ScriptResult.get_created_contracts()` integration with `ContractsByArtifact`
- `foundry-linking::Linker` for library detection during linking phase
- Script parameter / natspec annotation parser (parses `@custom:env` comments from source)
- Tests against captured log fixtures from real deployments

**User stories:** 6
**Dependencies:** Phase 5

---

## Phase 8 -- Deployment Recording Pipeline

Wire together in-process script execution, event parsing, and registry writes into
the end-to-end deployment recording flow. This is the core pipeline that `treb run`
drives.

**Deliverables**
- `RunPipeline`: compile → link → execute → parse events → record to registry
- Pipeline stops at `ExecutedState` / `PreSimulationState` for structured data extraction
  before deciding whether to proceed to simulation/broadcast
- Deployment strategy detection (CREATE, CREATE2, CREATE3) from `ScriptResult.traces`
- Proxy detection heuristics (ERC1967 storage slots, bytecode patterns)
- Transaction recording with linked deployments
- Duplicate deployment detection and conflict resolution
- Dry-run mode (execute in EVM but don't broadcast — stop before `fill_metadata()`)
- `ContractsByArtifact::find_by_creation_code()` / `find_by_deployed_code()` for
  contract identification
- Integration tests using real compilation + execution against test Solidity contracts

**User stories:** 6
**Dependencies:** Phases 4, 5, 6, 7

---

## Phase 9 -- Commands: `version`, `networks`

Implement the two simplest commands to validate the full CLI dispatch path from
clap parsing through to formatted output.

**Deliverables**
- `treb version`: binary version, git commit, build date, Rust version, foundry crate version
- `treb networks`: list networks from `Config::rpc_endpoints` with chain ID resolution
  via `Config::get_rpc_url()` + provider calls
- `--json` output flag for both commands
- Colored/formatted table output via `comfy-table`
- Output formatting utilities (shared table builder, JSON printer)
- CLI integration tests (assert stdout)

**User stories:** 4
**Dependencies:** Phases 1, 3

---

## Phase 10 -- Commands: `init`, `config`

Project initialization and configuration management commands.

**Deliverables**
- `treb init`: detect Foundry project via `Config::load()`, create `.treb/` directory
  structure, write default config, install treb-sol library
- `treb init --force`: overwrite existing config
- `treb config show`: print merged effective config (foundry + treb layers)
- `treb config set <key> <value>`: write to `.treb/config.local.json`
- `treb config remove <key>`: remove from local config
- Validation that cwd is a Foundry project (presence of `foundry.toml`)
- Tests for init idempotency and config round-trips

**User stories:** 5
**Dependencies:** Phases 3, 4

---

## Phase 11 -- Commands: `list`, `show`

Read-only registry query commands with filtering, sorting, and detail views.

**Deliverables**
- `treb list` (alias `ls`): tabular deployment listing
- Filters: `--network`, `--namespace`, `--type`, `--tag`, `--contract`, `--label`, `--fork`, `--no-fork`
- `treb show <deployment>`: detailed single-deployment view (addresses, tx hashes,
  proxy info, ABI, tags, verification status)
- Multiple ID format resolution: `Counter`, `Counter:v2`, `staging/Counter`, address, full ID
- Fuzzy deployment name resolution when argument is ambiguous
- `--json` output for both commands
- Tests against fixture registries

**User stories:** 6
**Dependencies:** Phase 4, 9 (output utilities)

---

## Phase 12 -- Command: `run`

The flagship command. Execute a Forge script entirely in-process with full sender
configuration, event parsing, and deployment recording.

**Deliverables**
- `treb run <script>` end-to-end: compile → link → execute → parse → record
- `ScriptArgs` construction from CLI flags and treb config (path, sig, args, sender, network)
- Pipeline control: stop at `ExecutedState` for dry-run, proceed through
  `PreSimulationState` → `FilledTransactionsState` → `BundledState` → `BroadcastedState` for broadcast
- Console.log output display (decoded from `ScriptResult.logs`)
- Trace display for `--verbose` / `--debug` modes
- Network selection (`--network`, `--chain-id`, `--rpc-url`)
- Namespace selection (`--namespace`)
- `--dry-run` flag (execute in EVM only, no broadcast)
- `--env` flag for passing script parameters
- Interactive confirmation prompt before broadcast
- `--debug`, `--debug-json`, `--verbose`, `--dump-command` flags
- Progress output during each pipeline stage
- Integration tests with real Solidity test contracts

**User stories:** 8
**Dependencies:** Phase 8, 9

---

## Phase 13 -- Command: `verify`

Post-deployment contract verification using `forge-verify` crate directly.
No custom Etherscan/Sourcify HTTP clients — use foundry's verification pipeline.

**Deliverables**
- `treb-verify` workspace crate (thin wrapper around `forge-verify`)
- `treb verify <deployment>` (or `--all`): verify source code on explorer
- `forge-verify::VerificationProvider` integration: Etherscan, Sourcify, Blockscout
- `VerificationContext` construction from our registry data + `foundry-config`
- Map deployment → `VerifyArgs` (compiler settings from `Config`, constructor args from registry)
- Verification status tracking in registry
- `--verifier` flag to select backend (`--etherscan`, `--blockscout`, `--sourcify`)
- `--force` flag for re-verification
- `--all` flag to verify all unverified deployments
- Rate limiting via `RetryArgs`
- Tests with mock HTTP responses

**User stories:** 6
**Dependencies:** Phases 4, 11

---

## Phase 14 -- Commands: `tag`, `register`, `sync`

Registry enrichment commands: tagging, retroactive registration, and Safe
transaction synchronization.

**Deliverables**
- `treb tag <deployment>`: show tags (default)
- `treb tag <deployment> --add <tag>`: add version/label tag
- `treb tag <deployment> --remove <tag>`: remove tag
- Tag uniqueness validation (optional per-network unique tags)
- `treb register`: register deployments from a historical transaction
- Transaction tracing via alloy provider `debug_traceTransaction`
- `ContractsByArtifact` for matching traced bytecode to known contracts
- Flags: `--address`, `--contract`, `--contract-name`, `--tx-hash`, `--label`, `--skip-verify`
- `treb sync`: pull Safe Transaction Service state, update safe-txs.json and transaction statuses
- `--clean` and `--debug` flags for sync
- Tests for each subcommand against fixture data

**User stories:** 7
**Dependencies:** Phases 4, 7, 11

---

## Phase 15 -- Command: `gen deploy`

Solidity deployment script code generation with strategy and proxy support.

**Deliverables**
- `treb gen deploy <artifact>`: generate a Foundry deploy script from a compiled contract artifact
- Auto-detection of library vs contract (via `ContractsByArtifact` metadata)
- Strategy templates: CREATE, CREATE2, CREATE3
- Proxy templates: ERC1967, UUPS, Transparent, Beacon
- Template engine (handlebars or askama) for Solidity output
- Constructor argument scaffolding from `JsonAbi`
- `--output` flag for target file path (default: `script/Deploy<Name>.s.sol`)
- `--strategy`, `--proxy`, `--proxy-contract` flags
- Generated code compiles via in-process `ProjectCompiler` (integration test)

**User stories:** 5
**Dependencies:** Phase 5

---

## Phase 16 -- Command: `compose`

YAML-based multi-step deployment orchestration with a dependency graph.
Must match the Go version's compose file format.

**Deliverables**
- Compose YAML schema: `group`, `components` with `script`, `deps`, `env`
- Dependency graph construction and topological sort
- Sequential execution in dependency order (each component runs the Phase 12 pipeline)
- Per-component environment variable injection
- Profile support (`--profile`)
- `--dry-run` flag (print execution plan without running)
- `--resume` flag (skip already-completed components)
- Progress display with component status indicators
- Tests with fixture compose files

**User stories:** 7
**Dependencies:** Phases 12, 13, 14

---

## Phase 17 -- Safe Multisig and Governor Integration

Full Safe Transaction Service integration and Governor proposal support.

**Deliverables**
- `treb-safe` workspace crate
- Safe Transaction Service API client (propose tx, list pending, get confirmations, execute)
- Safe sender implementation: pipeline stops at `BundledState`, proposes to Safe instead of broadcasting
- Governor sender implementation: creates governance proposals from bundled transactions
- Governor proposal tracking in registry
- `treb sync` full implementation: poll Safe service, update registry on execution/confirmation
- Safe transaction status lifecycle (proposed → confirmed → executed / failed)
- EIP-712 typed data signing for Safe tx confirmation (via alloy)
- Multi-chain Safe service URL resolution
- Integration tests with mock Safe service

**User stories:** 8
**Dependencies:** Phases 6, 8, 14

---

## Phase 18 -- Fork Mode and Programmatic Anvil

Fork-mode workflows using `anvil` crate's programmatic API. No subprocess
management — Anvil runs in-process via `anvil::spawn()`.

**Deliverables**
- `anvil::NodeConfig` construction from treb config (chain ID, fork URL, fork block, accounts)
- `anvil::spawn(config)` → `(EthApi, NodeHandle)` for in-process Anvil
- CreateX factory pre-deployment via `EthApi` direct calls
- `treb dev anvil start|stop|restart|status|logs`: manage in-process Anvil instances
  (no PID files needed — we own the process)
- `treb fork enter <network>`: spawn Anvil fork, snapshot registry, update config to use Anvil RPC
- `treb fork exit [network|--all]`: stop Anvil, restore registry snapshot
- `treb fork revert [network|--all]`: EVM snapshot revert via `EthApi`
- `treb fork restart [network]`: respawn Anvil from latest block
- `treb fork status`: show current fork info
- `treb fork history [network]`: list fork command history with snapshots
- `treb fork diff [network]`: diff registry state vs. snapshot
- Fork state persistence (`.treb/fork-state.json`)
- Graceful cleanup on SIGINT/SIGTERM (drop `NodeHandle`)
- Tests for state transitions

**User stories:** 8
**Dependencies:** Phases 5, 4

---

## Phase 19 -- Commands: `prune`, `reset`, `migrate`

Housekeeping and migration commands for registry maintenance and config format
upgrades.

**Deliverables**
- `treb prune`: detect and remove invalid registry entries (missing on-chain, broken references)
- On-chain existence check via alloy provider `eth_getCode`
- `--dry-run` flag for prune (report without deleting)
- `--include-pending` flag
- `treb reset`: wipe registry with interactive confirmation, `--network` / `--namespace` scope
- `treb migrate`: detect legacy treb.toml v1 format, convert to v2
- Migrate deprecated foundry.toml sender config to treb.toml
- Registry format migration (versioned, forward-only)
- Backup creation before destructive operations
- Tests for each migration path

**User stories:** 6
**Dependencies:** Phases 3, 4, 11

---

## Phase 20 -- TUI Polish, Release Packaging, and Parasitic CI

Final UX pass, release artifacts, and CI automation for tracking foundry upstream.

**Deliverables**
- Interactive deployment selector (fuzzy search via `nucleo`) used by show, verify, tag, etc.
- Interactive network selector for commands that need `--network`
- Interactive multiselect for batch operations
- Confirmation prompts standardized across all destructive commands
- Colored output audit: consistent palette, `--no-color` / `NO_COLOR` env support
- Shell completions generation (bash, zsh, fish) via clap
- Long-form help text for every command
- Cross-compilation CI matrix (linux-x86_64, linux-aarch64, macos-x86_64, macos-aarch64)
- Release binary packaging (tar.gz, checksums)
- `trebup` installer script
- **Parasitic CI**: GitHub Actions workflow that:
  - Monitors foundry releases/tags
  - Creates a branch bumping the foundry pin
  - Runs `cargo check` / `cargo test`
  - On failure: opens an issue with the build errors for AI-assisted or manual fix
  - On success: auto-merges the version bump
- Final integration test suite covering end-to-end workflows

**User stories:** 8
**Dependencies:** all prior phases

---

## Dependency Graph (ASCII)

```
Phase 1  (scaffold + foundry deps)
  │
Phase 2  (types)
  │_________________________
  │              │          │
Phase 3  (config)   Phase 4  (registry)
  │    \______________|_____|
  │         │         │
Phase 5  (forge in-process)  Phase 6  (wallets)
  │    │    │                    │
  │  Phase 7  (treb events)     │
  │    │         │              │
  │  Phase 8  (recording pipeline) ── Phase 6
  │    │
Phase 9  (version/networks)
  │    │
Phase 10 (init/config)
  │    │
Phase 11 (list/show)          Phase 15 (gen deploy) ── Phase 5
  │    │
Phase 12 (run — in-process)
  │    │
Phase 13 (verify — forge-verify)
  │    │
Phase 14 (tag/register/sync)
  │    │
Phase 16 (compose) ── Phases 12, 13, 14
  │
Phase 17 (safe/governor) ── Phases 6, 8, 14
  │
Phase 18 (fork — anvil::spawn) ── Phases 5, 4
  │
Phase 19 (prune/reset/migrate) ── Phases 3, 4, 11
  │
Phase 20 (TUI/release/parasitic CI) ── all prior
```

---

## Summary Table

| Phase | Title                        | Stories | Crate(s) Added    | Foundry Crates Used | Depends On |
|------:|------------------------------|--------:|-------------------|---------------------|------------|
|     1 | Repository Scaffold          |       5 | treb-cli, treb-core | foundry-config, foundry-common | -- |
|     2 | Core Domain Types            |       6 | —                 | alloy-primitives, alloy-json-abi | 1 |
|     3 | Configuration System         |       7 | treb-config       | foundry-config | 2 |
|     4 | Registry System              |       6 | treb-registry     | — | 2 |
|     5 | In-Process Forge             |       7 | treb-forge        | foundry-common, forge-script, forge-script-sequence | 3 |
|     6 | Sender/Wallet System         |       5 | —                 | foundry-wallets | 3 |
|     7 | Event Parsing / ABI          |       6 | —                 | foundry-linking, alloy sol! | 5 |
|     8 | Deployment Recording         |       6 | —                 | — | 4, 5, 6, 7 |
|     9 | version, networks            |       4 | —                 | foundry-config | 1, 3 |
|    10 | init, config                 |       5 | —                 | foundry-config | 3, 4 |
|    11 | list, show                   |       6 | —                 | — | 4, 9 |
|    12 | run (in-process)             |       8 | —                 | forge-script | 8, 9 |
|    13 | verify (forge-verify)        |       6 | treb-verify       | forge-verify | 4, 11 |
|    14 | tag, register, sync          |       7 | —                 | foundry-evm (optional) | 4, 7, 11 |
|    15 | gen deploy                   |       5 | —                 | foundry-common | 5 |
|    16 | compose                      |       7 | —                 | — | 12, 13, 14 |
|    17 | Safe / Governor              |       8 | treb-safe         | alloy | 6, 8, 14 |
|    18 | Fork (anvil::spawn)          |       8 | —                 | anvil | 5, 4 |
|    19 | prune, reset, migrate        |       6 | —                 | alloy (provider) | 3, 4, 11 |
|    20 | TUI / Release / Parasitic CI |       8 | —                 | — | all |
| **Total** |                          | **126** |                   |                     | |

---

## Risks and Mitigations

### Foundry API instability
Foundry crates are internal and have no semver guarantees. Any foundry commit can
break our build.

**Mitigation**: Pin to a specific git commit. The Phase 20 parasitic CI workflow
automatically detects upstream changes, attempts builds, and either auto-merges
clean updates or flags breaking changes.

### Private module types
`forge-script` state types (`CompiledState`, `LinkedState`, `ExecutedState`, etc.)
are in private modules. We can call their `pub` methods via the return-type chain
but cannot name them in function signatures.

**Mitigation**: Wrap the entire pipeline call in a single function that takes
`ScriptArgs` and returns our own result type. Consider contributing upstream PRs
to make key modules `pub`.

### `std::process::exit(0)` in ProjectCompiler
`ProjectCompiler::compile()` calls `std::process::exit(0)` when there are no
input files to compile.

**Mitigation**: Validate input files before calling compile. Or catch via
`std::panic::catch_unwind` if they change to panic.

### Dependency weight / compile times
Foundry pulls in revm, solar, alloy, and many other heavy crates. Expect
significant compile times.

**Mitigation**: Use `cargo-binstall` or pre-built deps in CI. Accept the cost —
the architectural benefits outweigh it. Use `sccache` aggressively.

### Shell global singleton
Foundry's `sh_println!` output system uses a process-global `OnceLock<Mutex<Shell>>`.
Cannot intercept or redirect per-invocation.

**Mitigation**: Set our own Shell configuration at startup. For output we control,
use our own output layer. For foundry internal output, accept the global behavior.
