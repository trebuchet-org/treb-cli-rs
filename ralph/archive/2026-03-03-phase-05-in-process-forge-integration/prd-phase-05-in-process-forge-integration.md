# PRD: Phase 5 — In-Process Forge Integration

## Introduction

Phase 5 creates the `treb-forge` workspace crate — the bridge between treb's
configuration/registry system and foundry's compilation and script execution
pipeline. All forge functionality is accessed through Rust crate APIs; no
subprocess calls to `forge` are used anywhere.

This phase integrates three foundry crates:
- **`foundry-common`**: `ProjectCompiler` for in-process Solidity compilation,
  `ContractsByArtifact` for artifact lookup by name/bytecode
- **`forge-script`**: `ScriptArgs` state machine for script execution
  (`preprocess → compile → link → prepare_execution → execute`)
- **`forge-script-sequence`**: `ScriptSequence` and `BroadcastReader` for
  reading historical broadcast files

The `treb-forge` crate wraps these foundry internals behind a stable API that
downstream phases consume. Phase 7 (event parsing), Phase 8 (recording pipeline),
Phase 12 (`treb run`), and Phase 15 (`treb gen deploy`) all depend on this crate.

This phase depends on Phase 3 (`treb-config`) for resolved configuration and
`foundry-config::Config` integration. It does not depend on Phase 4 (registry) —
the forge crate compiles and executes scripts but does not record results.

---

## Goals

1. **In-process compilation** — Compile Solidity projects using
   `foundry-common::ProjectCompiler` without spawning a `forge build` subprocess.
   Guard against `ProjectCompiler`'s `std::process::exit(0)` on empty input.

2. **Programmatic script execution** — Construct `ScriptArgs` from treb
   configuration (not clap parsing) and drive the execution state machine through
   to `ExecutedState`, extracting `ScriptResult` with logs, traces, transactions,
   return values, and labeled addresses.

3. **Console.log decoding** — Decode Solidity `console.log` calls from
   `ScriptResult.logs` into human-readable strings using foundry's log decoding
   utilities.

4. **Artifact lookup** — Build and query `ContractsByArtifact` to resolve
   contracts by name, creation bytecode, or deployed bytecode from compilation
   output.

5. **Broadcast file reading** — Parse historical broadcast files via
   `forge-script-sequence::{ScriptSequence, BroadcastReader}` as a supplementary
   data source.

---

## User Stories

### US-001: treb-forge Crate Scaffold and Foundry Dependency Wiring

**Description:** Create the `treb-forge` workspace crate with its Cargo.toml,
module structure, and foundry crate dependencies. Wire up `forge-script`,
`forge-script-sequence`, and `foundry-common` so they compile and link
correctly alongside the existing workspace. Add foundry re-exports that
downstream crates will use.

**Acceptance Criteria:**
- `crates/treb-forge/` directory created with `Cargo.toml` and `src/lib.rs`
- Crate added to `[workspace.members]` in root `Cargo.toml`
- `treb-forge` added to `[workspace.dependencies]` with `path = "crates/treb-forge"`
- New workspace-level foundry dependencies added:
  - `forge-script = { git = "https://github.com/foundry-rs/foundry", tag = "v1.5.1" }`
  - `forge-script-sequence = { git = "https://github.com/foundry-rs/foundry", tag = "v1.5.1" }`
- Crate dependencies:
  - `treb-core` (workspace) — error types, alloy primitives
  - `treb-config` (workspace) — resolved config
  - `foundry-config` (workspace) — `Config`, project paths
  - `foundry-common` (workspace) — `ProjectCompiler`, `ContractsByArtifact`
  - `forge-script` (workspace) — `ScriptArgs`, `ScriptResult`
  - `forge-script-sequence` (workspace) — `ScriptSequence`, `BroadcastReader`
  - `alloy-primitives` (workspace) — `Address`, `B256`, `Bytes`
  - `serde` (workspace), `serde_json` (workspace)
  - `thiserror` (workspace), `anyhow` (workspace)
  - `tokio` (workspace) — async runtime for tests
  - `eyre` — foundry crates return `eyre::Result`, need conversion
- Module structure in `src/`:
  - `lib.rs` — public re-exports and module declarations
  - `compiler.rs` — compilation wrapper (US-002)
  - `script.rs` — script execution pipeline (US-003, US-004)
  - `artifacts.rs` — `ContractsByArtifact` wrapper (US-005)
  - `broadcast.rs` — broadcast file reading (US-006)
  - `console.rs` — console.log decoding (US-004)
  - `version.rs` — forge version detection (US-007)
