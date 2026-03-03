PRD generated at `ralph/tasks/prd-phase-04-registry-system.md`. 

**Summary of Phase 4 — Registry System (Read/Write):**

- **6 user stories**, ordered by dependency:
  1. **US-001** — Crate scaffold + `RegistryMeta` and `LookupIndex` types
  2. **US-002** — Atomic file I/O (temp+rename) with advisory file locking
  3. **US-003** — `DeploymentStore` CRUD over `deployments.json`
  4. **US-004** — `TransactionStore` + `SafeTransactionStore` CRUD
  5. **US-005** — Lookup index build/rebuild and query methods
  6. **US-006** — `Registry` facade, migration detection, golden file integration tests

- **Key design decisions:**
  - Per-file advisory locking via `fs2` (matches Go's approach)
  - Atomic writes via `tempfile::NamedTempFile` + `persist()`
  - Lookup index fully rebuilt on every deployment mutation (simple and correct)
  - Version field in `registry.json` for migration detection (migration logic deferred to Phase 19)
  - All golden file tests compare via `serde_json::Value` equality (not string equality) since HashMap key order is non-deterministic
