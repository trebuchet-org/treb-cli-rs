# PRD: Phase 1 - Remove Registry Meta and Migration System

## Introduction

The Rust CLI maintains a `registry.json` metadata file (`RegistryMeta` with `version`, `createdAt`, `updatedAt`) and a migration runner (`migrations.rs`) that were built speculatively for future schema evolution. In practice, this system creates a compatibility conflict: the Go CLI uses `registry.json` for a completely different purpose (storing the Solidity Registry contract address map). The migration system adds complexity for a problem that doesn't exist yet — the only real migration (v1→v2) just renamed three files, and that rename has already been applied to all known registries.

This phase removes the meta/migration system entirely and replaces it with a lightweight `_format` field in each store file, following Foundry's pattern (`foundry-compilers` uses `"_format"` in cache files). The `_format` field is written on save but not validated on load — if a future schema change makes deserialization fail, the code treats the file as empty/corrupt (implicit invalidation, no migration runner needed).

This is Phase 1 of the "CLI Compatibility and Drop-in Parity with Go" master plan. It unblocks Phase 2 (Go Registry Coexistence Verification) and Phase 9 (Addressbook Command) by establishing the store patterns they will follow.

## Goals

1. **Eliminate the registry.json conflict** — The Rust CLI no longer creates, reads, or requires `registry.json`, allowing it to coexist with Go CLI registries that use the same filename for a different purpose.

2. **Remove dead complexity** — Delete the entire migration system (`migrations.rs`, `MigrationReport`, `run_migrations`, backup logic) and the `treb migrate registry` CLI subcommand. Zero migration code remains.

3. **Add format versioning to all store files** — Every store JSON file is saved with a `{"_format": "treb-v1", "entries": {...}}` wrapper, providing a lightweight version marker for future implicit invalidation without a migration runner.

4. **Maintain backward compatibility** — All stores accept both the bare map format (existing Rust CLI registries) and the new wrapped format on load, ensuring seamless upgrade without user intervention.

5. **All existing tests pass** — Update or remove tests affected by the removal; add new tests for the versioned wrapper round-trip and backward compatibility.

## User Stories

### P1-US-001: Add VersionedStore Wrapper Type and Versioned I/O Helpers

**As a** developer working on the registry crate,
**I want** a generic `VersionedStore<T>` wrapper type and corresponding read/write helpers,
**so that** all stores can adopt the `{"_format": "treb-v1", "entries": {...}}` format consistently.

**Changes:**
- `crates/treb-registry/src/types.rs` — Add `VersionedStore<T>` struct with `_format: String` and `entries: T` fields, using `#[serde(rename = "_format")]` for the format field
- `crates/treb-registry/src/io.rs` — Add `read_versioned_file<T>()` that tries wrapped format first, falls back to bare map; add `write_versioned_file<T>()` that always writes wrapped format
- `crates/treb-registry/src/lib.rs` — Export the new types and add `STORE_FORMAT: &str = "treb-v1"` constant

**Acceptance Criteria:**
- [ ] `VersionedStore<T>` serializes to `{"_format": "treb-v1", "entries": {...}}`
- [ ] `read_versioned_file<T>()` deserializes a wrapped-format file correctly
- [ ] `read_versioned_file<T>()` deserializes a bare-map file correctly (backward compat)
- [ ] `read_versioned_file<T>()` returns `T::default()` when the file does not exist
- [ ] `write_versioned_file<T>()` uses the same atomic write + file lock pattern as existing `write_json_file`
- [ ] `STORE_FORMAT` constant is `"treb-v1"`
- [ ] Typecheck passes (`cargo check -p treb-registry`)

---

### P1-US-002: Update DeploymentStore to Use Versioned Format

**As a** user with an existing deployment registry,
**I want** the deployment store to transparently upgrade to the versioned format on save,
**so that** my existing `deployments.json` continues to work without manual intervention.

**Changes:**
- `crates/treb-registry/src/store/deployments.rs` — Replace `read_json_file_or_default`/`write_json_file` calls with `read_versioned_file`/`write_versioned_file`

**Acceptance Criteria:**
- [ ] Loading a bare `{"id": {...}, ...}` deployments.json succeeds (backward compat)
- [ ] Loading a wrapped `{"_format": "treb-v1", "entries": {"id": {...}, ...}}` deployments.json succeeds
- [ ] Saving always writes the wrapped format with `"_format": "treb-v1"`
- [ ] All existing DeploymentStore tests pass
- [ ] Typecheck passes (`cargo check -p treb-registry`)

---

### P1-US-003: Update LookupStore to Use Versioned Format

**As a** developer maintaining the registry,
**I want** the lookup store to use the versioned format,
**so that** `lookup.json` is consistent with other store files.

**Changes:**
- `crates/treb-registry/src/lookup.rs` — Replace `read_json_file_or_default`/`write_json_file` calls with `read_versioned_file`/`write_versioned_file`

**Note:** LookupStore is structurally different from other stores — it holds a `LookupIndex` (with `by_name`, `by_address`, `by_tag` maps) rather than a keyed `HashMap`. The versioned wrapper wraps `LookupIndex` directly as the `entries` value.

