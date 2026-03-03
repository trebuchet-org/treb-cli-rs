# PRD: Phase 8 — Deployment Recording Pipeline

## Introduction

Phase 8 wires together the building blocks from Phases 4–7 into the end-to-end
deployment recording flow that `treb run` will drive. When a forge script
executes, it produces raw EVM logs, traces, and transaction data. Phase 7 already
decodes those logs into structured events (`ExtractedDeployment`,
`ProxyRelationship`, `TransactionSimulated`). Phase 4 provides the `Registry`
facade for persisting deployments and transactions. This phase bridges the gap:
it converts forge-domain alloy types into registry-compatible core domain types,
orchestrates the compile → execute → parse → record pipeline, handles duplicate
detection, and supports dry-run mode.

All pipeline code lives in a new `pipeline` module within the existing
`treb-forge` crate. This requires adding `treb-registry` and `treb-config` as
dependencies of `treb-forge`. No new workspace crates are created.

---

## Goals

1. **Type-safe conversion layer** — Convert alloy-typed forge output
   (`ExtractedDeployment`, `ProxyRelationship`, `TransactionSimulated`) into
   string-serialized core domain types (`Deployment`, `Transaction`) with zero
   data loss, matching the Go version's JSON format exactly.

2. **End-to-end pipeline** — Provide a `RunPipeline` that takes a script path
   and config, then executes compile → execute → decode events → hydrate domain
   types → record to registry, returning a structured result summary.

3. **Duplicate detection** — Before recording, check the registry for existing
   deployments by address+chainId and by deployment ID
   (`namespace/chainId/contractName:label`), with configurable conflict
   resolution (error, skip, or update).

4. **Dry-run support** — Execute the script in EVM and produce the full
   hydrated result (deployments, transactions) without writing to the registry,
   enabling `treb run --dry-run` in Phase 12.

---

## User Stories

### US-001: Pipeline Module Scaffold and Context Types

**Description:** Create the `pipeline` module within `treb-forge` and define the
context, configuration, and result types that the pipeline operates on. Add
`treb-registry` and `treb-config` as dependencies of `treb-forge`. Define
`PipelineContext` (runtime context: namespace, chain_id, script_path, git commit)
and `PipelineResult` (what was recorded: deployments, transactions, collisions,
skipped duplicates).

**Changes:**
- Add `treb-registry` and `treb-config` to `treb-forge`'s `Cargo.toml`
  dependencies
- Create `crates/treb-forge/src/pipeline/mod.rs` — module declarations and
  re-exports
- Create `crates/treb-forge/src/pipeline/context.rs` with:
  - `PipelineConfig` struct:
    - `script_path: String`
    - `dry_run: bool`
    - `namespace: String`
    - `chain_id: u64`
    - `script_sig: Option<String>` (function signature, default `"run()"`)
    - `script_args: Vec<String>`
    - `env_vars: HashMap<String, String>`
  - `PipelineContext` struct (populated during execution):
    - `config: PipelineConfig`
    - `script_path: String` (resolved absolute path)
    - `git_commit: String` (current HEAD sha, or empty)
    - `project_root: PathBuf`
  - `PipelineResult` struct:
    - `deployments: Vec<RecordedDeployment>` (what was recorded or would be)
    - `transactions: Vec<RecordedTransaction>`
    - `collisions: Vec<ExtractedCollision>`
    - `skipped: Vec<SkippedDeployment>` (duplicates that were skipped)
    - `dry_run: bool`
    - `success: bool`
    - `console_logs: Vec<String>`
  - `RecordedDeployment` struct:
    - `deployment: Deployment` (the core domain type)
    - `is_new: bool` (true if inserted, false if updated)
  - `RecordedTransaction` struct:
    - `transaction: Transaction`
    - `is_new: bool`
  - `SkippedDeployment` struct:
    - `deployment: Deployment` (what would have been recorded)
    - `reason: String` (e.g., "duplicate address", "duplicate ID")
    - `existing_id: String` (ID of the existing deployment)
