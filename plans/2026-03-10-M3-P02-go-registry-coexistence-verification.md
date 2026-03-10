# PRD: Phase 2 - Go Registry Coexistence Verification

## Introduction

Phase 2 verifies that the Rust CLI's registry stores are fully compatible with the Go CLI's registry format, enabling both CLIs to operate on the same `.treb/` directory without data corruption. Phase 1 removed the `registry.json` metadata file that blocked coexistence; this phase addresses the remaining store files (`deployments.json`, `transactions.json`, `safe-txs.json`) where format differences could cause failures.

A critical finding from pre-phase analysis: Rust's Phase 1 `{"_format":"treb-v1","entries":{...}}` wrapper **breaks Go CLI reads**. Go uses `json.Unmarshal` into `map[string]*T`, which fails when it encounters the `_format` string value instead of a struct. This phase must fix the write path to maintain bidirectional compatibility while preserving Rust's ability to read both formats.

## Goals

1. **Go→Rust read compatibility**: Rust CLI can load and operate on all store files created by the Go CLI without errors, tested against real production data (731 deployments, 1529 transactions, 96 safe-txs from `mento-deployments-v2`).
2. **Rust→Go write compatibility**: Files written by the Rust CLI remain parseable by the Go CLI — no wrapper or extra fields that break `json.Unmarshal` into `map[string]*T`.
3. **Documented compatibility surface**: Any fields Go writes that Rust ignores (or vice versa) are identified and documented, with `#[serde(default)]` coverage ensuring forward-compatible deserialization.
4. **Regression protection**: Automated tests prevent future type changes from breaking cross-CLI compatibility.

## User Stories

### P2-US-001: Create Go Registry Test Fixtures from Production Data

**Description:** Extract a representative subset of Go-created registry data from `~/projects/mento-deployments-v2/.treb/` and store it as test fixtures in the Rust codebase. These fixtures are the foundation for all compatibility tests in this phase.

**Tasks:**
- Copy `deployments.json`, `transactions.json`, and `safe-txs.json` from `~/projects/mento-deployments-v2/.treb/`
- Extract a representative subset (~10-15 entries per file) covering edge cases: SINGLETON/PROXY/LIBRARY types, CREATE/CREATE2/CREATE3 methods, entries with and without tags, entries with and without proxy info, transactions with and without safeContext, safe-txs with and without executedAt, empty vs populated operations arrays
- Store fixtures at `crates/treb-registry/tests/fixtures/go-compat/` (one file per store)
- Ensure fixture data uses the bare JSON map format (no `_format` wrapper) since that is what Go produces

**Acceptance Criteria:**
- [ ] `crates/treb-registry/tests/fixtures/go-compat/deployments.json` exists with 10-15 representative Go-created deployment entries
- [ ] `crates/treb-registry/tests/fixtures/go-compat/transactions.json` exists with 10-15 representative Go-created transaction entries
- [ ] `crates/treb-registry/tests/fixtures/go-compat/safe-txs.json` exists with 5-10 representative Go-created safe transaction entries
- [ ] All fixtures are bare JSON maps (no `_format` wrapper), matching Go's output format
- [ ] Fixtures include coverage for: SINGLETON, PROXY, and LIBRARY deployment types; CREATE, CREATE2, CREATE3 methods; null vs populated tags; null vs populated proxyInfo; present vs absent safeContext; present vs absent executedAt; timezone-offset timestamps (e.g., `+01:00`, `+02:00`)
- [ ] `cargo check -p treb-registry` passes

---

### P2-US-002: Verify and Fix Rust Deserialization of Go-Created Store Files

**Description:** Write unit tests that load each Go-compat fixture file into the corresponding Rust store and verify all entries deserialize without errors. Fix any field mismatches discovered.

**Tasks:**
- Add unit tests in `crates/treb-registry/src/store/deployments.rs` (or a dedicated test module) that load the Go-compat `deployments.json` fixture via `read_versioned_file()` and assert all entries parse correctly
- Add unit tests for `transactions.json` and `safe-txs.json` similarly
- Verify specific edge cases: timezone-offset timestamps parse into `DateTime<Utc>`, `tags: null` deserializes to `None`, empty `operations: []` deserializes to `Vec::new()`, `blockNumber` absent when 0
- Fix any deserialization failures by adding `#[serde(default)]` or adjusting types in `treb-core`
- Verify enum values from Go data match Rust enum variants (SINGLETON, PROXY, CREATE, CREATE2, CREATE3, EXECUTED, SIMULATED, QUEUED, FAILED, UNVERIFIED, VERIFIED, PARTIAL)

