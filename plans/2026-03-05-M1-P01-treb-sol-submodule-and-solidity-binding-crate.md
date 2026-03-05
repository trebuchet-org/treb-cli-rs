# PRD: Phase 1 - treb-sol Submodule and Solidity Binding Crate

## Introduction

This phase adds the `treb-sol` Solidity repository as a git submodule and creates a dedicated Rust workspace crate (`treb-sol`) that generates type-safe bindings from the Solidity interface files using `alloy::sol!`. Currently, `treb-forge/src/events/abi.rs` duplicates the Solidity event and struct definitions inline. This is fragile — if the Solidity interfaces change, the Rust code silently drifts out of sync. By sourcing bindings directly from the canonical Solidity files, we get compile-time guarantees that our event decoding matches the actual contract ABIs. The crate will also expose ICreateX factory interfaces and TrebDeploy base contract types needed by later phases (gen-deploy template generation, Phase 11).

## Goals

1. **Single source of truth**: All Solidity type definitions come from the `treb-sol` submodule, eliminating inline ABI duplication in `treb-forge`.
2. **Compile-time ABI safety**: If `treb-sol` Solidity interfaces change, `cargo build` fails immediately rather than silently producing incorrect event decoders.
3. **Zero behavioral change**: The event decoding pipeline (`decode_events`, `extract_deployments`, proxy detection) produces identical results after the migration — all existing tests pass without modification.
4. **Documented submodule workflow**: Contributors know how to clone with submodules, update `treb-sol`, and handle CI submodule checkout.

## User Stories

### P1-US-001: Add treb-sol Git Submodule

**Description**: Add the `treb-sol` Solidity repository as a git submodule at `lib/treb-sol`, pointing to `trebuchet-org/treb-sol`. This makes the Solidity source files available at build time for `sol!` macro expansion.

**Acceptance Criteria**:
- `.gitmodules` file exists with entry for `treb-sol` submodule pointing to `trebuchet-org/treb-sol`
- Submodule is checked out at `lib/treb-sol` with a pinned commit
- `git submodule update --init --recursive` succeeds from a fresh clone
- The submodule directory contains the expected Solidity interface files (at minimum: `ITrebEvents.sol`, `ICreateX.sol`)

---

### P1-US-002: Create treb-sol Workspace Crate with sol! Bindings

**Description**: Create a new `treb-sol` workspace crate (`crates/treb-sol`) that uses `alloy::sol!` to generate Rust bindings from the Solidity interface files in the `lib/treb-sol` submodule. The crate exposes three binding modules: ITrebEvents (deployment/transaction events and structs), ICreateX (factory contract events), and proxy events (ERC-1967 Upgraded/AdminChanged/BeaconUpgraded).

**Acceptance Criteria**:
- `crates/treb-sol/Cargo.toml` exists with `alloy-sol-types` and `alloy-primitives` as dependencies
- `crates/treb-sol` is listed in workspace `Cargo.toml` `members` and `workspace.dependencies`
- `sol!` macro invocations reference Solidity files from `lib/treb-sol` (not inline definitions)
- The crate publicly exports all event and struct types: `ContractDeployed`, `TransactionSimulated`, `SafeTransactionQueued`, `SafeTransactionExecuted`, `DeploymentCollision`, `GovernorProposalCreated`, `DeploymentDetails`, `SimulatedTransaction`, `Transaction`, `ContractCreation_0`, `ContractCreation_1`, `Create3ProxyContractCreation`, `Upgraded`, `AdminChanged`, `BeaconUpgraded`
- `cargo build -p treb-sol` succeeds (typecheck passes)
- Unit tests verify `SIGNATURE_HASH` is non-zero for all event types

**Notes**: If `sol!` cannot directly reference the submodule `.sol` files (e.g., due to import resolution), use `alloy::sol!` with inline Solidity that `import`s from the submodule path, or extract the relevant interface text. The key constraint is that the definitions must originate from `treb-sol` source, not be manually duplicated. If the Solidity files use imports that `sol!` cannot resolve, an acceptable fallback is to use `sol!` with the JSON ABI files from the submodule (if available) or to keep inline `sol!` definitions but add a compile-time or test-time assertion that the generated `SIGNATURE_HASH` values match the ABI files in the submodule.

---

### P1-US-003: Wire treb-forge to Use treb-sol Bindings

**Description**: Replace the inline `sol!` definitions in `crates/treb-forge/src/events/abi.rs` with re-exports from the `treb-sol` crate. Update all imports in the `events` module (`decoder.rs`, `deployments.rs`, `proxy.rs`, `mod.rs`) and the integration test to use the `treb-sol` types.

**Acceptance Criteria**:
- `treb-forge/Cargo.toml` depends on `treb-sol` (workspace dependency)
- `treb-forge/src/events/abi.rs` no longer contains any `sol!` macro invocations — all types are re-exported from `treb-sol`
- All existing unit tests in `abi.rs`, `decoder.rs` pass without modification (same type names, same behavior)
- All existing integration tests in `tests/events_integration_test.rs` pass without modification
- `cargo test -p treb-forge` passes (zero test regressions)

---

### P1-US-004: Integration Test Verifying Event Decoding Roundtrip Against treb-sol ABIs