- Register `pub mod pipeline;` in `crates/treb-forge/src/lib.rs`
- Add re-exports in `lib.rs` for `PipelineConfig`, `PipelineContext`,
  `PipelineResult`, `RecordedDeployment`, `RecordedTransaction`,
  `SkippedDeployment`
- Implement `PipelineContext::resolve_git_commit()` — runs `git rev-parse HEAD`
  and returns the short hash (or empty string if not in a git repo)

**Acceptance Criteria:**
- [ ] `treb-registry` and `treb-config` added to `treb-forge/Cargo.toml`
- [ ] `PipelineConfig` struct defined with all fields
- [ ] `PipelineContext` struct defined with config + resolved context
- [ ] `PipelineResult` struct defined with deployments, transactions, collisions,
      skipped, dry_run flag, console_logs
- [ ] `RecordedDeployment`, `RecordedTransaction`, `SkippedDeployment` helper
      structs defined
- [ ] `resolve_git_commit()` returns a short hash or empty string
- [ ] Unit test: `PipelineConfig` default construction
- [ ] Unit test: `resolve_git_commit()` returns non-empty string in this repo
- [ ] `cargo check --workspace` passes
- [ ] `cargo clippy --workspace` passes

---

### US-002: Deployment Hydration — ExtractedDeployment to Core Deployment

**Description:** Implement the conversion layer that transforms forge's
`ExtractedDeployment` + `ProxyRelationship` into the core `Deployment` struct
that the registry stores. This is the central type bridge between the alloy-typed
forge domain and the string-serialized registry domain.

The converter must:
- Generate deployment IDs in the format `{namespace}/{chain_id}/{contract_name}:{label}`
- Convert `Address`, `B256`, `Bytes` to checksummed/hex `String` values
- Build `DeploymentStrategy` from extracted salt, init_code_hash, constructor_args,
  entropy, and strategy method
- Build `ArtifactInfo` from `ArtifactMatch` (when available) plus pipeline context
  (script_path, git_commit)
- Build `ProxyInfo` from `ProxyRelationship` when the deployment is a proxy
- Infer `DeploymentType`: `Proxy` if proxy relationship detected, `Singleton` otherwise
- Default `VerificationInfo` to `Unverified`

**Changes:**
- Create `crates/treb-forge/src/pipeline/hydrate.rs` with:
  - `hydrate_deployment(extracted: &ExtractedDeployment, ctx: &PipelineContext, proxy: Option<&ProxyRelationship>) -> Deployment`:
    - Generates `id` as `{ctx.config.namespace}/{ctx.config.chain_id}/{extracted.contract_name}:{extracted.label}`
    - Converts `extracted.address` to checksummed hex string via `Address::to_checksum(None)`
    - Sets `deployment_type` to `Proxy` if `proxy` is `Some`, else `Singleton`
    - Converts `extracted.transaction_id` (B256) to `format!("{:#x}", tx_id)` for
      the `transaction_id` field
    - Builds `DeploymentStrategy`:
      - `method`: directly from `extracted.strategy` (same `DeploymentMethod` enum)
      - `salt`: `format!("{:#x}", extracted.salt)` if non-zero, else empty string
      - `init_code_hash`: `format!("{:#x}", extracted.init_code_hash)` if non-zero, else empty
      - `factory`: infer CreateX address for CREATE2/CREATE3, empty for CREATE
      - `constructor_args`: hex-encode `extracted.constructor_args` if non-empty
      - `entropy`: direct copy from `extracted.entropy`
    - Builds `ArtifactInfo`:
      - `path`: from `ArtifactMatch.artifact_id.source` if available, else empty
      - `compiler_version`: from `ArtifactMatch.artifact_id.version` if available
      - `bytecode_hash`: `format!("{:#x}", extracted.bytecode_hash)`
      - `script_path`: from `ctx.script_path`
      - `git_commit`: from `ctx.git_commit`
    - Builds `ProxyInfo` from `ProxyRelationship` if present:
      - `proxy_type`: map `ProxyType` enum to string
        (`Transparent` → `"TransparentUpgradeableProxy"`, `UUPS` → `"UUPSProxy"`,
        `Beacon` → `"BeaconProxy"`, `Minimal` → `"MinimalProxy"`)
      - `implementation`: checksummed hex of `proxy.implementation`
      - `admin`: checksummed hex of `proxy.admin` if present, else empty
      - `history`: empty vec (initial deployment has no upgrade history)
    - Sets `verification` to default `Unverified` with empty fields
    - Sets `tags` to `None`
    - Sets `created_at` and `updated_at` to `Utc::now()`
  - `generate_deployment_id(namespace: &str, chain_id: u64, contract_name: &str, label: &str) -> String`
  - `address_to_string(addr: &Address) -> String` — checksummed hex
  - `b256_to_string(val: &B256) -> String` — 0x-prefixed hex, empty if zero
  - `bytes_to_hex(val: &Bytes) -> String` — 0x-prefixed hex, empty if empty
  - `proxy_type_to_string(pt: &ProxyType) -> String` — enum to display string

