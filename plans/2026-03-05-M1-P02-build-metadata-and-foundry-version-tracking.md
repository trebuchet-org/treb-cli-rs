# PRD: Phase 2 - Build Metadata and Foundry Version Tracking

## Introduction

Phase 2 extends the treb CLI's compile-time metadata to include foundry version tracking and treb-sol submodule commit information. Currently, `treb version` reports the CLI version, git commit, build date, rust version, and forge version (detected at runtime from linked foundry crates). This phase adds two new fields -- the foundry git tag pinned in Cargo.toml and the treb-sol submodule HEAD commit -- and aligns the JSON output schema with the Go CLI's `version` command (using camelCase field names). This is critical for reproducibility: users and CI systems need to know exactly which foundry release and Solidity binding commit their treb binary was built against.

Depends on Phase 1 (treb-sol submodule must be present for commit hash extraction).

## Goals

1. **Foundry version at compile time**: `build.rs` captures the foundry git tag (e.g., `v1.5.1`) as `TREB_FOUNDRY_VERSION` and exposes it alongside existing build metadata.
2. **treb-sol commit tracking**: `build.rs` captures the treb-sol submodule HEAD commit hash as `TREB_SOL_COMMIT` so users can trace which Solidity bindings were compiled in.
3. **Go-compatible JSON schema**: `treb version --json` outputs camelCase field names (`version`, `commit`, `date`, `foundryVersion`, `trebSolCommit`, `rustVersion`, `forgeVersion`) matching the Go CLI schema.
4. **Golden file parity**: Updated golden files for both human and JSON version output that normalize all dynamic fields (hashes, dates, versions).
5. **Version pinning documentation**: Workspace Cargo.toml includes a documentation comment block explaining the alloy/foundry version pinning strategy and how to update it.

## User Stories

### P2-US-001: Add Foundry Version and treb-sol Commit to build.rs

**Description**: Extend `crates/treb-cli/build.rs` to capture two new compile-time environment variables: `TREB_FOUNDRY_VERSION` (the foundry git tag from the workspace Cargo.toml comment, e.g., `v1.5.1`) and `TREB_SOL_COMMIT` (the treb-sol submodule HEAD short hash via `git -C lib/treb-sol rev-parse --short HEAD`).

**Acceptance Criteria**:
- `build.rs` emits `cargo:rustc-env=TREB_FOUNDRY_VERSION=<tag>` where `<tag>` is extracted from the foundry dependency specification in workspace Cargo.toml (parse the `tag = "v1.5.1"` value from the `foundry-config` dependency line, or hardcode the tag string if parsing is impractical -- the tag only changes when foundry is intentionally upgraded)
- `build.rs` emits `cargo:rustc-env=TREB_SOL_COMMIT=<hash>` from `git -C lib/treb-sol rev-parse --short HEAD`, falling back to `"unknown"` if the submodule is not initialized
- `build.rs` adds `cargo:rerun-if-changed=../../lib/treb-sol` so the commit hash updates when the submodule pointer changes
- `cargo build` succeeds and both env vars are accessible via `env!()` in Rust source
- Typecheck passes (`cargo check`)

**Files to modify**: `crates/treb-cli/build.rs`

---

### P2-US-002: Update VersionInfo Struct and version Command Output

**Description**: Add the two new fields (`foundry_version` as the pinned tag, `treb_sol_commit`) to the `VersionInfo` struct and update both human-readable and JSON output. Rename the existing `forge_version` field to keep it (it shows runtime-detected forge info) and add the new `foundry_version` field for the pinned tag. Update JSON serialization to use camelCase field names matching the Go CLI schema.

**Acceptance Criteria**:
- `VersionInfo` struct has fields: `version`, `commit` (renamed from `git_commit`), `date` (renamed from `build_date`), `rust_version`, `forge_version`, `foundry_version`, `treb_sol_commit`
- JSON output uses `#[serde(rename_all = "camelCase")]` producing keys: `version`, `commit`, `date`, `rustVersion`, `forgeVersion`, `foundryVersion`, `trebSolCommit`
- Human output adds rows "Foundry Version" and "treb-sol Commit" to the key-value display
- Human output renames "Git Commit" to "Commit" and "Build Date" to "Date" for consistency with JSON field names
- `treb version` runs successfully and displays all fields
- `treb version --json` produces valid JSON with all expected keys
- Typecheck passes (`cargo check`)

**Files to modify**: `crates/treb-cli/src/commands/version.rs`

---

### P2-US-003: Update Golden Files and Integration Tests

**Description**: Update the golden files for `version_human` and `version_json` to include the new fields and reflect renamed fields. Update the integration test in `cli_version.rs` to assert the new fields are present in both human and JSON output.

**Acceptance Criteria**:
- `tests/golden/version_human/commands.golden` updated with new field rows (Foundry Version, treb-sol Commit) and renamed fields (Commit, Date)
- `tests/golden/version_json/commands.golden` updated with camelCase JSON keys and new fields (`foundryVersion`, `trebSolCommit`)
- `tests/cli_version.rs` assertions updated: `version_displays_expected_fields` checks for new field labels; `version_json_parses_with_expected_fields` checks for all camelCase JSON keys
- `tests/integration_version.rs` golden file tests pass with `ShortHexNormalizer` handling the new hash fields
- All version-related tests pass (`cargo test -p treb-cli -- version`)