**Description**: Add an integration test that verifies the `treb-sol` bindings produce identical event signature hashes to what the Solidity compiler would generate. This catches any drift between the Rust bindings and the canonical Solidity interfaces. The test encodes events using `treb-sol` types, decodes them through the existing `decode_events` pipeline, and asserts the roundtrip is lossless.

**Acceptance Criteria**:
- A new test (in `treb-sol` or `treb-forge` integration tests) constructs each event type from `treb-sol`, encodes it via `SolEvent::encode_log_data`, decodes it via `decode_events`, and asserts all fields match
- The test covers at minimum: `ContractDeployed`, `TransactionSimulated`, `DeploymentCollision`, `ContractCreation_0` (with salt), `Upgraded`
- If JSON ABI files exist in the `treb-sol` submodule, an additional assertion compares `SIGNATURE_HASH` values against the ABI's event signature
- `cargo test` passes (typecheck passes)

---

### P1-US-005: Document Submodule Update Workflow

**Description**: Add documentation explaining how to work with the `treb-sol` git submodule: initial clone, updating to a new version, and CI considerations. Add this to either `CONTRIBUTING.md` or `CLAUDE.md` (the project-level instructions file).

**Acceptance Criteria**:
- Documentation covers: `git clone --recurse-submodules`, `git submodule update --init`, updating the submodule to a new commit/tag
- Documentation mentions that CI workflows need `submodules: recursive` (or equivalent) in checkout steps
- Documentation is placed in `CLAUDE.md` or a new `CONTRIBUTING.md` at the repo root
- The documentation is concise (under 30 lines)

## Functional Requirements

- **FR-1**: The `treb-sol` submodule MUST point to `trebuchet-org/treb-sol` and be checked out at a specific pinned commit.
- **FR-2**: The `treb-sol` crate MUST use `alloy::sol!` to generate bindings that are sourced from (or validated against) the Solidity files in the submodule.
- **FR-3**: All event types currently defined inline in `treb-forge/src/events/abi.rs` MUST be available from `treb-sol` with identical type names and `SIGNATURE_HASH` values.
- **FR-4**: The `treb-forge` crate MUST depend on `treb-sol` and MUST NOT contain its own `sol!` event/struct definitions after migration.
- **FR-5**: The migration MUST be transparent — no changes to the public API of `treb-forge::events`, no changes to event decoding behavior, no test modifications required (beyond import paths if any are crate-internal).
- **FR-6**: The `treb-sol` crate MUST compile successfully even if the Solidity files use features not supported by `alloy::sol!` (via the fallback strategy described in P1-US-002 notes).

## Non-Goals

- **Template generation**: The `TrebDeploy` base contract bindings for `gen-deploy` template generation are exposed by this crate but not consumed until Phase 11.
- **Build-time compilation**: We do NOT compile the Solidity contracts as part of the Rust build. We only generate type bindings from interface definitions.
- **ABI JSON generation**: We do NOT generate or ship JSON ABI files from the Rust build. The submodule may contain pre-built ABI JSON, but we use `sol!` for Rust bindings.
- **Submodule auto-update**: We do NOT set up automated submodule version bumping. Updates are manual and intentional.
- **Foundry version changes**: No changes to the foundry/alloy version pinning in this phase.

## Technical Considerations

### Dependencies
- `alloy-sol-types` (workspace, v1.5) — provides the `sol!` macro
- `alloy-primitives` (workspace, v1.4.1) — provides `Address`, `B256`, `Bytes`, etc.
- No new external dependencies beyond what the workspace already uses

### sol! Macro Limitations
The `alloy::sol!` macro can accept inline Solidity or file paths. When using file paths, the macro runs at compile time and needs the files to exist relative to the crate's `Cargo.toml`. The submodule at `lib/treb-sol` is at the workspace root, so the `sol!` invocation will need a relative path like `../../lib/treb-sol/src/interfaces/ITrebEvents.sol`. If the Solidity files have `import` statements that `sol!` cannot resolve (it does not support Solidity import resolution), the fallback is to use inline `sol!` with the interface text copied, plus a test that validates signature hashes match the submodule's ABI artifacts.

### Workspace Integration
- Add `treb-sol = { path = "crates/treb-sol" }` to `[workspace.dependencies]`
- Add `"crates/treb-sol"` to `[workspace.members]`
- Add `treb-sol = { workspace = true }` to `treb-forge/Cargo.toml` dependencies

### Existing Code Impact
- `crates/treb-forge/src/events/abi.rs`: Remove all `sol!` blocks, replace with `pub use treb_sol::*` (or selective re-exports)
- `crates/treb-forge/src/events/mod.rs`: Re-export paths stay the same (they re-export from `abi`)
- `crates/treb-forge/src/events/decoder.rs`: Import paths change from `crate::events::abi::*` to still `crate::events::abi::*` (since `abi.rs` re-exports)
- No changes needed in `deployments.rs`, `proxy.rs`, or integration tests — they import from `crate::events::*` which re-exports from `abi`

### CI Considerations
- GitHub Actions checkout step needs `submodules: recursive` or a post-checkout `git submodule update --init`
- The submodule adds a network dependency to fresh clones but not to incremental builds