**Acceptance Criteria:**
- [ ] Loading a bare `{"byName": {...}, "byAddress": {...}, "byTag": {...}}` lookup.json succeeds
- [ ] Loading a wrapped `{"_format": "treb-v1", "entries": {"byName": {...}, ...}}` lookup.json succeeds
- [ ] Saving always writes the wrapped format
- [ ] `rebuild()` writes the wrapped format
- [ ] Existing LookupStore tests pass
- [ ] Typecheck passes (`cargo check -p treb-registry`)

---

### P1-US-004: Update TransactionStore, SafeTransactionStore, and GovernorProposalStore to Use Versioned Format

**As a** user with existing transaction records,
**I want** all transaction-related stores to transparently upgrade to the versioned format,
**so that** `transactions.json`, `safe-txs.json`, and `governor-txs.json` are consistently formatted.

**Changes:**
- `crates/treb-registry/src/store/transactions.rs` — Replace read/write calls with versioned variants
- `crates/treb-registry/src/store/safe_transactions.rs` — Same
- `crates/treb-registry/src/store/governor_proposals.rs` — Same

**Acceptance Criteria:**
- [ ] Each store loads bare map format (backward compat)
- [ ] Each store loads wrapped format
- [ ] Each store saves in wrapped format with `"_format": "treb-v1"`
- [ ] All existing tests for TransactionStore, SafeTransactionStore, and GovernorProposalStore pass
- [ ] Typecheck passes (`cargo check -p treb-registry`)

---

### P1-US-005: Update ForkStateStore to Use Versioned Format

**As a** user with active fork sessions,
**I want** the fork state store to use the versioned format,
**so that** `fork.json` is consistent with other stores and existing fork state is preserved.

**Changes:**
- `crates/treb-registry/src/store/fork_state.rs` — Replace read/write calls with versioned variants for `fork.json`

**Note:** ForkStateStore has additional complexity — it manages both the main `fork.json` and snapshot copies in `.treb/snapshots/`. Snapshot read/write should also use the versioned helpers for consistency, since `snapshot_registry()` and `restore_registry()` copy the same store files.

**Acceptance Criteria:**
- [ ] Loading a bare map `fork.json` succeeds (backward compat)
- [ ] Loading a wrapped `fork.json` succeeds
- [ ] Saving always writes the wrapped format
- [ ] Fork snapshot/restore operations work correctly with versioned files
- [ ] All existing ForkStateStore tests pass
- [ ] Typecheck passes (`cargo check -p treb-registry`)

---

### P1-US-006: Remove RegistryMeta, MetaStore, Migrations Module, and Constants

**As a** developer simplifying the registry crate,
**I want** to remove all registry metadata and migration code,
**so that** the crate no longer creates or depends on `registry.json` and has no migration complexity.

**Changes:**
- `crates/treb-registry/src/registry.rs` — Remove `MetaStore` struct and all its methods; remove version check from `Registry::open()`; remove `registry.json` creation from `Registry::init()`; remove `meta` field from internal state if present
- `crates/treb-registry/src/migrations.rs` — Delete entire file
- `crates/treb-registry/src/types.rs` — Remove `RegistryMeta` struct
- `crates/treb-registry/src/lib.rs` — Remove `REGISTRY_FILE` and `REGISTRY_VERSION` constants; remove `mod migrations` declaration; remove `pub use` of `MigrationReport` and `run_migrations`
- Update any tests in `registry.rs` that reference `RegistryMeta`, version checks, or `registry.json`

**Acceptance Criteria:**
- [ ] `registry.json` is not created by `Registry::init()`
- [ ] `Registry::open()` does not read or check `registry.json`
- [ ] `Registry::open()` works in a directory that contains a Go-created `registry.json` (simply ignores it)
- [ ] `migrations.rs` file is deleted
- [ ] `RegistryMeta` type no longer exists
- [ ] `REGISTRY_FILE` and `REGISTRY_VERSION` constants no longer exist
- [ ] `MigrationReport` and `run_migrations` are no longer exported
- [ ] All registry crate tests pass (update tests that checked for `registry.json` creation or version validation)
- [ ] Typecheck passes (`cargo check -p treb-registry`)

---

### P1-US-007: Remove `treb migrate registry` CLI Subcommand and Update Tests

**As a** CLI user,
**I want** the `treb migrate registry` subcommand removed,
**so that** the CLI surface reflects the simplified registry architecture.

**Changes:**
- `crates/treb-cli/src/commands/migrate.rs` — Remove `Registry` variant from `MigrateSubcommand` enum; remove `run_registry()` function and all supporting code; update `run()` to remove registry arm; keep `treb migrate config` intact
- `crates/treb-cli/src/main.rs` — If there are any references to registry migration in help text or command grouping, update them
- Delete golden test directories for registry migration: `migrate_registry_up_to_date`, `migrate_registry_dry_run_up_to_date`, and any other `migrate_registry_*` golden tests
- `crates/treb-cli/tests/integration_migrate.rs` — Remove registry migration test cases
- `crates/treb-cli/tests/cli_prune_reset_migrate.rs` — Remove registry migration test cases
- Add new unit tests verifying versioned format round-trip: write a wrapped file, read it back; write a bare map file, read it back with versioned reader

