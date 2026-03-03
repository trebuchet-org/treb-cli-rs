# PRD: Phase 4 — Registry System (Read/Write)

## Introduction

Phase 4 builds the JSON-backed deployment registry — treb's persistent storage
layer for deployments, transactions, and Safe transactions. Foundry has no
equivalent; this is entirely treb-specific. The registry lives in `.treb/` at the
project root alongside the local config from Phase 3.

**Exact Go compatibility is mandatory.** The Rust implementation must read and
write the same JSON schema, same file layout, and same key formats as the Go
version. Users switching from the Go CLI to the Rust CLI must see zero difference
in their `.treb/` directory contents. All serialization is validated against
golden file fixtures extracted from the Go version.

This phase introduces the `treb-registry` workspace crate. It depends on
`treb-core` for domain types (`Deployment`, `Transaction`, `SafeTransaction`) and
on `serde`/`serde_json` for serialization. No foundry crates are needed.

---

## Goals

1. **Go-compatible file format** — Read and write `.treb/deployments.json`,
   `transactions.json`, `safe-txs.json`, `lookup.json`, and `registry.json` in
   the exact same JSON format as the Go version. Round-trip tests against golden
   fixtures prove byte-level compatibility.

2. **CRUD operations** — Provide create, read, update, and delete operations for
   deployments, transactions, and safe transactions through a unified `Registry`
   facade.

3. **Lookup index** — Build and maintain secondary indices (contract name → ids,
   address → id, tag → ids) in `lookup.json` to support efficient querying
   without scanning all deployments.

4. **Data integrity** — Atomic writes (write-to-temp-then-rename) prevent partial
   writes from corrupting registry files. File-level locking prevents concurrent
   processes from clobbering each other.

5. **Migration readiness** — Registry files include a version field. The system
   detects version mismatches and reports actionable errors. Actual migration
   logic is deferred to Phase 19.

---

## User Stories

### US-001: treb-registry Crate Scaffold and Registry Types

**Description:** Create the `treb-registry` workspace crate with its module
structure. Define the registry metadata type (`RegistryMeta` with version field)
and the lookup index type (`LookupIndex` with name/address/tag maps). These are
the registry-specific types that don't exist in `treb-core`.

**Acceptance Criteria:**
- `crates/treb-registry/` directory created with `Cargo.toml` and `src/lib.rs`
- Crate added to `[workspace.members]` in root `Cargo.toml`
- `treb-registry` added to `[workspace.dependencies]` with `path = "crates/treb-registry"`
- Dependencies: `treb-core` (workspace), `serde` (workspace), `serde_json`
  (workspace), `chrono` (workspace), `thiserror` (workspace)
- Dev dependencies: `tempfile` (workspace)
- `RegistryMeta` struct with serde derives:
  - `version` (u32) — current version is `1`
  - `created_at` (DateTime\<Utc\>) — `#[serde(rename = "createdAt")]`
  - `updated_at` (DateTime\<Utc\>) — `#[serde(rename = "updatedAt")]`
- `RegistryMeta::new()` constructor that sets version to `1` and timestamps to
  `Utc::now()`
- `LookupIndex` struct with serde derives:
  - `by_name`: `HashMap<String, Vec<String>>` — contract name → deployment IDs
  - `by_address`: `HashMap<String, String>` — lowercase address → deployment ID
  - `by_tag`: `HashMap<String, Vec<String>>` — tag → deployment IDs
  - All fields use `#[serde(rename = "byName")]`, `#[serde(rename = "byAddress")]`,
    `#[serde(rename = "byTag")]`
- `LookupIndex::default()` returns empty maps
- Constants for file names: `DEPLOYMENTS_FILE = "deployments.json"`,
  `TRANSACTIONS_FILE = "transactions.json"`, `SAFE_TXS_FILE = "safe-txs.json"`,
  `LOOKUP_FILE = "lookup.json"`, `REGISTRY_FILE = "registry.json"`,
  `REGISTRY_DIR = ".treb"`
