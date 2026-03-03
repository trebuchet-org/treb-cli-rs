# PRD: Phase 2 — Core Domain Types and Serialization

## Introduction

Phase 2 defines the central data models that every subsequent module depends on:
deployments, transactions, safe transactions, governor proposals, contracts, and
networks. These types live in `treb-core` and are the shared vocabulary of the
entire codebase.

**Critical constraint:** All types must serialize to the _exact same JSON format_
as the Go version of treb-cli. Users switching from Go → Rust must see zero
differences in `.treb/` registry files. This is validated through golden file
tests using fixtures extracted from the Go version's live data.

This phase builds directly on Phase 1's workspace, alloy primitive re-exports,
and error types. It does not add new workspace crates — all types belong in
`treb-core`.

---

## Goals

1. **Byte-identical JSON compatibility** — Every struct round-trips through
   `serde_json` to produce output identical to the Go version's JSON. Verified
   by golden file tests against real Go-generated fixtures.

2. **Complete domain model coverage** — All six model families are defined:
   `Deployment`, `Transaction`, `SafeTransaction`, `GovernorProposal`,
   `Contract`, and `Network`/`ChainId`, plus all nested sub-types and enums.

3. **Type-safe alloy integration** — Blockchain primitives (`Address`, `B256`,
   `TxHash`, `ChainId`) use alloy types from foundry's pinned versions. Custom
   newtypes wrap these where domain semantics differ from raw primitives.

4. **Ergonomic Rust API** — Types derive `Clone`, `Debug`, `PartialEq`,
   `Serialize`, `Deserialize`. Enums use string representations matching the Go
   constants. Optional fields map to `Option<T>` with `#[serde(skip_serializing_if)]`.

5. **Test coverage for every model** — Each model has at least one serde
   round-trip test using golden JSON fixtures, plus unit tests for enum
   conversions and display formatting.

---

## User Stories

### US-001: Foundation Types — Enums, Newtypes, and Module Structure

**Description:** Add serde/chrono/alloy-json-abi dependencies to the workspace
and `treb-core`. Create a `types` module hierarchy in `treb-core`. Define all
shared enums (`DeploymentType`, `DeploymentMethod`, `TransactionStatus`,
`VerificationStatus`, `ProposalStatus`) and domain newtypes (`DeploymentId`,
`TxHash`). Extend the existing `primitives` module with `ChainId`.

**Acceptance Criteria:**
- `serde`, `serde_json`, `chrono` (with `serde` feature), and `alloy-json-abi`
  are added to `[workspace.dependencies]` in root `Cargo.toml` and wired into
  `treb-core/Cargo.toml`
- `crates/treb-core/src/types/mod.rs` exists and is declared as `pub mod types`
  in `lib.rs`
- Enums defined with `#[derive(Serialize, Deserialize)]` and correct string
  representations:
  - `DeploymentType`: `SINGLETON`, `PROXY`, `LIBRARY`, `UNKNOWN`
  - `DeploymentMethod`: `CREATE`, `CREATE2`, `CREATE3`
  - `TransactionStatus`: `SIMULATED`, `QUEUED`, `EXECUTED`, `FAILED`
  - `VerificationStatus`: `UNVERIFIED`, `VERIFIED`, `FAILED`, `PARTIAL`
  - `ProposalStatus`: `pending`, `active`, `succeeded`, `queued`, `executed`,
    `canceled`, `defeated`
- `DeploymentId` newtype wrapping `String` with `Serialize`/`Deserialize`
- `primitives` module extended with `pub type ChainId = u64` and
  `pub type TxHash = B256`
- All enums derive `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`
- All enums implement `Display` and `FromStr`
- Unit tests for each enum: serialize to JSON string, deserialize back, display
  formatting, fromstr round-trip
- `cargo check` and `cargo test` pass
- `cargo clippy` passes with no warnings

---

### US-002: Deployment Model

**Description:** Define the `Deployment` struct and all its nested types:
`DeploymentStrategy`, `ProxyInfo`, `ProxyUpgrade`, `ArtifactInfo`,
`VerificationInfo`, and `VerifierStatus`. JSON field names must exactly match
the Go version's camelCase tags.

**Acceptance Criteria:**
- `Deployment` struct defined in `crates/treb-core/src/types/deployment.rs` with
  these fields and exact JSON names:
  - `id` (String), `namespace` (String), `chainId` (u64), `contractName` (String),
    `label` (String), `address` (String), `type` (DeploymentType),
    `transactionId` (String), `deploymentStrategy` (DeploymentStrategy),
    `proxyInfo` (Option\<ProxyInfo\>), `artifact` (ArtifactInfo),
    `verification` (VerificationInfo), `tags` (Option\<Vec\<String\>\>),
    `createdAt` (DateTime\<Utc\>), `updatedAt` (DateTime\<Utc\>)