**Acceptance Criteria:**
- [ ] `hydrate_deployment` produces a valid `Deployment` from `ExtractedDeployment`
- [ ] Deployment ID follows `{namespace}/{chainId}/{contractName}:{label}` format
- [ ] `Address` values are checksummed hex strings (matching `alloy::primitives::Address::to_checksum`)
- [ ] `B256` values are `0x`-prefixed lowercase hex, empty string when all zeros
- [ ] `Bytes` constructor args are hex-encoded, empty string when empty
- [ ] `DeploymentStrategy` fields populated correctly for CREATE, CREATE2, CREATE3
- [ ] `ArtifactInfo` populated from `ArtifactMatch` when available
- [ ] `ArtifactInfo` has empty path/compiler_version when `ArtifactMatch` is `None`
- [ ] `ProxyInfo` correctly built from `ProxyRelationship` for all 4 proxy types
- [ ] `ProxyInfo` is `None` when no proxy relationship provided
- [ ] `DeploymentType` is `Proxy` when proxy relationship present, `Singleton` otherwise
- [ ] `VerificationInfo` defaults to `Unverified` with all fields empty
- [ ] Unit test: hydrate a CREATE deployment, verify all fields
- [ ] Unit test: hydrate a CREATE2 deployment with salt, verify strategy fields
- [ ] Unit test: hydrate a proxy deployment with Transparent proxy type
- [ ] Unit test: hydrate a deployment without ArtifactMatch, verify empty artifact fields
- [ ] Unit test: `generate_deployment_id` produces correct format
- [ ] `cargo check --workspace` passes
- [ ] `cargo clippy --workspace` passes

---

### US-003: Transaction Hydration — Events to Core Transaction Records

**Description:** Convert `TransactionSimulated` events from the decoded event
stream into core `Transaction` records suitable for registry insertion. Each
`TransactionSimulated` event contains one or more `SimulatedTransaction` entries
with sender, calldata, and a transaction ID. The hydrator links deployments to
transactions by matching the `transaction_id` field from `ExtractedDeployment`
to the `transactionId` in `SimulatedTransaction`.

Also handle `SafeTransactionQueued` events by creating stub `SafeTransaction`
records (full Safe integration is Phase 17, but the recording pipeline should
capture the event data now).

