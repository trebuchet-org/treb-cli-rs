# Plan Amendment: Foundry Crates as Direct Dependencies

The MASTER_PLAN treats forge as a black-box subprocess (`forge build`, `forge script`,
etc.). We have the foundry monorepo checked out locally and can depend on its crates
directly via `path` dependencies. This amendment documents which foundry crates are
useful, how they change each phase, and what new risks they introduce.

---

## Foundry Crate Inventory

### Tier 1 -- High-value, stable public API, direct replacement for planned work

| Crate | Package name | What it provides |
|-------|-------------|------------------|
| `crates/config` | `foundry-config` | Complete `foundry.toml` parser. The `Config` struct handles profiles, `[rpc_endpoints]`, `[etherscan]`, solc settings, remappings, EVM options, project paths -- everything. `Config::load()` / `Config::load_with_root()` returns a fully-resolved config. Exports `RpcEndpoints`, `ResolvedRpcEndpoints`, `Chain`, `NamedChain`. |
| `crates/common` | `foundry-common` | `ProjectCompiler` (wraps `foundry-compilers` with progress/reporting), `ContractsByArtifact` (artifact lookup by name, bytecode, deployed bytecode), `ProviderBuilder` (retry-aware alloy provider from config or URL), ABI helpers, file utilities. |
| `crates/script-sequence` | `forge-script-sequence` | `ScriptSequence` / `TransactionWithMetadata` / `AdditionalContract` / `BroadcastReader` -- the exact types used inside forge broadcast JSON files. These are the types we need to **parse** broadcast output. Fully public, serde-enabled, no CLI coupling. |
| `crates/verify` | `forge-verify` | `VerifyArgs`, `VerifierArgs`, `VerificationProvider` trait, `EtherscanVerificationProvider`, `SourcifyVerificationProvider`, `VerificationContext`, `RetryArgs`. The entire verification pipeline is exposed as a library. |
| `crates/wallets` | `foundry-wallets` | `WalletSigner` enum (Local/Ledger/Trezor/Browser/AWS-KMS/GCP-KMS/Turnkey), `WalletOpts` / `MultiWalletOpts` / `RawWalletOpts` clap argument structs. Handles all signer resolution from CLI flags. |
| `crates/linking` | `foundry-linking` | `Linker` -- CREATE2 and nonce-based library linking. Small, focused crate with clean public API. |
| `crates/anvil` | `anvil` | `NodeConfig` + `anvil::spawn(config)` / `anvil::try_spawn(config)` returns `(EthApi, NodeHandle)`. Programmatic Anvil -- no subprocess needed. `NodeConfig` supports fork URL, chain ID, block time, accounts, hardfork selection, and everything else. |

### Tier 2 -- Useful internals, but coupled to forge CLI patterns

| Crate | Package name | Notes |
|-------|-------------|-------|
| `crates/script` | `forge-script` | `ScriptArgs` is the entry point (`ScriptArgs::run_script()` drives the full state machine: preprocess -> compile -> link -> execute -> simulate -> bundle -> broadcast -> verify). However, **all internal modules are `mod` (private)**, not `pub mod`. The only public types are `ScriptArgs`, `ScriptResult`, and `ScriptConfig`. We can call `ScriptArgs::run_script()` to get full script execution, or `ScriptArgs::preprocess()` to get partial pipeline access, but we cannot cherry-pick internal stages. The `clap` args are also tightly coupled. |
| `crates/forge` | `forge` | The top-level forge binary crate. Depends on everything. Primarily useful as a reference, not as a library dependency. |
| `crates/evm/traces` | `foundry-evm-traces` | Trace decoding/rendering. Useful if we want to display decoded traces for `--verbose` / `--debug` modes, but heavy dependency. |
| `crates/evm/evm` | `foundry-evm` | The core EVM execution layer: `Backend`, `ExecutorBuilder`, cheatcode infrastructure. Required if we want to run scripts in-process via `forge-script`. |

### Tier 3 -- External crates used by foundry (also useful to us directly)

| Crate | Notes |
|-------|-------|
| `foundry-compilers` (crates.io) | Solc management, project compilation, artifact types. Already used by `foundry-config` and `foundry-common`. |
| `foundry-block-explorers` (crates.io) | Etherscan/Blockscout API client. Used by `forge-verify` internally. |
| `alloy-*` | Foundry pins alloy 1.4.x. We should match these versions exactly. |

---

## Phase-by-Phase Impact