- `DeploymentStrategy` struct with fields: `method`, `salt` (omitempty),
  `initCodeHash` (omitempty), `factory` (omitempty), `constructorArgs` (omitempty),
  `entropy` (omitempty)
- `ProxyInfo` struct with fields: `type`, `implementation`, `admin` (omitempty),
  `history`
- `ProxyUpgrade` struct with fields: `implementationId`, `upgradedAt`, `upgradeTxId`
- `ArtifactInfo` struct with fields: `path`, `compilerVersion`, `bytecodeHash`,
  `scriptPath`, `gitCommit`
- `VerificationInfo` struct with fields: `status`, `etherscanUrl` (omitempty),
  `verifiedAt` (omitempty), `reason` (omitempty), `verifiers` (omitempty)
- `VerifierStatus` struct with fields: `status`, `url` (omitempty),
  `reason` (omitempty)
- All optional fields use `#[serde(skip_serializing_if = "Option::is_none")]`
  (or equivalent for `Vec`)
- All structs derive `Clone`, `Debug`, `PartialEq`, `Serialize`, `Deserialize`
- `tags` serializes as `null` (not `[]`) when `None`, matching Go's `nil` slice
  behavior
- Unit test: construct a `Deployment`, serialize to JSON, verify field names are
  camelCase
- `cargo check` and `cargo test` pass

---

### US-003: Transaction Model

**Description:** Define the `Transaction` struct and its nested types:
`Operation` and `SafeContext`. JSON field names must exactly match the Go
version's camelCase tags.

**Acceptance Criteria:**
- `Transaction` struct defined in `crates/treb-core/src/types/transaction.rs`
  with these fields and exact JSON names:
  - `id` (String), `chainId` (u64), `hash` (String), `status` (TransactionStatus),
    `blockNumber` (u64, omitempty), `sender` (String), `nonce` (u64),
    `deployments` (Vec\<String\>), `operations` (Vec\<Operation\>),
    `safeContext` (Option\<SafeContext\>, omitempty), `environment` (String),
    `createdAt` (DateTime\<Utc\>)
- `Operation` struct with fields: `type`, `target`, `method`,
  `result` (HashMap\<String, serde_json::Value\>)
- `SafeContext` struct with fields: `safeAddress`, `safeTxHash`, `batchIndex` (i32),
  `proposerAddress`
- `blockNumber` serialized with `omitempty` semantics — omitted when 0, matching
  Go's `omitempty` on `uint64`
- `deployments` and `operations` serialize as `[]` (empty array) when empty,
  matching Go's initialized-slice behavior
- All structs derive `Clone`, `Debug`, `PartialEq`, `Serialize`, `Deserialize`
- Unit test: construct a `Transaction`, serialize to JSON, verify field names
- `cargo check` and `cargo test` pass

---

### US-004: Safe Transaction and Governor Proposal Models

**Description:** Define the `SafeTransaction` struct with `SafeTxData` and
`Confirmation` sub-types, and the `GovernorProposal` struct. JSON field names
must exactly match the Go version's camelCase tags.

**Acceptance Criteria:**
- `SafeTransaction` struct defined in
  `crates/treb-core/src/types/safe_transaction.rs` with fields:
  - `safeTxHash` (String), `safeAddress` (String), `chainId` (u64),
    `status` (TransactionStatus), `nonce` (u64), `transactions` (Vec\<SafeTxData\>),
    `transactionIds` (Vec\<String\>), `proposedBy` (String),
    `proposedAt` (DateTime\<Utc\>), `confirmations` (Vec\<Confirmation\>),
    `executedAt` (Option\<DateTime\<Utc\>\>, omitempty),
    `executionTxHash` (String, omitempty),
    `description` (String, omitempty)
- `SafeTxData` struct with fields: `to`, `value`, `data`, `operation` (u8)
- `Confirmation` struct with fields: `signer`, `signature`,
  `confirmedAt` (DateTime\<Utc\>)
- `GovernorProposal` struct defined in
  `crates/treb-core/src/types/governor_proposal.rs` with fields:
  - `proposalId` (String), `governorAddress` (String),
    `timelockAddress` (String, omitempty), `chainId` (u64),
    `status` (ProposalStatus), `transactionIds` (Vec\<String\>),
    `proposedBy` (String), `proposedAt` (DateTime\<Utc\>),
    `description` (String, omitempty),
    `executedAt` (Option\<DateTime\<Utc\>\>, omitempty),
    `executionTxHash` (String, omitempty)