**Acceptance Criteria:**
- [ ] `treb migrate registry` is no longer a valid command (returns unknown subcommand error)
- [ ] `treb migrate config` continues to work unchanged
- [ ] `treb migrate --help` no longer mentions `registry` subcommand
- [ ] All golden test `.expected` files for registry migration are removed
- [ ] Integration tests for registry migration are removed
- [ ] New tests verify: bare-map-to-versioned round-trip for at least one store, and `_format` field is present in saved output
- [ ] All tests pass (`cargo test --workspace --all-targets`)
- [ ] Clippy passes (`cargo clippy --workspace --all-targets`)

## Functional Requirements

- **FR-1:** A new `VersionedStore<T>` generic type wraps store data as `{"_format": "treb-v1", "entries": <T>}` in JSON serialization.
- **FR-2:** A `read_versioned_file<T>(path)` function attempts to deserialize the wrapped format first; if that fails, falls back to deserializing as bare `T`; returns `T::default()` if the file does not exist.
- **FR-3:** A `write_versioned_file<T>(path, value)` function always writes the wrapped format using the same atomic write + file lock pattern as `write_json_file`.
- **FR-4:** All six store files (`deployments.json`, `transactions.json`, `safe-txs.json`, `governor-txs.json`, `fork.json`, `lookup.json`) use `write_versioned_file` on save and `read_versioned_file` on load.
- **FR-5:** `Registry::init()` no longer creates `registry.json`.
- **FR-6:** `Registry::open()` no longer reads or validates `registry.json` and does not fail if a Go-created `registry.json` exists in the directory.
- **FR-7:** The `migrations.rs` module is deleted and the `treb migrate registry` CLI subcommand is removed.
- **FR-8:** The `treb migrate config` subcommand continues to function unchanged.
- **FR-9:** The `STORE_FORMAT` constant is set to `"treb-v1"` and used by all stores.

## Non-Goals

- **No migration from Go registry format** — This phase does not convert Go's `registry.json` (Solidity Registry addresses) into any Rust format. Coexistence verification is Phase 2.
- **No format validation on load** — The `_format` field is written but not checked. Implicit invalidation (deserialization failure = treat as empty) is the intended behavior. No version comparison logic.
- **No changes to the `treb migrate config` subcommand** — Config migration (v1→v2 treb.toml conversion) is unrelated and remains untouched.
- **No changes to store CRUD logic** — The store pattern (in-memory cache, write-through, file lock, atomic rename) is unchanged. Only the serialization wrapper changes.
- **No changes to snapshot/restore file lists** — `snapshot_registry()` and `restore_registry()` continue to operate on the same set of files. The snapshot functions just need to handle the new wrapped format, which they get for free since the stores themselves now write it.
- **No `_format` field on `registry.json`** — Since `registry.json` is being removed entirely, there is no need to version it.

## Technical Considerations

### Dependencies
- No external crate dependencies. This phase only uses existing `serde`, `serde_json`, and `chrono` (removal of `chrono` usage in `RegistryMeta` may simplify the dependency tree if no other types use it).

### Backward Compatibility Strategy
The `read_versioned_file<T>` function must handle three cases:
1. **File does not exist** → Return `T::default()`
2. **File contains wrapped format** (`{"_format": "...", "entries": {...}}`) → Deserialize wrapped, return entries
3. **File contains bare map** (`{"key": {...}, ...}`) → Deserialize as `T` directly

The simplest implementation: try deserializing as `VersionedStore<T>` first; if that fails, try deserializing as `T` directly. Since `VersionedStore` has a required `_format` field, bare maps will fail the first attempt cleanly.

### LookupStore Nuance
`LookupStore` wraps a `LookupIndex` (not a `HashMap`), so `entries` will contain `{"byName": {...}, "byAddress": {...}, "byTag": {...}}`. This is fine — `VersionedStore<T>` is generic over `T`, not restricted to maps.

### ForkStateStore Data Shape
`ForkStateStore` has a composite data shape: `{"forks": HashMap, "history": Vec}`. The versioned wrapper wraps this entire structure, not just the forks map. The `entries` field contains the full `ForkStateData` (or equivalent).

### Test Impact
- **Remove:** ~12 migration tests in `migrations.rs`, ~3 registry meta tests in `registry.rs`, golden tests for `migrate_registry_*`, integration tests for registry migration
- **Update:** Registry init/open tests that assert `registry.json` existence
- **Add:** Versioned format round-trip tests (bare → wrapped upgrade, wrapped → wrapped identity)

### Integration Points
- Phase 2 (Go Registry Coexistence) builds directly on this work — with `registry.json` no longer conflicting, the next step is verifying that store file formats are compatible between Go and Rust CLIs.
- Phase 9 (Addressbook Command) will follow the same `VersionedStore` pattern when creating the new `addressbook.json` store.