**Acceptance Criteria:**
- [ ] Test `go_compat_deployments_deserialize` loads all entries from Go fixture without errors
- [ ] Test `go_compat_transactions_deserialize` loads all entries from Go fixture without errors
- [ ] Test `go_compat_safe_txs_deserialize` loads all entries from Go fixture without errors
- [ ] Tests verify entry count matches expected fixture size
- [ ] Tests spot-check at least 2 entries per file for correct field values (address, chainId, status, etc.)
- [ ] Timezone-offset timestamps (e.g., `"2026-03-09T18:05:36.422290725+01:00"`) parse correctly
- [ ] `cargo test -p treb-registry` passes
- [ ] `cargo clippy -p treb-registry` passes

---

### P2-US-003: Fix Rust Write Path for Go-Compatible Output

**Description:** The `write_versioned_file()` function wraps store data in `{"_format":"treb-v1","entries":{...}}`, which breaks Go's `json.Unmarshal` into `map[string]*T`. Fix the write path so Rust-written files remain readable by Go, while preserving Rust's ability to read both wrapped and bare formats.

**Tasks:**
- Modify `write_versioned_file()` in `crates/treb-registry/src/io.rs` to write bare JSON maps (no `_format`/`entries` wrapper), matching Go's expected format
- Keep `read_versioned_file()` unchanged — it must continue reading both wrapped and bare formats for backward compatibility with files written by earlier Rust versions
- Update the `VersionedStore` type or write helpers as needed
- Update all unit tests in `io.rs` that assert on the wrapped write format
- Update any store-level tests that verify the `_format` field in written output
- Verify `write_versioned_file()` still acquires the advisory file lock before writing

**Acceptance Criteria:**
- [ ] `write_versioned_file()` writes bare JSON maps (sorted `BTreeMap`, 2-space indent, trailing newline) — no `_format` or `entries` wrapper
- [ ] `read_versioned_file()` still reads both wrapped format (for backward compat) and bare format
- [ ] File locking behavior unchanged — `write_versioned_file()` still acquires exclusive lock
- [ ] Store round-trip test: write via `write_versioned_file()`, read back via `read_versioned_file()`, data matches
- [ ] Backward compat test: files with `{"_format":"treb-v1","entries":{...}}` written by earlier Rust versions still load correctly
- [ ] `cargo test -p treb-registry` passes
- [ ] `cargo clippy -p treb-registry` passes

---

### P2-US-004: Verify Round-Trip Compatibility and Document Field Gaps

**Description:** Verify the full round-trip: load Go-created fixtures → modify via Rust (e.g., add tag) → save → verify the written file is still Go-parseable (bare map format, no extra fields). Document any fields Go writes that Rust ignores.

**Tasks:**
- Write a test that loads Go-compat `deployments.json`, modifies an entry (e.g., adds a tag), saves via `DeploymentStore::save()`, then re-reads and verifies the output is a bare JSON map with correct camelCase keys
- Write similar tests for `TransactionStore` and `SafeTransactionStore`
- Verify that Rust does not add any fields that Go doesn't know about (compare output JSON keys against Go struct JSON tags)
- Audit Rust types for `#[serde(deny_unknown_fields)]` — confirm it is NOT used (would break forward compat when Go adds new fields)
- Document findings: any fields Go writes that Rust silently ignores, any fields Rust writes that Go doesn't expect
- Update `CLAUDE.md` or add comments in type files with compatibility notes if gaps are found

**Acceptance Criteria:**
- [ ] Test `go_compat_deployments_round_trip`: load Go fixture → modify → save → verify bare JSON output with all expected keys present and no extra keys
- [ ] Test `go_compat_transactions_round_trip`: same pattern
- [ ] Test `go_compat_safe_txs_round_trip`: same pattern
- [ ] Output JSON keys match Go's JSON tags exactly (verified by comparing key sets)
- [ ] No Rust type uses `#[serde(deny_unknown_fields)]`
- [ ] Any compatibility gaps are documented in code comments on the relevant type definitions
- [ ] `cargo test -p treb-registry` passes
- [ ] `cargo clippy -p treb-registry` passes

---

### P2-US-005: CLI Integration Tests with Go Registry Data

**Description:** Verify that CLI read commands (`list`, `show`) and write commands (`tag`) work correctly against Go-created registry data, end-to-end.

**Tasks:**
- Create an integration test that sets up a project workdir with Go-compat fixture data (using the fixtures from P2-US-001) and runs `treb list` — verify output shows deployments correctly
- Create a test that runs `treb show <deployment-id>` against Go-compat data — verify correct output
- Create a test that runs `treb tag --add <tag> <deployment-id>` against Go-compat data — verify tag is added and the store file remains Go-compatible (bare JSON, correct keys)
- Use the existing `IntegrationTest` framework with a `post_setup_hook` that copies Go-compat fixtures into `.treb/`
- Verify the lookup index is rebuilt correctly from Go-created deployments