- `transactions` and `transactionIds` on SafeTransaction, `transactionIds` on
  GovernorProposal serialize as `[]` when empty
- `confirmations` serializes as `[]` when empty
- `executionTxHash` omitted when empty string, matching Go's `omitempty` on string
- All structs derive `Clone`, `Debug`, `PartialEq`, `Serialize`, `Deserialize`
- Unit tests for both `SafeTransaction` and `GovernorProposal` serialization
- `cargo check` and `cargo test` pass

---

### US-005: Contract and Network Models

**Description:** Define the `Contract` struct and its nested artifact types
(`Artifact`, `BytecodeObject`, `ArtifactMetadata`), plus the `Network` type.
The `Contract` model represents a compiled Solidity artifact; the `Artifact`
sub-type wraps the full compiler output including ABI as `JsonAbi` from
`alloy-json-abi`.

**Acceptance Criteria:**
- `Contract` struct defined in `crates/treb-core/src/types/contract.rs` with
  fields: `name`, `path`, `artifactPath` (omitempty), `version` (omitempty),
  `artifact` (Option\<Artifact\>, omitempty)
- `Artifact` struct with fields: `abi` (serde_json::Value — raw JSON to match
  Go's `json.RawMessage`), `bytecode` (BytecodeObject),
  `deployedBytecode` (BytecodeObject),
  `methodIdentifiers` (HashMap\<String, String\>), `rawMetadata` (String),
  `metadata` (ArtifactMetadata)
- `BytecodeObject` struct with fields: `object`, `sourceMap`,
  `linkReferences` (HashMap\<String, serde_json::Value\>)
- `ArtifactMetadata` struct with nested `compiler` (with `version`),
  `language`, `output` (with `abi`, `devdoc`, `userdoc`, `metadata`),
  `settings` (with `compilationTarget`)
- `Network` type alias or struct — at minimum `pub type Network = String` with
  a plan to enrich later; `ChainId` already defined in US-001
- All structs derive `Clone`, `Debug`, `PartialEq`, `Serialize`, `Deserialize`
- Unit test: construct a `Contract` with a minimal `Artifact`, serialize, verify
  JSON structure
- `cargo check` and `cargo test` pass

---

### US-006: Golden File Fixtures and Serde Round-Trip Tests

**Description:** Extract representative JSON fixtures from the Go version's
`.treb/` directory and add comprehensive serde round-trip tests that deserialize
real Go-generated JSON into Rust types and re-serialize back, asserting
byte-level (or semantic JSON) equality.

**Acceptance Criteria:**
- Test fixture directory created at `crates/treb-core/tests/fixtures/`
- Golden fixture files extracted from real Go-generated data:
  - `deployment.json` — a single `Deployment` object (from `deployments.json`)
  - `deployment_with_proxy.json` — a `Deployment` with non-null `proxyInfo`
  - `transaction.json` — a single `Transaction` object (from `transactions.json`)
  - `safe_transaction.json` — a single `SafeTransaction` (from `safe-txs.json`)
  - `deployments_map.json` — a small map of deployment ID → Deployment (testing
    `HashMap<String, Deployment>` deserialization, as the registry uses this shape)
  - `transactions_map.json` — a small map of transaction ID → Transaction
  - `safe_txs_map.json` — a small map of safe tx hash → SafeTransaction
- Each fixture is real data extracted from the Go version (not hand-crafted),
  scrubbed of any sensitive data if needed
- Integration test file at `crates/treb-core/tests/serde_golden.rs` with:
  - For each fixture: deserialize JSON → Rust type → re-serialize → parse both
    as `serde_json::Value` → assert equality
  - Test that `HashMap<String, Deployment>` deserializes correctly (this is the
    actual registry file format)
  - Test that `HashMap<String, Transaction>` deserializes correctly
  - Test that `HashMap<String, SafeTransaction>` deserializes correctly
- All tests pass with `cargo test`
- Tests are clearly documented with comments explaining what each fixture tests
- `cargo clippy` passes with no warnings

---

## Functional Requirements

- **FR-1:** All domain types live in `crates/treb-core/src/types/` with a
  `mod.rs` that re-exports everything publicly.

- **FR-2:** JSON serialization uses `#[serde(rename_all = "camelCase")]` on
  structs where all fields are camelCase. For structs with mixed casing or
  reserved words (e.g., the `type` field), use explicit `#[serde(rename)]`
  per-field.

- **FR-3:** Optional fields that use Go's `omitempty` semantics must use
  `#[serde(skip_serializing_if = "Option::is_none")]` for `Option<T>` fields,
  and equivalent skip predicates for empty strings and zero values where Go's
  `omitempty` applies.

- **FR-4:** DateTime fields use `chrono::DateTime<Utc>` with RFC 3339
  serialization (matching Go's `time.Time` default JSON format). The serde
  representation must include timezone offset (e.g.,
  `"2026-03-02T20:30:12.945621099+01:00"`).

- **FR-5:** The `Deployment.tags` field serializes as JSON `null` when `None`
  (not as `[]`), matching Go's behavior where a nil `[]string` serializes as
  `null`.

- **FR-6:** Vec fields like `deployments`, `operations`, `transactions`,
  `transactionIds`, and `confirmations` must never serialize as `null` — they
  default to `[]`. Use `#[serde(default)]` on deserialization and ensure
  constructors initialize them as empty vecs.

- **FR-7:** Enum string representations are exact matches to Go constants:
  uppercase for `DeploymentType`, `DeploymentMethod`, `TransactionStatus`,
  `VerificationStatus`; lowercase for `ProposalStatus`.

- **FR-8:** The `types` module is re-exported from `treb-core`'s `lib.rs` so
  downstream crates can `use treb_core::types::*`.

- **FR-9:** Address and hash fields are stored as `String` (hex-encoded with
  `0x` prefix), not as alloy `Address`/`B256` types, because the JSON format
  uses checksummed hex strings. Alloy newtypes (`TxHash`, `ChainId`) are
  available in the `primitives` module for typed contexts outside of
  serialization.

---

## Non-Goals

- **No registry read/write logic** — That is Phase 4. This phase only defines
  the types; it does not implement file I/O, CRUD, or indexing.
- **No config parsing** — Configuration types belong to Phase 3.
- **No validation logic** — Field validation (e.g., valid Ethereum address
  format, valid chain ID ranges) is deferred to the modules that consume these
  types.
- **No builder pattern or constructors** — Simple struct construction is
  sufficient. Builders can be added in later phases if ergonomics demand it.
- **No new workspace crates** — All types go in `treb-core`. The `treb-registry`
  crate is Phase 4.
- **No ABI parsing** — The `Contract.artifact.abi` field stores raw JSON. Actual
  ABI parsing via `alloy-json-abi::JsonAbi` is for Phase 5+.
- **No CLI changes** — Phase 2 is purely library types and tests.

---

## Technical Considerations

### Dependencies to Add

| Crate | Version | Features | Purpose |
|---|---|---|---|
| `serde` | 1.x | `derive` | Serialization framework |
| `serde_json` | 1.x | — | JSON serialization |
| `chrono` | 0.4.x | `serde` | DateTime types with serde support |
| `alloy-json-abi` | (foundry-pinned) | — | `JsonAbi` type for future ABI use |

### DateTime Serialization

Go's `time.Time` serializes to RFC 3339 with nanosecond precision and timezone
offset (e.g., `"2026-03-02T20:30:12.945621099+01:00"`). Chrono's default serde
uses RFC 3339 as well but may differ in precision or offset formatting. The
golden file tests will catch any mismatches. If chrono's default doesn't match,
a custom serde module may be needed — but try the default first.

Note: Go serializes local time with offset (e.g., `+01:00`), while the Rust
types should use `DateTime<FixedOffset>` rather than `DateTime<Utc>` to preserve
the original offset during round-trip. Evaluate during US-006 and adjust if
needed.

### Go's `omitempty` Semantics

Go's `omitempty` skips:
- `false` for bools
- `0` for numbers
- `""` for strings
- `nil` for pointers, slices, maps
- Empty slices/maps (but NOT zero-value structs)

In Rust, model this with:
- `Option<T>` + `skip_serializing_if = "Option::is_none"` for pointer fields
- Custom `skip_serializing_if` predicates for string (`is_empty`) and numeric
  (`is_zero`) fields where Go uses `omitempty`

### Fixture Extraction

Golden fixtures should be extracted from `/home/sol/projects/treb-state/.treb/`
which contains real deployment data from the Go version. Select representative
entries that exercise all field variations (with/without proxy, various strategies,
safe context present/absent, etc.).

### Module Layout

```
crates/treb-core/src/
├── lib.rs                    (add: pub mod types;)
├── error.rs                  (unchanged)
└── types/
    ├── mod.rs                (re-exports all types)
    ├── enums.rs              (DeploymentType, DeploymentMethod, etc.)
    ├── deployment.rs         (Deployment, DeploymentStrategy, ProxyInfo, etc.)
    ├── transaction.rs        (Transaction, Operation, SafeContext)
    ├── safe_transaction.rs   (SafeTransaction, SafeTxData, Confirmation)
    ├── governor_proposal.rs  (GovernorProposal)
    └── contract.rs           (Contract, Artifact, BytecodeObject, etc.)
```