**Files to modify**: `crates/treb-cli/tests/golden/version_human/commands.golden`, `crates/treb-cli/tests/golden/version_json/commands.golden`, `crates/treb-cli/tests/cli_version.rs`, `crates/treb-cli/tests/integration_version.rs`

---

### P2-US-004: Document Alloy/Foundry Version Pinning Strategy in Cargo.toml

**Description**: Add a documentation comment block at the top of the workspace `Cargo.toml` (or adjacent to the existing foundry/alloy sections) explaining the version pinning strategy: why alloy crates are pinned to v1.1.1, how the foundry tag determines the alloy version, and the procedure for updating when a new foundry release is adopted.

**Acceptance Criteria**:
- Workspace `Cargo.toml` has a comment block (3-8 lines) above the `[patch.crates-io]` section or the foundry dependencies explaining:
  - Foundry is pinned to a specific git tag (currently `v1.5.1`)
  - Alloy crates are pinned to v1.1.1 via `[patch.crates-io]` because foundry v1.5.1 requires alloy 1.1.x; without pinning, cargo resolves to 1.7.x which breaks anvil/alloy-evm
  - When upgrading foundry: update the tag on all foundry dependencies, then update alloy pins to match the new foundry release's alloy requirements
- Comment references the `TREB_FOUNDRY_VERSION` build variable so future maintainers know to update `build.rs` if the extraction method changes
- `cargo check` still passes (comments don't break TOML parsing)

**Files to modify**: `Cargo.toml` (workspace root)

## Functional Requirements

- **FR-1**: `build.rs` captures `TREB_FOUNDRY_VERSION` as a compile-time environment variable containing the foundry git tag string (e.g., `"v1.5.1"`)
- **FR-2**: `build.rs` captures `TREB_SOL_COMMIT` as a compile-time environment variable containing the treb-sol submodule's short HEAD commit hash, with `"unknown"` fallback
- **FR-3**: `build.rs` triggers rebuild when `lib/treb-sol` submodule pointer changes
- **FR-4**: `treb version` human output displays all 7 fields in aligned key-value format: Version, Commit, Date, Rust Version, Forge Version, Foundry Version, treb-sol Commit
- **FR-5**: `treb version --json` outputs a JSON object with camelCase keys: `version`, `commit`, `date`, `rustVersion`, `forgeVersion`, `foundryVersion`, `trebSolCommit`
- **FR-6**: All values in JSON output are non-empty strings (never null or missing)
- **FR-7**: Golden files for version output normalize dynamic values (hashes, dates, versions) for stable cross-environment comparison

## Non-Goals

- **No runtime foundry version resolution changes**: The existing `detect_forge_version()` runtime detection in `treb-forge` remains unchanged; this phase adds a separate compile-time foundry tag field
- **No CI workflow changes**: CI configuration for submodule checkout is already documented in CONTRIBUTING.md (Phase 1); this phase does not create or modify GitHub Actions workflows (that is Phase 17)
- **No breaking changes to treb-forge crate API**: The `ForgeVersion` struct and `detect_forge_version()` function remain as-is
- **No version command flags beyond --json**: No `--short`, `--components`, or other version sub-flags
- **No automated foundry version extraction from Cargo.lock**: Parsing the foundry tag from Cargo.toml comments or hardcoding it is sufficient; no need to parse Cargo.lock metadata

## Technical Considerations

### Dependencies
- **Phase 1 (treb-sol submodule)**: Must be present for `git -C lib/treb-sol rev-parse HEAD` to succeed. Fallback to `"unknown"` handles CI environments where submodule is not initialized.
- **build.rs rerun triggers**: Adding `cargo:rerun-if-changed=../../lib/treb-sol` ensures the treb-sol commit hash updates when the submodule pointer changes (e.g., after `git submodule update`).

### Foundry Version Extraction
The foundry git tag (`v1.5.1`) is defined in the workspace Cargo.toml comment on line 21: `# Foundry -- pinned to tag v1.5.1 (commit ...)`. Two approaches:
1. **Hardcode in build.rs**: Set `TREB_FOUNDRY_VERSION=v1.5.1` directly. Simple but requires manual update when foundry is upgraded.
2. **Parse from Cargo.toml**: Read the workspace Cargo.toml and extract the `tag = "..."` value from the `foundry-config` dependency. More robust but adds file parsing to build.rs.

Either approach is acceptable. The hardcoded approach is simpler and the foundry version changes rarely (only when intentionally upgraded). The implementer should choose based on preference.

### JSON Schema Alignment
The Go CLI's `treb version --json` uses camelCase keys. The current Rust implementation uses snake_case (serde's default). Adding `#[serde(rename_all = "camelCase")]` to `VersionInfo` and renaming the struct fields to match the Go schema (`git_commit` -> `commit`, `build_date` -> `date`) achieves parity. Note: `forgeVersion` and `foundryVersion` are distinct fields -- `forgeVersion` contains the runtime-detected display string (e.g., `"forge v1.5.1 (b0a9dd9)"`), while `foundryVersion` contains just the pinned tag (e.g., `"v1.5.1"`).

### Golden File Normalization
The existing `ShortHexNormalizer` (replaces 7-10 char hex strings with `<SHORT_HASH>`) should handle the new `trebSolCommit` field. The `VersionNormalizer` in the default chain handles semver patterns. No new normalizers should be needed.

### Downstream Impact
- Phase 17 (Cross-Platform Build and Release Pipeline) depends on this phase for including foundry version and treb-sol commit in release notes
- The `VersionInfo` struct may be consumed by other commands in the future (e.g., `--version` flag on root command)