**Changes:**
- Create `crates/treb-forge/src/pipeline/transactions.rs` with:
  - `hydrate_transactions(events: &[ParsedEvent], deployments: &[Deployment], ctx: &PipelineContext) -> Vec<Transaction>`:
    - Finds all `TrebEvent::TransactionSimulated` events
    - For each `SimulatedTransaction` in the event:
      - Creates a `Transaction` with:
        - `id`: hex string of `transactionId`
        - `chain_id`: from `ctx.config.chain_id`
        - `hash`: empty string (not yet broadcast)
        - `status`: `TransactionStatus::Simulated`
        - `block_number`: 0
        - `sender`: checksummed hex of `SimulatedTransaction.sender`
        - `nonce`: 0 (not yet known)
        - `deployments`: vec of deployment IDs whose `transaction_id` matches
          this transaction's ID
        - `operations`: build `Operation` entries from the transaction data
          (type=`"DEPLOY"` for deployment txs, target/method from calldata)
        - `safe_context`: `None` (populated by Safe hydrator if applicable)
        - `environment`: `ctx.config.namespace`
        - `created_at`: `Utc::now()`
  - `hydrate_safe_transactions(events: &[ParsedEvent], ctx: &PipelineContext) -> Vec<SafeTransaction>`:
    - Finds all `TrebEvent::SafeTransactionQueued` events
    - Creates a `SafeTransaction` with:
      - `safe_tx_hash`: hex string of `safeTxHash`
      - `safe_address`: checksummed hex of `safe`
      - `chain_id`: from `ctx.config.chain_id`
      - `status`: `TransactionStatus::Queued`
      - `nonce`: 0 (to be filled by sync)
      - `transactions`: empty (to be filled by sync)
      - `transaction_ids`: hex strings of `transactionIds`
      - `proposed_by`: checksummed hex of `proposer`
      - `proposed_at`: `Utc::now()`
      - `confirmations`: empty
      - `executed_at`: `None`
      - `execution_tx_hash`: empty
  - `link_deployments_to_transaction(tx_id: &str, deployments: &[Deployment]) -> Vec<String>`:
    - Returns deployment IDs whose `transaction_id` matches `tx_id`

**Acceptance Criteria:**
- [ ] `hydrate_transactions` creates one `Transaction` per `SimulatedTransaction`
- [ ] Transaction `id` is the hex string of `transactionId` from the event
- [ ] Transaction `sender` is checksummed hex of the event's sender address
- [ ] Transaction `status` is `Simulated` for all newly created transactions
- [ ] Transaction `deployments` vec contains IDs of matching deployments
- [ ] Transaction `environment` matches the pipeline namespace
- [ ] `hydrate_safe_transactions` creates `SafeTransaction` from queued events
- [ ] SafeTransaction `transaction_ids` contains hex strings of all linked tx IDs
- [ ] SafeTransaction `status` is `Queued`
- [ ] Unit test: hydrate a single TransactionSimulated with 2 deployments, verify linking
- [ ] Unit test: hydrate a SafeTransactionQueued event, verify all fields
- [ ] Unit test: empty events produce empty results (no panic)
- [ ] `cargo check --workspace` passes
- [ ] `cargo clippy --workspace` passes

---

### US-004: Duplicate Deployment Detection and Conflict Resolution

**Description:** Before recording deployments to the registry, check for
duplicates. A deployment can conflict in two ways: (1) same deployment ID
(`namespace/chainId/contractName:label`), meaning the same logical deployment
slot is occupied; or (2) same address on the same chain, meaning a contract
already exists at that address. The resolution strategy determines whether to
error, skip, or update.

**Changes:**
- Create `crates/treb-forge/src/pipeline/duplicates.rs` with:
  - `DuplicateStrategy` enum: `Error`, `Skip`, `Update`
  - `DuplicateCheck` struct:
    - `deployment: Deployment` (the candidate)
    - `conflict: Option<DuplicateConflict>`
  - `DuplicateConflict` struct:
    - `existing_id: String`
    - `conflict_type: ConflictType`
  - `ConflictType` enum: `SameId`, `SameAddress`
  - `check_duplicate(deployment: &Deployment, registry: &Registry) -> Option<DuplicateConflict>`:
    - First check: does `registry.get_deployment(&deployment.id)` return `Some`?
      → `ConflictType::SameId`
    - Second check: iterate `registry.list_deployments()` looking for matching
      `address` + `chain_id` → `ConflictType::SameAddress`
  - `resolve_duplicates(candidates: Vec<Deployment>, registry: &Registry, strategy: DuplicateStrategy) -> (Vec<Deployment>, Vec<SkippedDeployment>)`:
    - For each candidate, call `check_duplicate`
    - If no conflict: add to insert list
    - If conflict + `Error`: return error
    - If conflict + `Skip`: add to skipped list with reason
    - If conflict + `Update`: add to update list (will call `registry.update_deployment`)
    - Returns (to_record, skipped) tuple
  - Default strategy: `Skip` (matches Go behavior — warn and skip duplicates)