### Phase 1 -- Repository Scaffold
**No change.** We still set up our own workspace. But we add `foundry-config`,
`foundry-common`, and `forge-script-sequence` as `path` dependencies in the
workspace `Cargo.toml` from day one. Pin to the same alloy/revm versions foundry uses.

### Phase 2 -- Core Domain Types
**Minor change.** Our `Address`, `TxHash`, `B256` newtypes should wrap (or re-export)
`alloy-primitives` at the exact same version foundry uses. The `Contract` struct in
our registry can borrow ABI types from `alloy-json-abi::JsonAbi` directly.
`foundry-common::ContractData` is close to what we need but is oriented around
compilation artifacts, not registry records -- we still define our own domain types.

### Phase 3 -- Configuration System
**Dramatically simpler.** Replace the entire `treb-config` foundry.toml parser with a
direct dependency on `foundry-config::Config`.

What we import directly:
- `Config::load()` / `Config::load_with_root()` -- full foundry.toml parsing
- `Config::rpc_endpoints` / `Config::get_rpc_url()` -- RPC endpoint resolution
- `Config::get_etherscan_config()` -- Etherscan API key resolution
- `Config::project()` -- returns a configured `foundry-compilers::Project`
- Profile handling, remappings, solc settings -- all free

What we still build:
- `treb.toml` v1 and v2 parsing (treb-specific, not in foundry)
- `.treb/config.local.json` read/write
- Layered merge of treb config on top of foundry config
- `.env` loading (but `dotenvy` is trivial)

**Estimated effort reduction: ~50% of Phase 3 stories.**

### Phase 4 -- Registry System
**No change.** The registry is treb-specific JSON. Foundry has no equivalent.

### Phase 5 -- Forge Subprocess Integration
**Architecturally split.** This is the phase most affected. The plan calls for
shelling out to `forge build` and `forge script`. With foundry crates, we have two
options:

**Option A: In-process execution (recommended for build, partial for script)**
- **Compilation:** Use `foundry-common::ProjectCompiler` + `Config::project()` to
  compile in-process. No subprocess. This gives us `ProjectCompileOutput` with all
  artifacts, ABI, bytecode -- everything the plan's "compiler artifact parser" was
  going to get by reading JSON files from `out/`.
- **Script execution:** `forge-script::ScriptArgs::run_script()` runs the full
  pipeline in-process (compile -> link -> EVM execute -> simulate -> broadcast).
  However, its internals are private, so we cannot easily insert our event-parsing
  step between "execute" and "broadcast". Two sub-options:
  - **A1:** Use `ScriptArgs::run_script()` end-to-end, then parse the broadcast
    output files after the fact (using `forge-script-sequence::BroadcastReader`).
    This is the simplest path and is what the Go version effectively does.
  - **A2:** Fork/vendor `forge-script` to make key internal modules public, allowing
    us to insert custom logic between pipeline stages. Higher effort, maintenance
    burden.
- **Broadcast parsing:** Use `forge-script-sequence::ScriptSequence` and
  `BroadcastReader` directly. These types exactly match the JSON format forge writes.
  No custom parser needed.

**Option B: Subprocess for script, in-process for everything else**
- Shell out to `forge script` for execution/broadcast (as the current plan describes)
- Use `foundry-common::ProjectCompiler` for compilation
- Use `forge-script-sequence` types for broadcast parsing
- This is the safest option and still eliminates custom artifact parsing

**Recommendation:** Option B for initial implementation (Phases 5-12), with a path
to Option A1 later. In-process compilation is safe and high-value. Script execution
as subprocess is simpler to reason about and debug. Broadcast parsing via
`forge-script-sequence` types is pure upside.

What we import directly:
- `foundry-common::ProjectCompiler` -- replaces custom `forge build` subprocess + output parser
- `Config::project()` -- replaces custom `Project` construction
- `forge-script-sequence::{ScriptSequence, TransactionWithMetadata, AdditionalContract, BroadcastReader}` -- replaces custom broadcast JSON parser

What we still build:
- `ForgeRunner` trait (but the default impl can use in-process compilation)
- `forge script` subprocess invocation (for script execution/broadcast)
- Forge version detection

**Estimated effort reduction: ~40% of Phase 5 stories.** The broadcast parser and
artifact parser stories are effectively eliminated.

### Phase 6 -- Sender System
**Significantly simpler.** `foundry-wallets` provides the entire signer stack.

What we import directly:
- `WalletSigner` enum -- handles Local, Ledger, Trezor, Browser, AWS KMS, GCP KMS, Turnkey
- `WalletOpts` / `MultiWalletOpts` / `RawWalletOpts` -- CLI argument parsing for wallet selection
- Keystore file support, mnemonic support, HD path handling