- Constants for current version: `REGISTRY_VERSION = 1`
- Module structure: `lib.rs` with type re-exports, `types.rs` for registry types
- Unit tests for `RegistryMeta` and `LookupIndex` serde round-trip
- `cargo check`, `cargo test`, and `cargo clippy` pass

---

### US-002: Atomic File I/O with Locking

**Description:** Implement the low-level file I/O layer that all registry
operations build on. This includes atomic writes (write to a temp file in the
same directory, then rename) and file-level locking (advisory locks via
`std::fs::File` and `flock`/`fcntl` or a cross-platform approach). These
primitives ensure data integrity when multiple treb processes access the same
registry.

**Acceptance Criteria:**
- `io.rs` module in `treb-registry` with:
  - `read_json_file<T: DeserializeOwned>(path: &Path) -> Result<T>`:
    - Returns deserialized `T` from JSON file
    - Returns `TrebError::Registry` with path and parse error message on invalid JSON
    - Returns `TrebError::Io` if file cannot be read
  - `read_json_file_or_default<T: DeserializeOwned + Default>(path: &Path) -> Result<T>`:
    - Returns `T::default()` if file does not exist
    - Otherwise same as `read_json_file`
  - `write_json_file<T: Serialize>(path: &Path, value: &T) -> Result<()>`:
    - Creates parent directory if missing (`create_dir_all`)
    - Writes to a temp file in the same directory (using `tempfile::NamedTempFile`
      with `tempfile_in` to ensure same filesystem)
    - 2-space indented JSON with trailing newline (matching Go output)
    - Renames temp file to target path atomically (`persist()`)
    - Returns `TrebError::Registry` on serialization failure
  - `with_file_lock<F, T>(path: &Path, f: F) -> Result<T>` where `F: FnOnce() -> Result<T>`:
    - Acquires an exclusive advisory lock on a `.lock` file adjacent to `path`
    - Executes `f` while lock is held
    - Releases lock on drop (via RAII)
    - Lock file: `<path>.lock` (e.g., `deployments.json.lock`)