- `lib.rs` declares all modules and re-exports key public types
- Any additional `[patch.crates-io]` entries needed for foundry's transitive
  dependencies are added to the root `Cargo.toml`
- `cargo check --workspace` passes with zero errors
- `cargo clippy --workspace` passes with zero warnings
- A trivial smoke test exists (e.g., assert `foundry_config::Config` is importable)

---

### US-002: In-Process Compilation via ProjectCompiler

**Description:** Wrap `foundry-common::ProjectCompiler` in a safe API that
compiles a Foundry project without risking `std::process::exit(0)`. The wrapper
validates inputs before calling `compile()` and converts the output into treb
types.

**Acceptance Criteria:**
- `compiler.rs` module with:
  - `CompilationOutput` struct containing:
    - `output: ProjectCompileOutput` (foundry's compilation result)
    - `project_root: PathBuf`
  - `compile_project(config: &Config) -> Result<CompilationOutput>`:
    - Calls `config.project()` to get the `Project`
    - Validates `project.paths.has_input_files()` returns `true` before
      compiling. Returns `TrebError::Forge("no Solidity input files found")`
      if empty
    - Creates `ProjectCompiler::new().quiet(true).bail(true)` and calls
      `.compile(&project)`
    - Converts `eyre::Result` errors to `TrebError::Forge` with the error
      message preserved
    - Returns `CompilationOutput` on success
  - `compile_files(config: &Config, files: Vec<PathBuf>) -> Result<CompilationOutput>`:
    - Same as `compile_project` but passes specific files via
      `ProjectCompiler::new().files(files)`
    - Still validates that the files vector is non-empty
- `CompilationOutput` provides accessor methods:
  - `has_compiler_errors(&self) -> bool` — checks if compilation output
    contains errors
  - `artifact_ids(&self) -> Vec<&ArtifactId>` — lists all artifact IDs from
    the output
- Error messages include the project root path for context
- Unit test: `compile_project` with empty project returns
  `TrebError::Forge` (not process exit)
- `cargo check` and `cargo clippy` pass

---

### US-003: ScriptArgs Construction from Treb Config

**Description:** Build `forge-script::ScriptArgs` programmatically from treb's
resolved configuration, without going through clap argument parsing. This
translates treb's config model into forge's execution parameters.

**Acceptance Criteria:**
- `script.rs` module with:
  - `ScriptConfig` builder struct with fields:
    - `script_path: String` — path to `.s.sol` script file
    - `sig: String` — function signature (default: `"run()"`)
    - `args: Vec<String>` — arguments to pass to the function
    - `target_contract: Option<String>` — specific contract in the file
    - `sender: Option<Address>` — transaction sender address
    - `rpc_url: Option<String>` — RPC endpoint URL
    - `chain_id: Option<u64>` — target chain ID
    - `fork_url: Option<String>` — fork RPC URL (for local execution)
    - `broadcast: bool` — whether to broadcast transactions
    - `slow: bool` — send transactions one at a time
    - `debug: bool` — enable debugger
    - `dry_run: bool` — execute but don't broadcast
    - `gas_estimate_multiplier: u64` — gas multiplier (default: 130)
    - `legacy: bool` — use legacy (non-EIP-1559) transactions
    - `non_interactive: bool` — skip confirmation prompts
    - `etherscan_api_key: Option<String>` — for verification
    - `verify: bool` — verify after deployment
  - `ScriptConfig::new(script_path: impl Into<String>) -> Self`
    constructor with sensible defaults
  - Builder methods for each field (returning `&mut Self` or `Self`)
  - `ScriptConfig::into_script_args(self) -> Result<ScriptArgs>`:
    - Constructs a `ScriptArgs` with `Default::default()` as base
    - Sets `path`, `sig`, `args`, `target_contract`
    - Sets `broadcast`, `slow`, `debug`, `non_interactive`
    - Sets `gas_estimate_multiplier`, `legacy`
    - Wires RPC/chain config through `ScriptArgs.evm` (EvmArgs)
    - Wires sender address through `ScriptArgs.evm` if provided
    - Wires etherscan key and verify flag
    - Returns the configured `ScriptArgs`
  - `build_script_config(resolved: &ResolvedConfig, script_path: &str) -> Result<ScriptConfig>`:
    - Extracts RPC URL from resolved config's network
    - Extracts sender address from the default sender in resolved config
    - Sets `slow` from resolved config
    - Returns a pre-populated `ScriptConfig`
- Unit test: `ScriptConfig::new("script/Deploy.s.sol").into_script_args()` produces
  a valid `ScriptArgs` with correct `path` and default `sig` of `"run()"`
- Unit test: builder methods set all fields correctly on the resulting `ScriptArgs`
- Unit test: `build_script_config` extracts sender address from
  `ResolvedConfig.senders` when a default sender is present
- `cargo check` and `cargo clippy` pass

---

### US-004: Script Execution Pipeline and Result Extraction

**Description:** Drive the `ScriptArgs` state machine through the full execution
pipeline (`preprocess → compile → link → prepare_execution → execute`) and
extract structured results. Includes console.log decoding from execution logs.

**Acceptance Criteria:**
- `script.rs` additions:
  - `ExecutionResult` struct containing:
    - `success: bool` — whether script execution succeeded
    - `logs: Vec<String>` — decoded console.log messages
    - `raw_logs: Vec<Log>` — raw EVM logs for downstream event parsing
    - `gas_used: u64` — total gas consumed
    - `returned: Bytes` — raw return value from the script
    - `labeled_addresses: HashMap<Address, String>` — address labels from the script
    - `transactions: Option<BroadcastableTransactions>` — transactions generated
      by the script (if any)
    - `traces: Traces` — execution traces for debug/verbose output
  - `execute_script(args: ScriptArgs) -> Result<ExecutionResult>`:
    - Calls `args.preprocess().await`
    - Chains `.compile()` on the result
    - Chains `.link().await`
    - Chains `.prepare_execution().await`
    - Chains `.execute().await`
    - Extracts `ScriptResult` from the `ExecutedState`
    - Decodes console.log messages from `ScriptResult.logs` (see console.rs)
    - Maps all `eyre::Result` errors into `TrebError::Forge` with descriptive
      messages indicating which pipeline stage failed
    - Returns `ExecutionResult`
- `console.rs` module:
  - `decode_console_logs(logs: &[Log]) -> Vec<String>`:
    - Filters logs for console.log events (address
      `0x000000000000000000636F6e736F6c652e6c6f67`)
    - Decodes each log's data using foundry's console log decoding
      (`alloy_sol_types` or foundry's built-in decoder)
    - Returns decoded log strings in order
- Errors from each pipeline stage are wrapped with context (e.g.,
  `"forge compilation failed: ..."`, `"forge linking failed: ..."`,
  `"forge execution failed: ..."`)
- `cargo check` and `cargo clippy` pass

---

### US-005: ContractsByArtifact Wrapper and Artifact Queries

**Description:** Wrap `foundry-common::ContractsByArtifact` to provide artifact
lookup methods that downstream phases use for contract identification — matching
deployed bytecode to known artifacts, looking up contracts by name, etc.

**Acceptance Criteria:**
- `artifacts.rs` module with:
  - `ArtifactIndex` struct wrapping `ContractsByArtifact`:
    - `from_compile_output(output: &CompilationOutput) -> Result<Self>`:
      builds the index from compilation output using
      `ContractsByArtifact::new()` or the builder pattern
    - `find_by_name(&self, name: &str) -> Result<Option<ArtifactMatch>>`:
      looks up a contract by name or identifier via
      `find_by_name_or_identifier`
    - `find_by_creation_code(&self, code: &[u8]) -> Option<ArtifactMatch>`:
      looks up by creation bytecode
    - `find_by_deployed_code(&self, code: &[u8]) -> Option<ArtifactMatch>`:
      looks up by deployed (runtime) bytecode
    - `inner(&self) -> &ContractsByArtifact` — access the underlying type
      for passing to foundry APIs that require it
  - `ArtifactMatch` struct:
    - `artifact_id: ArtifactId` — foundry's artifact identifier
      (source path + contract name)
    - `name: String` — contract name
    - `abi: JsonAbi` — the contract's ABI
    - `has_bytecode: bool` — whether creation bytecode is available
    - `has_deployed_bytecode: bool` — whether runtime bytecode is available
- Conversion from foundry's `(&ArtifactId, &ContractData)` tuple to
  `ArtifactMatch` is implemented
- Unit test: construct an `ArtifactMatch` and verify field access
- `cargo check` and `cargo clippy` pass

---

### US-006: Broadcast File Reader

**Description:** Wrap `forge-script-sequence::{ScriptSequence, BroadcastReader}`
to read historical broadcast files from foundry's `broadcast/` directory. This is
a supplementary data source for reading past deployment data — not our primary
execution path.

**Acceptance Criteria:**
- `broadcast.rs` module with:
  - `BroadcastData` struct:
    - `sequence: ScriptSequence` — the parsed broadcast sequence
    - `chain_id: u64` — chain ID of the broadcast
    - `timestamp: u128` — broadcast timestamp
  - `read_latest_broadcast(config: &Config, contract_name: &str, chain_id: u64) -> Result<BroadcastData>`:
    - Resolves the broadcast directory from `config.broadcast` path
    - Constructs a `BroadcastReader::new(contract_name, chain_id, &broadcast_path)`
    - Calls `reader.read_latest()` to get the most recent broadcast
    - Converts `eyre::Result` to `TrebError::Forge`
    - Returns `BroadcastData`
  - `read_all_broadcasts(config: &Config, contract_name: &str, chain_id: u64) -> Result<Vec<BroadcastData>>`:
    - Same setup but calls `reader.read()` for all broadcasts
    - Returns sorted by timestamp (newest first)
  - `BroadcastTransaction` struct (simplified view of `TransactionWithMetadata`):
    - `hash: Option<B256>`
    - `contract_name: Option<String>`
    - `contract_address: Option<Address>`
    - `function: Option<String>`
    - `tx_type: String` — "CREATE", "CREATE2", "CALL", etc.
  - `BroadcastData::transactions(&self) -> Vec<BroadcastTransaction>`:
    - Iterates `sequence.transactions` and maps to `BroadcastTransaction`
- Error messages include the broadcast directory path and contract name for
  debugging
- Unit test: `BroadcastTransaction` field access works correctly
- `cargo check` and `cargo clippy` pass

---

### US-007: Forge Version Detection and Integration Tests

**Description:** Implement forge version detection from foundry crate metadata.
Write integration tests against an embedded Foundry project fixture to validate
the full compilation and execution pipeline end-to-end.

**Acceptance Criteria:**
- `version.rs` module with:
  - `ForgeVersion` struct:
    - `version: String` — semver version string (e.g., "1.5.1")
    - `commit: Option<String>` — git commit hash if available
  - `detect_forge_version() -> ForgeVersion`:
    - Reads version from foundry-config's `CARGO_PKG_VERSION` env var
      (compiled into the binary)
    - Returns a `ForgeVersion` struct
  - `ForgeVersion::display_string(&self) -> String`:
    - Formats as `"forge v1.5.1"` or `"forge v1.5.1 (abc1234)"` with commit
- Integration test fixture:
  - `crates/treb-forge/tests/fixtures/sample-project/` containing:
    - `foundry.toml` — minimal foundry config
    - `src/Counter.sol` — simple contract (increment, get count)
    - `script/Deploy.s.sol` — simple deployment script using `vm.broadcast()`
  - These are real Solidity files that can be compiled by foundry
- Integration test: `compile_project` against the fixture project succeeds
  and returns artifacts including `Counter`
- Integration test: `ArtifactIndex::from_compile_output` returns an index
  where `find_by_name("Counter")` returns a match
- Integration test: `detect_forge_version()` returns a non-empty version string
- All integration tests are gated behind `#[cfg(test)]` and use the fixture
  project
- `cargo test --workspace` passes (all existing tests plus new ones)
- `cargo clippy --workspace` passes

---

## Functional Requirements

- **FR-1:** The `treb-forge` crate is the single owner of all foundry compilation
  and script execution integration. Other treb crates never import `forge-script`,
  `forge-script-sequence`, or use `ProjectCompiler` directly — they go through
  `treb-forge`'s public API.

- **FR-2:** `ProjectCompiler::compile()` must never be called when there are no
  input files, as it calls `std::process::exit(0)`. The wrapper validates inputs
  and returns a `TrebError::Forge` instead.

- **FR-3:** `ScriptArgs` is constructed programmatically using `Default::default()`
  and field assignment. Treb never invokes clap parsing to build `ScriptArgs` —
  the CLI layer in `treb-cli` will use `ScriptConfig` from this crate.

- **FR-4:** All `eyre::Result` errors from foundry crates are converted to
  `TrebError::Forge` with the original error message preserved. No `eyre` errors
  escape the `treb-forge` crate boundary.

- **FR-5:** The execution pipeline stops at `ExecutedState` by default. Advancing
  to simulation/broadcast is the caller's responsibility (Phase 8/12). This crate
  provides the execution result; it does not broadcast transactions.

- **FR-6:** Console.log decoding handles all Solidity `console.log` variants
  (string, uint, int, bool, address, bytes, mixed). Unrecognized log formats are
  silently skipped (not errors).

- **FR-7:** `ContractsByArtifact` queries are exposed through `ArtifactIndex`
  which provides a stable API. Callers can also access the raw
  `ContractsByArtifact` via `inner()` for passing to foundry APIs that require it.

- **FR-8:** Broadcast file reading is a supplementary feature for reading
  historical data. It does not affect the primary compilation/execution path.

- **FR-9:** The forge version string is compiled into the binary at build time.
  It reflects the pinned foundry tag version, not a runtime `forge --version`
  call.

---

## Non-Goals

- **No transaction broadcasting** — The execution pipeline stops at
  `ExecutedState`. Broadcasting (advancing through `PreSimulationState` →
  `BundledState` → `BroadcastedState`) is Phase 8/12's responsibility.
- **No event parsing** — Extracting treb-specific events (`ContractDeployed`,
  `ProxyDeployed`, etc.) from execution logs belongs to Phase 7. This phase
  provides the raw logs.
- **No registry writes** — Recording deployments/transactions to the registry
  is Phase 8. This crate compiles and executes; it does not persist results.
- **No wallet/sender integration** — Wiring `WalletSigner` into `ScriptArgs`
  is Phase 6. This phase constructs `ScriptArgs` with sender address but does
  not handle key management.
- **No CLI command implementation** — `treb run` is Phase 12. This crate is the
  library layer.
- **No foundry Shell global configuration** — Foundry's `sh_println!` uses a
  process-global `OnceLock<Mutex<Shell>>`. This phase does not attempt to
  intercept or redirect it. Quiet mode via `ProjectCompiler::quiet(true)` is
  sufficient.

---

## Technical Considerations

### New Workspace Dependencies

| Crate | Source | Purpose |
|---|---|---|
| `forge-script` | `git = "...", tag = "v1.5.1"` | `ScriptArgs` state machine, `ScriptResult` |
| `forge-script-sequence` | `git = "...", tag = "v1.5.1"` | `ScriptSequence`, `BroadcastReader`, `TransactionWithMetadata` |
| `eyre` | crates.io | Error conversion from foundry APIs |

`foundry-common` and `foundry-config` are already in the workspace from Phase 1.

### Foundry State Machine Privacy

The intermediate state types (`PreprocessedState`, `CompiledState`, `LinkedState`,
`PreExecutionState`, `ExecutedState`) live in private modules within
`forge-script`. We can call their `pub` methods via the return-type chain but
cannot name them in function signatures. The `execute_script` function must
therefore be a single function that chains all stages and returns our own
`ExecutionResult` type — it cannot accept or return intermediate states.

### ProjectCompiler::compile() Exit Guard

`ProjectCompiler::compile()` at line 157-159 of `foundry-common/src/compile.rs`:
```rust
if !project.paths.has_input_files() && self.files.is_empty() {
    sh_println!("Nothing to compile")?;
    std::process::exit(0);
}
```

Our guard checks `project.paths.has_input_files()` (or `!files.is_empty()` for
the file-specific variant) before calling `compile()`. This is a runtime check,
not `catch_unwind`.

### eyre → TrebError Conversion

Foundry crates return `eyre::Result`. The conversion pattern:

```rust
foundry_call().map_err(|e| TrebError::Forge(format!("stage failed: {e}")))?;
```

The `eyre::Report` message chain is preserved in the `TrebError::Forge` string.

### Module Layout

```
crates/treb-forge/
├── Cargo.toml
├── src/
│   ├── lib.rs          (re-exports, module declarations)
│   ├── compiler.rs     (ProjectCompiler wrapper)
│   ├── script.rs       (ScriptConfig, execute_script, ExecutionResult)
│   ├── artifacts.rs    (ArtifactIndex wrapper around ContractsByArtifact)
│   ├── broadcast.rs    (BroadcastReader wrapper, BroadcastData)
│   ├── console.rs      (console.log decoding)
│   └── version.rs      (ForgeVersion detection)
└── tests/
    └── fixtures/
        └── sample-project/
            ├── foundry.toml
            ├── src/
            │   └── Counter.sol
            └── script/
                └── Deploy.s.sol
```

### Integration Test Strategy

Integration tests require a real Solidity compilation toolchain (solc). The
fixture project is kept minimal (one contract, one script) to minimize compile
time and external dependencies. Tests that call `compile_project` or
`execute_script` are integration tests, not unit tests — they require solc to be
installed.

If solc is not available in CI, these tests should be gated behind a feature flag
or cargo test filter (e.g., `#[ignore]` with `--ignored` in CI where solc is
available).

### Dependency on treb-config

`treb-forge` depends on `treb-config` for `ResolvedConfig` (used by
`build_script_config` to extract RPC URLs and sender addresses). It also directly
depends on `foundry-config` for `Config` (used by `compile_project` and broadcast
reading).