What we still build:
- Sender abstraction layer that maps our config/CLI to `WalletSigner` selection
- Safe sender stub (foundry-wallets doesn't know about Safe)
- Governor sender stub
- Forge flag generation for subprocess mode (if we keep subprocess for scripts)

**Estimated effort reduction: ~40% of Phase 6 stories.** The private key, Ledger,
and Trezor stories collapse into "wire up foundry-wallets."

### Phase 7 -- Event Parsing and ABI Bindings
**No change.** `ITrebEvents` and `ICreateX` are treb-specific. Foundry has no
knowledge of these. We still use `alloy::sol!` macro and build our own event decoder.

### Phase 8 -- Deployment Recording Pipeline
**Minor simplification.** `ContractsByArtifact::find_by_creation_code()` and
`find_by_deployed_code()` from `foundry-common` can help with deployment strategy
detection and contract identification from bytecode. The `Linker` from
`foundry-linking` handles library detection.

### Phase 9 -- Commands: `version`, `networks`
**`networks` becomes simpler.** `Config::rpc_endpoints` + `Config::get_rpc_url()`
gives us the full `[rpc_endpoints]` section already resolved, including env var
interpolation and MESC support. We just iterate and display.

### Phase 10 -- Commands: `init`, `config`
**Minor simplification.** `Config::load()` handles detection of foundry project root.

### Phase 11-12 -- `list`, `show`, `run`
**`run` benefits from broadcast parsing.** If using subprocess mode, the broadcast
file parsing is handled by `forge-script-sequence::BroadcastReader`. If using
in-process mode, `ScriptArgs::run_script()` handles the entire flow.

### Phase 13 -- Command: `verify`
**Dramatically simpler.** `forge-verify` provides the complete verification pipeline
as a library.

What we import directly:
- `VerifyArgs` / `VerifierArgs` -- verification argument types
- `VerificationProvider` trait + `EtherscanVerificationProvider` + `SourcifyVerificationProvider`
- `VerificationContext` -- compiler settings extraction for verification
- `RetryArgs` -- retry configuration

What we still build:
- Integration with our registry (map deployment -> VerifyArgs)
- Verification status tracking in registry
- `--all` flag to verify all unverified deployments
- CLI wiring

**Estimated effort reduction: ~60% of Phase 13 stories.** The Etherscan client,
Sourcify client, compiler settings extraction, and constructor arg encoding stories
are eliminated.

### Phase 14 -- Commands: `tag`, `register`, `sync`
**`register` benefits from trace decoding.** `foundry-evm-traces` can help decode
`debug_traceTransaction` results, but this is a heavy dependency. Probably not worth
pulling in just for this.

### Phase 15 -- Command: `gen deploy`
**No change.** Code generation is treb-specific.

### Phase 16 -- Command: `compose`
**No change.** Compose orchestration is treb-specific.

### Phase 17 -- Safe Multisig and Governor Integration
**Minor benefit.** `foundry-wallets::WalletSigner` handles the signing side, but the
Safe Transaction Service API client and Governor proposal logic are treb-specific.

### Phase 18 -- Fork Mode and Anvil Management
**Dramatically simpler.** The `anvil` crate provides programmatic spawning.

What we import directly:
- `anvil::NodeConfig` -- full Anvil configuration (chain ID, fork URL, fork block,
  accounts, gas limit, hardfork, block time, etc.)
- `anvil::spawn(config)` / `anvil::try_spawn(config)` -- returns `(EthApi, NodeHandle)`
- `EthApi` -- full Ethereum JSON-RPC API for interacting with the node
- `NodeHandle` -- server address, shutdown control

What we still build:
- Fork state persistence (`.treb/fork-state.json`)
- Registry snapshot/restore around fork enter/exit
- CreateX factory pre-deployment (but we can call `EthApi` directly)
- `treb fork` subcommand UI and state machine
- PID file / log capture (unnecessary -- we own the process in-memory)

**Estimated effort reduction: ~50% of Phase 18 stories.** The entire "Anvil process
manager (PID file, log capture)" story is eliminated. Start/stop/restart become
trivial in-process operations. We get better error handling and no subprocess race
conditions.

### Phase 19-20 -- Housekeeping and Polish
**No significant change.**

---

## What Can Be Merged or Eliminated

1. **Phase 5 broadcast parser story** -- eliminated entirely by `forge-script-sequence`
2. **Phase 5 artifact parser story** -- eliminated by in-process compilation via `foundry-common::ProjectCompiler`
3. **Phase 6 Ledger/Trezor/private-key stories** -- collapse into one story: "integrate `foundry-wallets`"
4. **Phase 13 Etherscan/Sourcify/Blockscout client stories** -- collapse into one story: "integrate `forge-verify`"
5. **Phase 18 Anvil process manager story** -- eliminated by programmatic `anvil::spawn()`

Rough estimate: **15-20 stories eliminated or significantly reduced** out of 124 total.

---

## New Risks

### 1. Foundry crate API stability
Foundry does not publish these crates to crates.io (except `foundry-compilers`,
`foundry-block-explorers`, `foundry-fork-db`). The internal crates are versioned at
the workspace level (`1.6.0` currently) but have no semver stability guarantee. Any
foundry commit can break our build.

**Mitigation:** Pin to a specific foundry git commit or tag. Use `path` dependencies
during development, switch to `git` dependencies with a pinned `rev` for releases.
Vendor the crates if API churn becomes unmanageable.

### 2. Dependency weight
Foundry crates pull in large dependency trees: `revm`, `solar`, `foundry-compilers`,
the full alloy stack. The `anvil` crate alone is enormous. This will significantly
increase compile times and binary size.

**Mitigation:**
- Use feature flags to minimize what gets compiled (e.g., `anvil` without `cli` feature)
- Consider making `anvil` integration an optional feature of treb-cli
- Accept the compile time cost -- foundry's `profile.dev.package` optimizations help
- Tier the dependencies: `foundry-config` and `forge-script-sequence` are lightweight;
  `anvil` and `forge-script` are heavy

### 3. Rust toolchain version
Foundry requires `rust-version = "1.89"` (Rust edition 2024). We must use at least
this version. This is not a problem today but locks us to nightly-ish toolchain
features.

### 4. Version synchronization
We must keep our alloy/revm versions exactly aligned with foundry's pinned versions.
Mismatched versions will cause duplicate type errors (e.g., foundry's `Address` !=
our `Address` if alloy versions differ). The foundry workspace `Cargo.toml` is the
source of truth.

**Mitigation:** Import alloy/revm through foundry's re-exports where possible. Use
`[patch.crates-io]` in our workspace to force version alignment if needed.

### 5. `forge-script` private internals
The `forge-script` crate's internal modules (build, execute, simulate, broadcast,
etc.) are all private (`mod`, not `pub mod`). We can only use the top-level
`ScriptArgs::run_script()` and `ScriptArgs::preprocess()`. If we need finer-grained
control over the script pipeline, we would need to either:
- Fork and modify `forge-script`
- Contribute upstream PRs to make modules public
- Use subprocess fallback for script execution

