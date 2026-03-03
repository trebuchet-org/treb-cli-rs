# PRD: Phase 7 — Event Parsing and ABI Bindings

## Introduction

Phase 7 adds the event parsing layer that converts raw EVM logs from forge script
execution into structured treb domain events. When a treb deployment script runs,
it emits events via the `ITrebEvents` and `ICreateX` Solidity interfaces — events
like `ContractDeployed`, `SafeTransactionQueued`, `DeploymentCollision`, and
`ContractCreation`. This phase defines these event ABIs using alloy's `sol!` macro,
decodes them from `ExecutionResult.raw_logs`, and extracts deployment data, proxy
relationships, labels, and tags.

This phase also adds a natspec annotation parser for `@custom:env` tags in script
source files, enabling treb to discover what parameters a deployment script expects
(e.g., `@custom:env {address} owner Owner address`).

Phase 5 (`treb-forge`) provides the `ExecutionResult` with `raw_logs: Vec<Log>` —
the raw EVM events emitted during script execution. This phase consumes those logs
and produces structured, typed event data that Phase 8 (Deployment Recording
Pipeline) will use to create registry entries. The event types and proxy
relationship logic implemented here mirror the Go version's
`internal/domain/bindings/` and `internal/adapters/abi/event_parser.go`.

---

## Goals

1. **Type-safe event ABIs** — Define `ITrebEvents` and `ICreateX` event
   signatures using alloy's `sol!` macro so event decoding is compile-time
   verified against the Solidity interface.

2. **Structured event extraction** — Decode raw EVM logs from `ExecutionResult`
   into typed Rust enums (`TrebEvent`, `CreateXEvent`, `ProxyEvent`) with all
   indexed and non-indexed parameters accessible as native Rust types.

3. **Proxy relationship detection** — Detect ERC-1967 proxy patterns
   (`Upgraded`, `AdminChanged`, `BeaconUpgraded` events) from execution logs
   and link proxy addresses to their implementation addresses.

4. **Natspec parameter discovery** — Parse `@custom:env` annotations from
   compiled script artifacts to discover script parameters, types, and
   optionality.

5. **Library detection support** — Integrate `foundry-linking::Linker` to
   detect linked libraries during the compilation/linking phase, enabling
   downstream library deployment tracking.

---

## User Stories

### US-001: ABI Definitions via alloy sol! Macro

**Description:** Define the `ITrebEvents` and `ICreateX` Solidity interfaces in
Rust using alloy's `sol!` macro. This generates type-safe Rust structs for every
event, struct, and type defined in these interfaces. The generated types provide
`decode_log` methods for parsing raw EVM logs. Also define the ERC-1967 proxy
events (`Upgraded`, `AdminChanged`, `BeaconUpgraded`) which are standard OpenZeppelin
events not part of ITrebEvents but required for proxy relationship detection.

All new code goes into `crates/treb-forge/src/events/` as a new module under the
existing `treb-forge` crate. No new workspace crates are added.