**Acceptance Criteria:**
- [ ] Integration test: `list` against Go registry data shows correct deployment count and names
- [ ] Integration test: `show <id>` against Go registry data displays correct deployment details
- [ ] Integration test: `tag --add core <id>` modifies a Go-created deployment and the resulting `deployments.json` is still a bare JSON map (Go-compatible)
- [ ] Lookup index (`.treb/lookup.json`) is correctly built from Go-created deployments
- [ ] All integration tests pass with `cargo test -p treb-cli`
- [ ] `cargo clippy --workspace --all-targets` passes

---

## Functional Requirements

- **FR-1:** Rust CLI must load Go-created `deployments.json` (bare `map[string]Deployment` format) without errors.
- **FR-2:** Rust CLI must load Go-created `transactions.json` (bare `map[string]Transaction` format) without errors.
- **FR-3:** Rust CLI must load Go-created `safe-txs.json` (bare `map[string]SafeTransaction` format) without errors.
- **FR-4:** Rust CLI must write store files in bare JSON map format (no `_format`/`entries` wrapper) so Go CLI can read them.
- **FR-5:** Rust CLI must continue reading files with the `{"_format":"treb-v1","entries":{...}}` wrapper (backward compat with earlier Rust versions).
- **FR-6:** Timestamps with timezone offsets (e.g., `+01:00`, `+02:00`) must parse correctly into `DateTime<Utc>`.
- **FR-7:** `null` values for optional fields (`tags`, `proxyInfo`, `safeContext`, `executedAt`) must deserialize correctly.
- **FR-8:** Go JSON field names (camelCase) must match Rust serde output exactly — no extra or missing fields.
- **FR-9:** Store files must maintain deterministic key ordering (sorted `BTreeMap`) on write.
- **FR-10:** File locking (advisory exclusive lock) must be maintained on all write operations.

## Non-Goals

- **Modifying the Go CLI** — all compatibility fixes are on the Rust side only.
- **`registry.json` compatibility** — already handled in Phase 1 (Rust ignores this file).
- **`governor-txs.json` and `fork.json`** — these are Rust-specific stores not written by Go; coexistence is not a concern.
- **`lookup.json` compatibility** — this is a Rust-generated index file; Go does not read or write it.
- **`addressbook.json`** — will be handled in Phase 9.
- **Environment config files** (`mainnet.json`, `sepolia.json`, etc.) — these are config files managed by `treb-config`, not registry stores.
- **Performance optimization** of file I/O — out of scope.
- **Schema validation or migration** — Phase 1 deliberately removed the migration system; this phase does not reintroduce any schema versioning logic.

## Technical Considerations

### Dependencies
- **Phase 1 (completed):** Versioned I/O helpers (`read_versioned_file`, `write_versioned_file`) in `crates/treb-registry/src/io.rs`, removal of `registry.json` metadata.
- **Go CLI source:** `../treb-cli` for reference (read-only, not modified).
- **Production data:** `~/projects/mento-deployments-v2/.treb/` for fixture extraction.

### Key Files
| File | Role |
|------|------|
| `crates/treb-registry/src/io.rs` | Versioned I/O helpers — write path needs modification |
| `crates/treb-registry/src/store/deployments.rs` | Deployment store — verify Go compat |
| `crates/treb-registry/src/store/transactions.rs` | Transaction store — verify Go compat |
| `crates/treb-registry/src/store/safe_transactions.rs` | Safe transaction store — verify Go compat |
| `crates/treb-core/src/types/deployment.rs` | Deployment type — potential field fixes |
| `crates/treb-core/src/types/transaction.rs` | Transaction type — potential field fixes |
| `crates/treb-core/src/types/safe_transaction.rs` | SafeTransaction type — potential field fixes |
| `crates/treb-core/src/types/enums.rs` | Enum definitions — verify variant names match Go |

### Critical Compatibility Issue
Go's `json.Unmarshal` into `map[string]*Deployment` **fails** on Rust's wrapped format `{"_format":"treb-v1","entries":{...}}` because it tries to deserialize the string `"treb-v1"` as a `*Deployment` pointer. The fix is to write bare JSON maps from Rust while keeping read support for wrapped files (backward compat with earlier Rust versions).

### Backward Compatibility Chain
After this phase, the Rust CLI's read path must handle three scenarios:
1. **Go-created files:** bare `map[string]T` → read via `read_versioned_file()` bare path
2. **Earlier Rust-created files:** `{"_format":"treb-v1","entries":{...}}` → read via wrapped path
3. **Current Rust-created files:** bare `map[string]T` (matching Go format) → read via bare path

### Test Strategy
- **Unit tests** in `treb-registry`: fixture-based deserialization and round-trip tests
- **Integration tests** in `treb-cli`: CLI commands against Go-created registry data
- **Validation approach:** compare JSON key sets between Go struct tags and Rust serde output to detect drift
- **No live Go CLI needed:** compatibility verified structurally (output format matches Go's expected input format)