### 6. Breaking changes in broadcast JSON format
If foundry changes the `ScriptSequence` or `TransactionWithMetadata` serde format,
our parsing breaks. This is the same risk as the subprocess approach (we'd be parsing
the same JSON), but now the risk is a Rust compile error rather than a silent runtime
parse failure -- which is actually better.

---

## Recommended Dependency Strategy

```
Phase 1-3:   foundry-config, foundry-common (lightweight, high value)
Phase 5:     + forge-script-sequence (broadcast parsing)
Phase 6:     + foundry-wallets (signer stack)
Phase 13:    + forge-verify (verification pipeline)
Phase 18:    + anvil (programmatic node, heavy but worth it)
```

Add dependencies incrementally. Start with the lightweight crates that have the
highest value-to-weight ratio. Defer the heavy crates (`anvil`, `forge-script`,
`foundry-evm`) until the phases that need them.

---

## Summary

Using foundry crates as direct Rust dependencies eliminates roughly **15-20 stories**
worth of work, with the biggest wins in:

- **Configuration parsing** (Phase 3): `foundry-config` replaces our custom `foundry.toml` parser
- **Broadcast parsing** (Phase 5): `forge-script-sequence` gives us exact type compatibility
- **Compilation** (Phase 5): `foundry-common::ProjectCompiler` replaces subprocess `forge build`
- **Wallet/signer** (Phase 6): `foundry-wallets` replaces our custom signer implementations
- **Verification** (Phase 13): `forge-verify` provides the entire pipeline as a library
- **Anvil management** (Phase 18): `anvil::spawn()` replaces subprocess management

The main trade-offs are increased compile times, tight version coupling with foundry,
and reliance on unstable internal APIs. These are manageable with version pinning and
incremental adoption.