**Acceptance Criteria:**
- [ ] `check_duplicate` detects existing deployment by ID
- [ ] `check_duplicate` detects existing deployment by address + chain_id
- [ ] `SameId` conflict takes precedence over `SameAddress`
- [ ] `resolve_duplicates` with `Skip` strategy returns skipped entries with reason
- [ ] `resolve_duplicates` with `Error` strategy returns error on first conflict
- [ ] `resolve_duplicates` with `Update` strategy marks entries for update
- [ ] No-conflict deployments pass through unchanged
- [ ] Unit test: no duplicates — all candidates pass through
- [ ] Unit test: duplicate by ID — correctly detected and skipped
- [ ] Unit test: duplicate by address+chain — correctly detected
- [ ] Unit test: Error strategy returns error
- [ ] `cargo check --workspace` passes
- [ ] `cargo clippy --workspace` passes

---

### US-005: RunPipeline Orchestrator with Dry-Run Mode

**Description:** Implement the `RunPipeline` struct that orchestrates the full
deployment recording flow: compile → build artifact index → execute script →
decode events → extract deployments → detect proxies → hydrate domain types →
check duplicates → record to registry. Supports dry-run mode where everything
executes but nothing is written to the registry.

The pipeline calls into existing APIs from previous phases:
- `compile_project()` / `compile_files()` from Phase 5
- `ArtifactIndex::from_compile_output()` from Phase 5
- `build_script_config()` / `execute_script()` from Phase 5
- `decode_events()` from Phase 7
- `extract_deployments()` / `extract_collisions()` from Phase 7
- `detect_proxy_relationships()` / `link_proxy_to_deployment()` from Phase 7
- `hydrate_deployment()` from US-002
- `hydrate_transactions()` / `hydrate_safe_transactions()` from US-003
- `resolve_duplicates()` from US-004
- `Registry::insert_deployment()` / `insert_transaction()` / `insert_safe_transaction()` from Phase 4

**Changes:**
- Create `crates/treb-forge/src/pipeline/run.rs` with:
  - `RunPipeline` struct (stateless — all state flows through method args):
    - `pub fn execute(config: PipelineConfig, resolved_config: &ResolvedConfig, registry: &mut Registry) -> Result<PipelineResult>`:
      1. Build `PipelineContext` from config + resolved_config (resolve git commit,
         project root)
      2. **Compile**: call `compile_project()` with foundry `Config::load()`
      3. **Build artifacts**: `ArtifactIndex::from_compile_output()`
      4. **Build script config**: `build_script_config(resolved_config, &config.script_path)`
      5. **Execute**: `execute_script(script_args)` → `ExecutionResult`
      6. Check `execution_result.success` — if false, return error with console logs
      7. **Decode events**: `decode_events(&execution_result.raw_logs)`
      8. **Extract deployments**: `extract_deployments(&events, Some(&artifacts))`
      9. **Extract collisions**: `extract_collisions(&events)`
      10. **Detect proxies**: `detect_proxy_relationships(&events)`
      11. **Link proxies**: for each proxy, `link_proxy_to_deployment()` to find
          matching deployment
      12. **Hydrate deployments**: call `hydrate_deployment()` for each extracted
          deployment, passing proxy relationship if linked
      13. **Hydrate transactions**: `hydrate_transactions(&events, &deployments, &ctx)`
      14. **Hydrate safe transactions**: `hydrate_safe_transactions(&events, &ctx)`
      15. **Check duplicates**: `resolve_duplicates(deployments, registry, strategy)`
      16. **Record** (unless dry_run):
          - `registry.insert_deployment()` for new deployments
          - `registry.update_deployment()` for updates
          - `registry.insert_transaction()` for transactions
          - `registry.insert_safe_transaction()` for safe transactions
      17. Return `PipelineResult` with all recorded/skipped data + console logs
    - `pub fn dry_run(config: PipelineConfig, resolved_config: &ResolvedConfig, registry: &Registry) -> Result<PipelineResult>`:
      - Convenience method that sets `config.dry_run = true` and calls `execute`
        with a mutable clone of the registry (or simply skips the write step)