- Add `fs2` (or equivalent) to workspace dependencies for cross-platform file
  locking, OR use `libc::flock` directly on Unix (simpler, matching Go's approach)
- Test: atomic write creates file with correct JSON content and trailing newline
- Test: atomic write creates parent directory if missing
- Test: read nonexistent file with `read_json_file_or_default` returns default
- Test: read invalid JSON returns descriptive `TrebError::Registry`
- Test: write then read round-trips correctly for `HashMap<String, Deployment>`
- Test: concurrent lock acquisition blocks (second lock waits for first to release)
- `cargo check`, `cargo test`, and `cargo clippy` pass

---

### US-003: Deployment Store (CRUD)

**Description:** Implement the deployment store — a typed layer over
`deployments.json` that provides create, read, update, delete, and list
operations for `Deployment` records. The store manages the
`HashMap<String, Deployment>` that maps deployment ID to deployment.

**Acceptance Criteria:**
- `store/deployments.rs` module (or `deployments.rs`) in `treb-registry` with
  `DeploymentStore` struct:
  - `DeploymentStore::new(registry_dir: PathBuf)` — stores path to `.treb/`
  - `load(&self) -> Result<HashMap<String, Deployment>>` — reads
    `deployments.json` (empty HashMap if file missing)
  - `save(&self, deployments: &HashMap<String, Deployment>) -> Result<()>` —
    atomic write to `deployments.json`
  - `get(&self, id: &str) -> Result<Option<Deployment>>` — load and look up by ID
  - `insert(&self, deployment: Deployment) -> Result<()>` — load, insert by
    `deployment.id`, save. Returns `TrebError::Registry` if ID already exists
  - `update(&self, deployment: Deployment) -> Result<()>` — load, replace by
    `deployment.id`, save. Returns `TrebError::Registry` if ID not found. Sets
    `updated_at` to `Utc::now()`
  - `remove(&self, id: &str) -> Result<Option<Deployment>>` — load, remove by ID,
    save. Returns the removed deployment or None
  - `list(&self) -> Result<Vec<Deployment>>` — load, return all values sorted by
    `created_at`
  - `count(&self) -> Result<usize>` — load and return count
- All mutating operations use `with_file_lock` from US-002
- All mutating operations use atomic writes from US-002
- Test: insert then get returns the deployment
- Test: insert duplicate ID returns error
- Test: update existing deployment succeeds, updates `updated_at`
- Test: update nonexistent ID returns error
- Test: remove returns the removed deployment
- Test: list returns deployments sorted by `created_at`
- Test: operations on empty store (no file) work correctly
- Golden file test: load `deployments_map.json` fixture from `treb-core/tests/fixtures/`,
  save through store, re-read and verify JSON value equality
- `cargo check`, `cargo test`, and `cargo clippy` pass

---

### US-004: Transaction and Safe Transaction Stores

**Description:** Implement the transaction store (`transactions.json`) and safe
transaction store (`safe-txs.json`) following the same pattern as the deployment
store. These manage `HashMap<String, Transaction>` and
`HashMap<String, SafeTransaction>` respectively.

**Acceptance Criteria:**
- `store/transactions.rs` module with `TransactionStore` struct:
  - Same API pattern as `DeploymentStore`: `new`, `load`, `save`, `get`, `insert`,
    `update`, `remove`, `list`, `count`
  - Key is `transaction.id` (e.g., `"tx-0x1234..."`)
  - `list` returns transactions sorted by `created_at`
  - All mutating operations use file locking and atomic writes
- `store/safe_transactions.rs` module with `SafeTransactionStore` struct:
  - Same API pattern as `DeploymentStore`
  - Key is `safe_transaction.safe_tx_hash` (e.g., `"0xabcd..."`)
  - `list` returns safe transactions sorted by `proposed_at`
  - All mutating operations use file locking and atomic writes
- `store/mod.rs` to organize store modules with re-exports
- Test: transaction insert, get, update, remove round-trip
- Test: safe transaction insert, get, update, remove round-trip
- Test: list ordering is correct for both stores
- Golden file test: load `transactions_map.json` fixture, save through store,
  verify JSON value equality
- Golden file test: load `safe_txs_map.json` fixture, save through store, verify
  JSON value equality
- `cargo check`, `cargo test`, and `cargo clippy` pass

---

### US-005: Lookup Index Build and Query

**Description:** Implement the lookup index that provides fast secondary lookups
into the deployment store. The index maps contract names, addresses, and tags to
deployment IDs. It is rebuilt from the deployment store on demand and persisted to
`lookup.json`.

**Acceptance Criteria:**
- `lookup.rs` module in `treb-registry` with:
  - `build_lookup_index(deployments: &HashMap<String, Deployment>) -> LookupIndex`:
    - `by_name`: maps lowercase `contract_name` → Vec of deployment IDs that
      share that name
    - `by_address`: maps lowercase `address` → deployment ID (address is unique
      per deployment; last write wins if duplicates exist)
    - `by_tag`: maps each tag → Vec of deployment IDs that have that tag
    - Skips deployments with empty address
  - `LookupStore` struct:
    - `new(registry_dir: PathBuf)` — stores path to `.treb/`
    - `load(&self) -> Result<LookupIndex>` — reads `lookup.json` (default if missing)
    - `save(&self, index: &LookupIndex) -> Result<()>` — atomic write to `lookup.json`
    - `rebuild(&self, deployments: &HashMap<String, Deployment>) -> Result<LookupIndex>`:
      builds index from deployments, saves to disk, returns index
  - Query methods on `LookupIndex`:
    - `find_by_name(&self, name: &str) -> &[String]` — case-insensitive lookup
    - `find_by_address(&self, address: &str) -> Option<&String>` — case-insensitive
    - `find_by_tag(&self, tag: &str) -> &[String]` — exact match
- Test: build index from fixture deployments, verify name/address/tag mappings
- Test: case-insensitive name lookup works
- Test: case-insensitive address lookup works
- Test: tag lookup returns correct deployment IDs
- Test: empty deployments produces empty index
- Test: rebuild saves and can be re-loaded with same results
- `cargo check`, `cargo test`, and `cargo clippy` pass

---

### US-006: Registry Facade, Migration Detection, and Integration Tests

**Description:** Implement the `Registry` facade that ties together all stores
and the lookup index into a single entry point. Add registry metadata management
(`registry.json` with version field) and migration detection. Write comprehensive
integration tests using golden file fixtures.

**Acceptance Criteria:**
- `registry.rs` module in `treb-registry` with `Registry` struct:
  - `Registry::open(project_root: &Path) -> Result<Registry>`:
    - Resolves registry dir as `project_root/.treb/`
    - If `registry.json` exists, reads it and checks version. Returns
      `TrebError::Registry` with actionable message if version > `REGISTRY_VERSION`
      (e.g., "registry version 2 is newer than supported version 1; upgrade treb")
    - If `registry.json` does not exist, this is fine (first use)
    - Constructs internal stores: `DeploymentStore`, `TransactionStore`,
      `SafeTransactionStore`, `LookupStore`
  - `Registry::init(project_root: &Path) -> Result<Registry>`:
    - Creates `.treb/` directory if missing
    - Writes `registry.json` with `RegistryMeta::new()` if it doesn't exist
    - Returns opened registry
  - Delegate methods that forward to underlying stores:
    - Deployments: `get_deployment`, `insert_deployment`, `update_deployment`,
      `remove_deployment`, `list_deployments`, `deployment_count`
    - Transactions: `get_transaction`, `insert_transaction`, `update_transaction`,
      `remove_transaction`, `list_transactions`
    - Safe transactions: `get_safe_transaction`, `insert_safe_transaction`,
      `update_safe_transaction`, `remove_safe_transaction`, `list_safe_transactions`
  - `rebuild_lookup_index(&self) -> Result<LookupIndex>` — loads deployments,
    rebuilds index, saves
  - `lookup(&self) -> Result<LookupIndex>` — loads current lookup index
  - Deployment mutations (`insert_deployment`, `update_deployment`,
    `remove_deployment`) automatically trigger a lookup index rebuild after the
    store operation succeeds
- `MetaStore` (internal) for `registry.json` read/write:
  - Same atomic write pattern as other stores
  - `load` returns `Option<RegistryMeta>` (None if file missing)
  - `save` writes `RegistryMeta`
- Integration test: create a `Registry`, insert 3 deployments, 2 transactions,
  1 safe transaction, then verify all can be retrieved
- Integration test: insert deployment, verify lookup index is updated, query by
  name/address/tag
- Integration test: remove deployment, verify lookup index is updated
- Integration test: load Go golden fixtures (`deployments_map.json`,
  `transactions_map.json`, `safe_txs_map.json`) through the stores, save, re-read,
  verify JSON value equality (proves Go round-trip compatibility)
- Integration test: `Registry::open` with a `registry.json` containing version 2
  returns descriptive error
- Integration test: `Registry::open` with no `.treb/` directory returns ok
  (empty state)
- `cargo check`, `cargo test`, and `cargo clippy` pass

---

## Functional Requirements

- **FR-1:** The `treb-registry` crate is the single owner of all registry I/O.
  Other crates access the registry through the `Registry` facade — they never
  read or write `.treb/*.json` files directly.

- **FR-2:** All registry JSON files use 2-space indentation with a trailing
  newline, matching the Go version's output format. `serde_json::to_string_pretty`
  produces 2-space indentation by default; append `\n` before writing.

- **FR-3:** Deployments are keyed by `deployment.id` (e.g.,
  `"mainnet/42220/FPMM:v3.0.0"`). Transactions are keyed by `transaction.id`
  (e.g., `"tx-0x1234..."`). Safe transactions are keyed by `safe_tx_hash` (e.g.,
  `"0xabcd..."`).

- **FR-4:** All file writes are atomic: write to a temp file in the same
  directory, then rename to the target path. This prevents partial writes from
  corrupting files on crash.

- **FR-5:** Mutating operations acquire an exclusive advisory file lock before
  reading, modifying, and writing. The lock is held for the entire read-modify-write
  cycle to prevent TOCTOU races.

- **FR-6:** The lookup index is rebuilt from the full deployment set after every
  deployment mutation. This is simple and correct — optimizing for incremental
  updates is unnecessary given the expected data sizes.

- **FR-7:** `registry.json` contains a `version` field. If the version on disk
  is higher than the code supports, `Registry::open` returns an error telling the
  user to upgrade. If lower or missing, the registry opens normally (migration
  logic is deferred to Phase 19).

- **FR-8:** All registry errors use `TrebError::Registry` with messages that
  include the file path and a human-readable description.

- **FR-9:** The `HashMap` serialization order in JSON files is non-deterministic
  (serde_json does not sort keys). Round-trip tests must compare via
  `serde_json::Value` equality, not string equality. This matches the Go
  behavior where map iteration order is also non-deterministic.

---

## Non-Goals

- **No CLI commands** — `treb list`, `treb show`, `treb init` (which creates the
  registry) belong to Phases 10 and 11. This phase implements the library.
- **No registry migration logic** — Detecting version mismatches is in scope;
  actually migrating data between versions is Phase 19.
- **No query filtering** — Complex queries (filter by namespace, chain, type,
  tag) belong to Phase 11. This phase provides the raw CRUD and lookup index.
- **No deployment ID generation** — ID format construction
  (`namespace/chainId/Name:label`) is a concern of the recording pipeline
  (Phase 8). The registry stores whatever ID it receives.
- **No network-aware operations** — The registry stores chain IDs but does not
  validate them against RPC endpoints or resolve network names.
- **No encryption or access control** — Registry files are plain JSON, readable
  and writable by the user. No secrets are stored in the registry.

---

## Technical Considerations

### Dependencies to Add

| Crate | Version | Purpose |
|---|---|---|
| `fs2` | 0.4.x | Cross-platform file locking (`lock_exclusive`, `unlock`) |
| `tempfile` | 3.x | Atomic writes via `NamedTempFile` + `persist()` (already in workspace dev-deps; add as regular dep for `treb-registry`) |

### File Layout

```
.treb/
├── config.local.json     (Phase 3 — already exists)
├── registry.json          (version metadata)
├── deployments.json       (HashMap<String, Deployment>)
├── transactions.json      (HashMap<String, Transaction>)
├── safe-txs.json          (HashMap<String, SafeTransaction>)
└── lookup.json            (LookupIndex)
```

### Module Layout

```
crates/treb-registry/
├── Cargo.toml
├── src/
│   ├── lib.rs             (pub re-exports, constants)
│   ├── types.rs           (RegistryMeta, LookupIndex)
│   ├── io.rs              (atomic read/write, file locking)
│   ├── lookup.rs          (index build/query, LookupStore)
│   ├── registry.rs        (Registry facade, MetaStore)
│   └── store/
│       ├── mod.rs          (re-exports)
│       ├── deployments.rs  (DeploymentStore)
│       ├── transactions.rs (TransactionStore)
│       └── safe_transactions.rs (SafeTransactionStore)
└── tests/
    └── (inline unit tests + integration tests using treb-core golden fixtures)
```

### Golden File Testing Strategy

The golden fixture files from Phase 2 (`crates/treb-core/tests/fixtures/`) serve
double duty:

1. **Phase 2** uses them to prove domain type serde correctness
2. **Phase 4** uses them to prove the registry can round-trip Go-generated data

The registry tests should:
- Read the fixture JSON into a `HashMap<String, T>`
- Write it through the store's `save` method
- Read it back through the store's `load` method
- Compare the original and loaded values via `serde_json::Value` equality

### Concurrency Model

File locking uses advisory locks (POSIX `flock` via `fs2`). This protects
against concurrent treb processes but not against external tools that don't
respect advisory locks. This matches the Go version's behavior.

The lock granularity is per-file: `deployments.json.lock`,
`transactions.json.lock`, etc. Different file types can be modified concurrently
by different processes (e.g., one process writes deployments while another writes
transactions).

### Compatibility with treb-config

Both `treb-config` and `treb-registry` write to the `.treb/` directory. They use
different files and do not conflict. The `REGISTRY_DIR` constant should match
`treb-config`'s `LOCAL_DIR` constant (both `.treb`).