**Changes:**
- Add `alloy-sol-types` as a workspace dependency in root `Cargo.toml` (version
  aligned with foundry v1.5.1's alloy). Add it to `treb-forge`'s `Cargo.toml`.
- Create `crates/treb-forge/src/events/mod.rs` — module declarations and
  re-exports
- Create `crates/treb-forge/src/events/abi.rs` containing three `sol!` blocks:
  - `ITrebEvents` — matching the Solidity interface exactly:
    - `DeploymentDetails` struct (artifact, label, entropy, salt, bytecodeHash,
      initCodeHash, constructorArgs, createStrategy)
    - `SimulatedTransaction` struct (transactionId, senderId, sender, returnData,
      transaction) with nested `Transaction` struct (to, data, value)
    - Events: `TransactionSimulated`, `ContractDeployed` (3 indexed),
      `SafeTransactionQueued` (3 indexed), `SafeTransactionExecuted` (3 indexed),
      `DeploymentCollision` (1 indexed), `GovernorProposalCreated` (3 indexed)
  - `ICreateX` — three events:
    - `ContractCreation(address indexed newContract, bytes32 indexed salt)`
    - `ContractCreation(address indexed newContract)` (overloaded, no salt)
    - `Create3ProxyContractCreation(address indexed newContract, bytes32 indexed salt)`
  - `ProxyEvents` — ERC-1967 standard events:
    - `Upgraded(address indexed implementation)`
    - `AdminChanged(address previousAdmin, address newAdmin)`
    - `BeaconUpgraded(address indexed beacon)`
- Register `pub mod events;` in `crates/treb-forge/src/lib.rs`
- Add re-exports in `lib.rs` for the key public types

**Acceptance Criteria:**
- [ ] `alloy-sol-types` added to workspace deps and `treb-forge/Cargo.toml`
- [ ] `sol!` macro compiles and generates types for all ITrebEvents events
- [ ] `sol!` macro compiles and generates types for all ICreateX events
- [ ] `sol!` macro compiles and generates types for ERC-1967 proxy events
- [ ] Generated structs have correct field types (e.g., `Address` for addresses,
      `B256` for bytes32, `Vec<B256>` for `bytes32[]`)
- [ ] `DeploymentDetails` struct is generated with all 8 fields matching the
      Solidity definition
- [ ] Unit test: construct each event type manually and verify field access
- [ ] `cargo check --workspace` passes
- [ ] `cargo clippy --workspace` passes

---

### US-002: Event Log Decoder

**Description:** Implement the core event decoding engine that takes a slice of
raw EVM `Log` entries (from `ExecutionResult.raw_logs`) and attempts to decode
each one against the known event signatures (ITrebEvents, ICreateX, proxy events).
Returns a list of decoded, typed events in emission order.

The decoder uses topic[0] (the event signature hash) to identify which event type
a log corresponds to, then calls the appropriate alloy-generated `decode_log`
method. Unrecognized events are collected as `UnknownEvent` entries (not errors).

**Changes:**
- Create `crates/treb-forge/src/events/decoder.rs` with:
  - `TrebEvent` enum — variants for each ITrebEvents event type:
    - `TransactionSimulated { ... }`
    - `ContractDeployed { deployer, location, transaction_id, deployment: DeploymentDetails }`
    - `SafeTransactionQueued { safe_tx_hash, safe, proposer, transaction_ids }`
    - `SafeTransactionExecuted { safe_tx_hash, safe, executor, transaction_ids }`
    - `DeploymentCollision { existing_contract, deployment_details }`
    - `GovernorProposalCreated { proposal_id, governor, proposer, transaction_ids }`
  - `CreateXEvent` enum:
    - `ContractCreation { new_contract, salt: Option<B256> }`
    - `Create3ProxyContractCreation { new_contract, salt }`
  - `ProxyEvent` enum:
    - `Upgraded { proxy_address, implementation }`
    - `AdminChanged { proxy_address, previous_admin, new_admin }`
    - `BeaconUpgraded { proxy_address, beacon }`
  - `ParsedEvent` enum wrapping all three: `Treb(TrebEvent)`,
    `CreateX(CreateXEvent)`, `Proxy(ProxyEvent)`, `Unknown(UnknownLog)`
  - `UnknownLog` struct: `address`, `topics`, `data`
  - `decode_events(logs: &[Log]) -> Vec<ParsedEvent>`:
    - Iterates logs in order
    - Checks topic[0] against known event signature hashes
    - Calls the matching `decode_log` method from the sol!-generated types
    - For proxy events, attaches the emitting address as `proxy_address`
    - On decode failure, logs a warning and adds `UnknownLog`
    - Returns all decoded events in order
  - `extract_treb_events(events: &[ParsedEvent]) -> Vec<&TrebEvent>` — filter
    helper
  - `extract_proxy_events(events: &[ParsedEvent]) -> Vec<&ProxyEvent>` — filter
    helper

**Acceptance Criteria:**
- [ ] `decode_events` correctly decodes all ITrebEvents event types from raw logs
- [ ] `decode_events` correctly decodes all ICreateX event types from raw logs
- [ ] `decode_events` correctly decodes ERC-1967 proxy events with emitter as
      `proxy_address`
- [ ] Unrecognized logs produce `ParsedEvent::Unknown` (not errors)
- [ ] Indexed parameters (address, bytes32) are decoded from topics
- [ ] Non-indexed parameters are decoded from log data
- [ ] Event order matches the input log order
- [ ] Unit test: construct synthetic logs for `ContractDeployed` with known values,
      decode them, and verify all fields match
- [ ] Unit test: construct synthetic logs for `ContractCreation` (with and without
      salt), decode and verify
- [ ] Unit test: mixed logs (treb + proxy + unknown) are decoded in order
- [ ] `cargo check --workspace` passes
- [ ] `cargo clippy --workspace` passes

---

### US-003: Deployment Extraction from Events

**Description:** Implement the logic that processes decoded events to extract
deployment information: contract address, deployer, artifact name, label,
deployment strategy (CREATE/CREATE2/CREATE3), salt, constructor args, and
collision detection. This builds the intermediate deployment data that Phase 8
will use to create registry `Deployment` entries.

Also integrates with `ArtifactIndex` (`ContractsByArtifact`) to match deployment
artifacts to compiled contract data (ABI, bytecode hash).

**Changes:**
- Create `crates/treb-forge/src/events/deployments.rs` with:
  - `ExtractedDeployment` struct:
    - `address: Address`
    - `deployer: Address`
    - `transaction_id: B256`
    - `contract_name: String` — from `DeploymentDetails.artifact`
    - `label: String` — from `DeploymentDetails.label`
    - `strategy: DeploymentMethod` — parsed from `DeploymentDetails.createStrategy`
    - `salt: B256`
    - `bytecode_hash: B256`
    - `init_code_hash: B256`
    - `constructor_args: Bytes`
    - `entropy: String`
    - `artifact_match: Option<ArtifactMatch>` — from `ArtifactIndex` lookup
  - `DeploymentCollision` struct:
    - `existing_address: Address`
    - `deployment_details: ExtractedDeployment` (same fields, minus address/deployer/tx_id)
  - `extract_deployments(events: &[ParsedEvent], artifacts: Option<&ArtifactIndex>) -> Vec<ExtractedDeployment>`:
    - Filters for `TrebEvent::ContractDeployed` events
    - Maps `DeploymentDetails.createStrategy` string to `DeploymentMethod`
      enum: `"create"` → `Create`, `"create2"` → `Create2`,
      `"create3"` → `Create3`
    - If `artifacts` is provided, calls `find_by_name` on the artifact field
      to attach the `ArtifactMatch`
    - Returns deployments in event order
  - `extract_collisions(events: &[ParsedEvent]) -> Vec<DeploymentCollision>`:
    - Filters for `TrebEvent::DeploymentCollision` events
    - Returns collision data for caller to handle (warn or error)
  - `parse_deployment_strategy(s: &str) -> DeploymentMethod`:
    - Maps strategy string to enum, defaults to `Create` for unknown values

**Acceptance Criteria:**
- [ ] `extract_deployments` correctly extracts address, deployer, and transaction
      ID from `ContractDeployed` events
- [ ] `DeploymentDetails` fields (artifact, label, entropy, salt, bytecodeHash,
      initCodeHash, constructorArgs, createStrategy) are all mapped correctly
- [ ] Strategy string parsing: "create" → `Create`, "create2" → `Create2`,
      "create3" → `Create3`
- [ ] When `ArtifactIndex` is provided, `artifact_match` is populated for matching
      contract names
- [ ] When `ArtifactIndex` is `None`, `artifact_match` is `None` (no panic)
- [ ] `extract_collisions` returns collision data with the address and deployment
      details
- [ ] Unit test: create `ContractDeployed` event, extract deployment, verify all
      fields
- [ ] Unit test: create `DeploymentCollision` event, extract collision, verify
      fields
- [ ] Unit test: unknown strategy string defaults to `Create`
- [ ] `cargo check --workspace` passes
- [ ] `cargo clippy --workspace` passes

---

### US-004: Proxy Relationship Linking

**Description:** Implement logic to detect proxy relationships from ERC-1967
proxy events emitted during script execution. When an `Upgraded` event is
followed by an `AdminChanged` event from the same address, this indicates a
TransparentUpgradeableProxy. When only `Upgraded` is present, it's UUPS. When
`BeaconUpgraded` is present, it's a Beacon proxy.

The proxy linker correlates proxy events with deployment events to establish
which deployed contract is a proxy for which implementation.

**Changes:**
- Create `crates/treb-forge/src/events/proxy.rs` with:
  - `ProxyType` enum: `Transparent`, `UUPS`, `Beacon`, `Minimal`
  - `ProxyRelationship` struct:
    - `proxy_address: Address`
    - `implementation_address: Address`
    - `admin_address: Option<Address>`
    - `beacon_address: Option<Address>`
    - `proxy_type: ProxyType`
  - `extract_proxy_relationships(events: &[ParsedEvent]) -> HashMap<Address, ProxyRelationship>`:
    - Iterates proxy events in order
    - For `Upgraded`: creates or updates entry with implementation address.
      Initial type is `UUPS` (overridden to `Transparent` if `AdminChanged`
      follows)
    - For `AdminChanged`: creates or updates entry with admin address. Sets
      type to `Transparent` if was `Minimal`/`UUPS`
    - For `BeaconUpgraded`: creates or updates entry with beacon address. Sets
      type to `Beacon`
    - Returns map from proxy address to relationship
  - `link_proxy_to_deployment(proxy_address: Address, deployments: &[ExtractedDeployment]) -> Option<&ExtractedDeployment>`:
    - Finds the deployment whose address matches the proxy address
    - Returns it for the caller to mark as a proxy deployment

**Acceptance Criteria:**
- [ ] `Upgraded` event alone → `UUPS` proxy type
- [ ] `Upgraded` + `AdminChanged` from same proxy → `Transparent` proxy type
- [ ] `BeaconUpgraded` → `Beacon` proxy type
- [ ] Multiple proxies in the same execution are tracked independently
- [ ] `link_proxy_to_deployment` matches proxy address to a deployment
- [ ] Unit test: single UUPS proxy (Upgraded only)
- [ ] Unit test: TransparentUpgradeableProxy (Upgraded + AdminChanged)
- [ ] Unit test: Beacon proxy (BeaconUpgraded event)
- [ ] Unit test: multiple proxies in same execution
- [ ] `cargo check --workspace` passes
- [ ] `cargo clippy --workspace` passes

---

### US-005: Script Parameter / Natspec Parser

**Description:** Parse `@custom:env` natspec annotations from compiled script
artifacts to discover what parameters a deployment script expects. The parser reads
the `devdoc` metadata from the compiled artifact JSON and extracts parameter
definitions including type, name, description, and optionality.

This enables `treb run` (Phase 12) to validate that all required environment
variables / parameters are provided before executing a script.

Also integrates `foundry-linking::Linker` as a workspace dependency for library
detection during the linking phase (used by Phase 8's deployment recording
pipeline).

**Changes:**
- Add `foundry-linking` workspace dependency to root `Cargo.toml`:
  `foundry-linking = { git = "https://github.com/foundry-rs/foundry", tag = "v1.5.1" }`
- Add `foundry-linking` to `treb-forge`'s `Cargo.toml` dependencies
- Create `crates/treb-forge/src/events/params.rs` with:
  - `ParameterType` enum: `String`, `Address`, `Uint256`, `Int256`, `Bytes32`,
    `Bytes`, `Bool`, `Sender`, `Deployment`, `Artifact`
  - `ScriptParameter` struct:
    - `name: String`
    - `param_type: ParameterType`
    - `description: String`
    - `optional: bool`
  - `parse_script_parameters(devdoc: &serde_json::Value) -> Vec<ScriptParameter>`:
    - Looks for `methods["run()"]["custom:env"]` in the devdoc JSON
    - Parses the annotation string format:
      `{type[:optional]} name description text...`
    - Supports types: string, address, uint256, int256, bytes32, bytes, bool,
      sender, deployment, artifact
    - The `:optional` suffix on the type makes the parameter optional
    - Returns empty vec if no `run()` method or no `custom:env` annotations
  - `parse_custom_env_string(env_str: &str) -> Vec<ScriptParameter>`:
    - Splits on `{` to find type blocks
    - Extracts type (with optional `:optional` suffix), name, and description
    - Maps type strings to `ParameterType` enum
- Add `foundry-linking` re-export in `lib.rs` for downstream use:
  `pub use foundry_linking;`

**Acceptance Criteria:**
- [ ] `foundry-linking` added to workspace deps and `treb-forge/Cargo.toml`
- [ ] `parse_script_parameters` extracts parameters from devdoc JSON
- [ ] All 10 parameter types are supported (string, address, uint256, int256,
      bytes32, bytes, bool, sender, deployment, artifact)
- [ ] `:optional` suffix is parsed correctly
- [ ] Multi-parameter annotations are split correctly
- [ ] Missing devdoc or missing `run()` method returns empty vec (not error)
- [ ] Unit test: parse `{string} label Deployment label` → `ScriptParameter` with
      name="label", type=String, optional=false
- [ ] Unit test: parse `{address:optional} owner Optional owner` → optional=true
- [ ] Unit test: parse multi-param string with 3+ parameters
- [ ] Unit test: empty devdoc returns empty vec
- [ ] `cargo check --workspace` passes
- [ ] `cargo clippy --workspace` passes

---

### US-006: Integration Tests with Captured Log Fixtures

**Description:** Write integration tests that validate the full event parsing
pipeline against realistic log data. Create test fixtures containing captured EVM
logs from real deployment scenarios (manually constructed from known event
signatures and parameters). Tests should cover the complete flow: raw logs →
decode → extract deployments → detect proxies → link relationships.

**Changes:**
- Create `crates/treb-forge/tests/events.rs` — integration test file
- Create `crates/treb-forge/tests/fixtures/events/` directory with JSON fixture
  files containing raw log data:
  - `simple_deploy.json` — logs from a single `ContractDeployed` event
  - `create2_deploy.json` — logs from a CREATE2 deployment with
    `ContractCreation` (salt variant) + `ContractDeployed`
  - `proxy_deploy.json` — logs from a proxy deployment: `ContractDeployed`
    (impl) + `Upgraded` + `AdminChanged` + `ContractDeployed` (proxy)
  - `multi_deploy.json` — logs from multiple deployments in one script
  - `collision.json` — logs including a `DeploymentCollision` event
- Each fixture is a JSON array of log objects with `address`, `topics[]`, and
  `data` fields (hex-encoded), matching alloy's `Log` serialization format
- Integration tests:
  - Test: decode simple deploy fixture, verify one `ContractDeployed` event
    with correct address and label
  - Test: decode create2 fixture, verify `ContractCreation` event with salt
    and matching `ContractDeployed`
  - Test: decode proxy fixture, verify proxy relationship is detected as
    `Transparent` with correct implementation and admin addresses
  - Test: decode multi-deploy fixture, verify correct count and order of
    deployments
  - Test: decode collision fixture, verify `DeploymentCollision` is extracted
  - Test: full pipeline — decode → extract deployments → extract proxy
    relationships, verify cross-references are correct
- Update `events/mod.rs` re-exports to expose all public types needed by tests

**Acceptance Criteria:**
- [ ] At least 5 fixture files with realistic log data created
- [ ] Log fixture format matches alloy's `Log` JSON serialization
- [ ] Integration test: simple deploy round-trip succeeds
- [ ] Integration test: CREATE2 deploy with salt is correctly decoded
- [ ] Integration test: proxy relationship detection (Transparent) succeeds
- [ ] Integration test: multiple deployments in single script are all extracted
- [ ] Integration test: collision event is extracted with correct address
- [ ] Integration test: full pipeline (decode → deployments → proxies) works
      end-to-end
- [ ] All existing tests continue to pass
- [ ] `cargo test --workspace` passes
- [ ] `cargo clippy --workspace` passes

---

## Functional Requirements

- **FR-1:** Event ABI definitions use alloy's `sol!` macro exclusively. No
  hand-written ABI JSON or manual topic hash computation. The `sol!` macro
  provides compile-time verification against the Solidity interface.

- **FR-2:** Event decoding is exhaustive for known event types and gracefully
  handles unknown events. An unrecognized log is never an error — it becomes
  `ParsedEvent::Unknown`. Malformed logs that match a known signature but fail
  to decode are logged as warnings and skipped.

- **FR-3:** The `ITrebEvents` Solidity interface definition in Rust must exactly
  match the canonical Solidity source at
  `treb-sol/src/internal/ITrebEvents.sol`. Event signatures (topic[0] hashes)
  must match what the Solidity compiler produces.

- **FR-4:** Proxy relationship detection follows ERC-1967 conventions:
  `Upgraded` event = implementation set, `AdminChanged` = transparent proxy
  admin, `BeaconUpgraded` = beacon proxy. Proxy type inference
  (`Transparent` vs `UUPS` vs `Beacon`) uses the same logic as the Go version.

- **FR-5:** The natspec parser handles the exact `@custom:env` format used by
  treb-sol scripts. All 10 parameter types (string, address, uint256, int256,
  bytes32, bytes, bool, sender, deployment, artifact) and the `:optional`
  modifier are supported.

- **FR-6:** All new code lives within the existing `treb-forge` crate under a
  new `events` module. No new workspace crates are created in this phase.

- **FR-7:** `foundry-linking` is added as a workspace dependency and re-exported
  from `treb-forge` for use by Phase 8's deployment recording pipeline. This
  phase adds the dependency and re-export; actual `Linker` usage is Phase 8.

---

## Non-Goals

- **No deployment recording** — Converting `ExtractedDeployment` into registry
  `Deployment` entries and writing them to disk is Phase 8's responsibility.
  This phase extracts structured data from events but does not persist it.

- **No transaction management** — Processing `TransactionSimulated`,
  `SafeTransactionQueued`, `SafeTransactionExecuted`, and
  `GovernorProposalCreated` events into registry `Transaction` /
  `SafeTransaction` / `GovernorProposal` records is Phase 8. This phase decodes
  these events but the hydration pipeline that creates transaction records is
  deferred.

- **No broadcast enrichment** — Cross-referencing event data with broadcast file
  data (matching tx hashes, block numbers, gas used) is Phase 8's hydrator
  responsibility.

- **No trace analysis** — Walking execution traces to extract deployment
  strategies from trace node types (CREATE vs CREATE2 opcodes) belongs to
  Phase 8. This phase relies on the `createStrategy` field in
  `DeploymentDetails`.

- **No CLI integration** — Displaying event data, printing deployment summaries,
  or rendering proxy relationships is Phase 12 (`treb run`) work.

- **No CreateX factory detection** — Detecting whether a deployment used the
  CreateX factory contract (vs direct CREATE/CREATE2) from trace data is
  Phase 8. This phase decodes CreateX events when they're present in logs.

---

## Technical Considerations

### New Workspace Dependencies

| Crate | Source | Purpose |
|---|---|---|
| `alloy-sol-types` | crates.io (aligned with foundry v1.5.1) | `sol!` macro for event type generation |
| `foundry-linking` | `git = "...", tag = "v1.5.1"` | `Linker` for library detection (re-exported for Phase 8) |

### alloy sol! Macro Usage

The `sol!` macro generates Rust types from inline Solidity definitions. Each event
gets a struct with `SIGNATURE`, `SIGNATURE_HASH` constants and a
`decode_log(log, validate)` class method. Example:

```rust
sol! {
    event ContractDeployed(
        address indexed deployer,
        address indexed location,
        bytes32 indexed transactionId,
        DeploymentDetails deployment
    );
}
```

This generates `ContractDeployed` struct with `decode_log(&RawLog, bool) -> Result`.
The `SIGNATURE_HASH` is computed at compile time and can be compared against
`log.topics()[0]`.

### ICreateX Overloaded Events

ICreateX has two `ContractCreation` events with different signatures (one with
`salt`, one without). The `sol!` macro handles overloaded events by generating
separate types (e.g., `ContractCreation_0` and `ContractCreation_1`, or similar
disambiguated names). The decoder must handle both variants.

### Proxy Event Address Attribution

ERC-1967 proxy events (`Upgraded`, `AdminChanged`, `BeaconUpgraded`) are emitted
by the proxy contract itself. The `log.address` field tells us which contract
emitted the event — this is the proxy address. The decoder attaches this address
to the `ProxyEvent` so the proxy relationship builder can correlate events from
the same proxy.

### Module Layout

```
crates/treb-forge/src/events/
├── mod.rs           (module declarations, re-exports)
├── abi.rs           (sol! macro definitions for ITrebEvents, ICreateX, ProxyEvents)
├── decoder.rs       (ParsedEvent enum, decode_events(), filter helpers)
├── deployments.rs   (ExtractedDeployment, extract_deployments(), extract_collisions())
├── proxy.rs         (ProxyRelationship, ProxyType, extract_proxy_relationships())
└── params.rs        (ScriptParameter, ParameterType, parse_script_parameters())
```

### Test Fixture Construction

Integration test fixtures are JSON files with hand-constructed log data. To
create accurate fixtures, compute event signature hashes using the `sol!`-generated
`SIGNATURE_HASH` constants and ABI-encode parameters using alloy's encoding
utilities. A test helper should be provided to construct `Log` entries from
event types, making fixture creation straightforward.

### Compatibility with Go Version

The Go version uses `abigen`-generated bindings in
`internal/domain/bindings/treb.go` and `createx.go`. The Rust `sol!` macro serves
the same purpose. Event signature hashes must match between Go and Rust — this is
guaranteed because both derive from the same Solidity interface. The integration
tests should verify this by checking `SIGNATURE_HASH` values against known hashes.