- Update `pipeline/mod.rs` re-exports to include `RunPipeline`
- Update `crates/treb-forge/src/lib.rs` re-exports to include `RunPipeline`

**Acceptance Criteria:**
- [ ] `RunPipeline::execute` compiles the project, executes the script, and
      records deployments to the registry
- [ ] Failed script execution (success=false) returns an error with console log
      output
- [ ] Events are decoded and deployments are extracted from logs
- [ ] Proxy relationships are detected and linked to deployments
- [ ] Deployments are hydrated into core `Deployment` types
- [ ] Transactions are hydrated and linked to their deployments
- [ ] Safe transactions are recorded when SafeTransactionQueued events are present
- [ ] Duplicate detection runs before recording
- [ ] Dry-run mode: `PipelineResult` is populated but registry is unchanged
- [ ] `PipelineResult.console_logs` contains decoded console.log output
- [ ] `PipelineResult.collisions` contains any `DeploymentCollision` events
- [ ] `PipelineResult.success` is true when all steps complete
- [ ] `cargo check --workspace` passes
- [ ] `cargo clippy --workspace` passes

---

### US-006: Pipeline Integration Tests

**Description:** Write integration tests that exercise the full pipeline using
synthetic execution data. Since running real forge compilation+execution requires
a full Solidity project and foundry toolchain, these tests mock/construct the
intermediate data (parsed events, extracted deployments) and verify the hydration
→ duplicate check → registry write flow end-to-end. Also include a test that
verifies the pipeline's behavior with dry-run mode and with duplicate deployments.

**Changes:**
- Create `crates/treb-forge/tests/pipeline.rs` — integration test file
- Create `crates/treb-forge/tests/fixtures/pipeline/` directory for test data
- Test helpers:
  - `make_extracted_deployment(name, label, strategy)` — construct an
    `ExtractedDeployment` with known values
  - `make_proxy_relationship(proxy_addr, impl_addr, proxy_type)` — construct a
    `ProxyRelationship`
  - `make_pipeline_context(namespace, chain_id)` — construct a `PipelineContext`
  - `make_test_registry(dir)` — initialize an empty registry in a temp directory
- Integration tests:
  - **Test: single deployment hydration and recording** — create one
    `ExtractedDeployment`, hydrate it, insert into registry, verify the
    `Deployment` was persisted with correct field values (ID format, checksummed
    address, strategy fields, artifact info)
  - **Test: proxy deployment recording** — create an `ExtractedDeployment` +
    `ProxyRelationship`, hydrate, insert, verify `deployment_type` is `Proxy`
    and `proxy_info` is correctly populated
  - **Test: transaction linking** — create `TransactionSimulated` events and
    matching deployments, hydrate transactions, verify `deployments` vec contains
    correct deployment IDs
  - **Test: duplicate detection — skip strategy** — insert a deployment into
    registry, then attempt to record a deployment with the same ID, verify it
    is skipped and appears in `PipelineResult.skipped`
  - **Test: duplicate detection — same address** — insert a deployment, then
    attempt to record a different deployment at the same address+chain, verify
    conflict is detected
  - **Test: dry-run mode** — run the hydration + duplicate check + recording
    steps with `dry_run=true`, verify `PipelineResult` is populated but registry
    remains empty
  - **Test: multiple deployments in single pipeline run** — hydrate 3
    deployments (one CREATE, one CREATE2, one proxy), record all, verify registry
    contains all 3 with correct types and strategies
  - **Test: collision reporting** — create `ExtractedCollision` data, verify it
    appears in `PipelineResult.collisions`

**Acceptance Criteria:**
- [ ] At least 8 integration tests covering the scenarios listed above
- [ ] Single deployment hydration + recording test passes with correct field values
- [ ] Proxy deployment test verifies `deployment_type`, `proxy_info.proxy_type`,
      `proxy_info.implementation`
- [ ] Transaction linking test verifies deployment IDs are correctly linked
- [ ] Duplicate skip test verifies skipped entries in result
- [ ] Duplicate address detection test correctly identifies conflict
- [ ] Dry-run test verifies registry is unchanged after pipeline execution
- [ ] Multi-deployment test verifies all 3 deployments recorded with correct types
- [ ] Collision test verifies collisions are reported in result
- [ ] All existing tests continue to pass
- [ ] `cargo test --workspace` passes
- [ ] `cargo clippy --workspace` passes

---

## Functional Requirements

- **FR-1:** Deployment IDs follow the format
  `{namespace}/{chainId}/{contractName}:{label}`, matching the Go version's ID
  scheme. The `DeploymentId` newtype from `treb-core` is used for type safety.

- **FR-2:** All `Address` values are serialized as checksummed hex strings using
  `alloy::primitives::Address::to_checksum(None)`. All `B256` values are
  serialized as `0x`-prefixed lowercase hex strings. Zero values become empty
  strings (matching the Go version's omitempty behavior).

- **FR-3:** The pipeline is transactional at the deployment level — if recording
  a single deployment fails, previously recorded deployments in the same run are
  not rolled back (the registry is append-only per the Go version's behavior).

- **FR-4:** Duplicate detection checks by deployment ID first, then by
  address+chain_id. The default resolution strategy is `Skip` (log a warning
  and continue), matching the Go version's behavior.

- **FR-5:** Dry-run mode produces a complete `PipelineResult` with all hydrated
  data but makes zero mutations to the registry. The result is indistinguishable
  from a real run except for `dry_run: true`.

- **FR-6:** Console.log output from script execution is captured in
  `PipelineResult.console_logs` for display by the CLI layer (Phase 12).

- **FR-7:** `DeploymentCollision` events from the script are captured and
  included in `PipelineResult.collisions` as warnings — they do not abort the
  pipeline.

- **FR-8:** `ProxyType` enum values map to human-readable strings:
  `Transparent` → `"TransparentUpgradeableProxy"`, `UUPS` → `"UUPSProxy"`,
  `Beacon` → `"BeaconProxy"`, `Minimal` → `"MinimalProxy"`.

---

## Non-Goals

- **No broadcast/on-chain submission** — The pipeline stops after EVM
  execution and event parsing. Broadcasting transactions to the network
  (progressing past `ExecutedState` to `BroadcastedState`) is Phase 12's
  responsibility.

- **No CLI integration** — Printing pipeline results, progress indicators,
  deployment summaries, and interactive confirmation prompts are Phase 12
  (`treb run` command) concerns.

- **No trace-based strategy detection** — Walking `ScriptResult.traces` to
  detect CREATE vs CREATE2 opcodes at the EVM level is deferred. This phase
  relies on the `createStrategy` field in `DeploymentDetails` events, which
  the treb-sol Solidity library sets explicitly.

- **No verification triggering** — Post-recording contract verification is
  Phase 13's responsibility.

- **No Safe/Governor execution** — While this phase records
  `SafeTransactionQueued` events as stub `SafeTransaction` entries, the actual
  Safe Transaction Service integration (proposing, confirming, executing) is
  Phase 17.

- **No compose orchestration** — Multi-script compose workflows that run
  the pipeline multiple times with dependency ordering are Phase 16.

---

## Technical Considerations

### Dependency Changes to treb-forge

`treb-forge` currently depends on `treb-core`, `foundry-config`,
`foundry-common`, and the forge/alloy crates. This phase adds:

| Dependency | Purpose |
|---|---|
| `treb-registry` | Write deployments and transactions to the registry |
| `treb-config` | Access `ResolvedConfig` for namespace, network, project root |

This creates a dependency: `treb-forge` → `treb-registry` → `treb-core`. Since
`treb-forge` already depends on `treb-core`, and `treb-registry` also depends on
`treb-core`, this is a diamond dependency — acceptable in Rust because Cargo
unifies versions.

### Type Conversion: alloy → String

The central challenge of this phase. The forge layer uses alloy's strongly-typed
primitives (`Address`, `B256`, `Bytes`, `U256`), while the core domain types use
plain `String` for all hex values (matching the Go version's JSON schema). The
conversion functions in `hydrate.rs` must handle:

| Source Type | Target | Conversion |
|---|---|---|
| `Address` | `String` | `addr.to_checksum(None)` |
| `B256` | `String` | `format!("{:#x}", val)`, empty if zero |
| `Bytes` | `String` | `format!("0x{}", hex::encode(&val))`, empty if empty |
| `DeploymentMethod` | `DeploymentMethod` | Same enum, direct use |
| `ProxyType` | `String` | Map enum to display string |

### Pipeline State Flow

```
PipelineConfig + ResolvedConfig
        │
        ▼
   PipelineContext (resolved paths, git commit)
        │
        ▼
  CompilationOutput ──► ArtifactIndex
        │
        ▼
  ExecutionResult (raw_logs, success, transactions, traces)
        │
        ▼
  Vec<ParsedEvent> (decoded TrebEvents, CreateX, Proxy)
        │
        ├──► Vec<ExtractedDeployment>
        ├──► Vec<ExtractedCollision>
        ├──► HashMap<Address, ProxyRelationship>
        │
        ▼
  Vec<Deployment> (hydrated, with proxy info merged)
  Vec<Transaction> (hydrated, with deployment links)
  Vec<SafeTransaction> (hydrated stubs)
        │
        ▼
  Duplicate Check (skip/error/update)
        │
        ▼
  Registry Write (unless dry_run)
        │
        ▼
  PipelineResult
```

### Registry Interaction

The `Registry` facade from Phase 4 handles all persistence:
- `insert_deployment()` automatically rebuilds the lookup index
- `insert_transaction()` and `insert_safe_transaction()` persist to their
  respective JSON files
- All writes are atomic (write-to-temp then rename) via the `io` module

The pipeline should hold a `&mut Registry` for the duration of the run and
call insert methods sequentially. No batch insert API is needed — the registry
handles one record at a time.

### Module Layout

```
crates/treb-forge/src/pipeline/
├── mod.rs            (module declarations, re-exports)
├── context.rs        (PipelineConfig, PipelineContext, PipelineResult)
├── hydrate.rs        (ExtractedDeployment → Deployment conversion)
├── transactions.rs   (event → Transaction/SafeTransaction conversion)
├── duplicates.rs     (duplicate detection and conflict resolution)
└── run.rs            (RunPipeline orchestrator)
```

### Testing Strategy

Full end-to-end pipeline tests requiring real forge compilation are expensive
and depend on the Solidity toolchain being installed. The integration tests in
US-006 test the hydration → duplicate → registry path using hand-constructed
intermediate types (`ExtractedDeployment`, `ProxyRelationship`, parsed events).
This exercises all Phase 8 code without requiring compilation or script execution.

True end-to-end tests with real Solidity compilation will be added in Phase 12
when the `treb run` command is implemented.
