# PRD: Phase 17 - Cross-Platform Build and Release Pipeline

## Introduction

Phase 17 completes the CI/CD pipeline for treb-cli-rs so that every push and PR is validated, and tagged releases produce multi-platform binaries that users can install as a drop-in replacement for the Go CLI. The project already has four GitHub Actions workflows (`ci.yml`, `release-build.yml`, `release.yml`, `foundry-track.yml`), an installation script (`scripts/trebup`), and build metadata embedding (`build.rs`). However, these workflows have gaps: CI is missing submodule checkout and foundry installation (so tests that need treb-sol or anvil fail), the release build is also missing submodules, binary archive naming uses Rust target triples instead of the Go-compatible `treb-{os}-{arch}` convention, release notes lack foundry/treb-sol version metadata, and shell completions are not included in release artifacts. This phase closes all those gaps to produce a fully working build-and-release pipeline.

## Goals

1. **CI validates all code on every push/PR** — `cargo check`, `cargo test`, `cargo clippy`, `cargo fmt` all pass in CI including tests that require the treb-sol submodule and anvil (foundry).
2. **Tagged releases produce correct multi-platform binaries** — Linux (x86_64, aarch64 musl) and macOS (x86_64, aarch64) binaries are built, named with the Go-compatible `treb-{os}-{arch}` convention, and published as GitHub release assets with SHA256 checksums.
3. **Release notes include build metadata** — Each GitHub release includes the foundry version, treb-sol commit, and Rust version extracted from the built binary.
4. **Shell completions ship with releases** — Bash, Zsh, Fish, and Elvish completion scripts are included in each platform archive.
5. **One-liner installation works end-to-end** — `scripts/trebup` installs the correct platform binary with checksum verification, using the actual GitHub repository coordinates.

## User Stories

### P17-US-001: Fix CI Workflow for Submodules and Foundry

**Description:** Update `.github/workflows/ci.yml` so that all four jobs (check, test, clippy, fmt) use `submodules: recursive` on checkout, and the `test` job installs foundry (anvil) so E2E and integration tests that spawn anvil nodes can run. Add `--all-targets` to the test job to include integration tests and doc tests.

**Acceptance Criteria:**
- All `actions/checkout@v4` steps in `ci.yml` include `submodules: recursive`
- The `test` job installs foundry via `foundry-rs/foundry-toolchain` action (or equivalent) before running tests
- The `test` job runs `cargo test --workspace --all-targets` to include integration tests
- `cargo check --workspace` passes locally after the workflow changes (no Cargo.toml regressions)
- The workflow file is valid YAML (parseable without errors)

**Files to modify:**
- `.github/workflows/ci.yml`

---

### P17-US-002: Fix Release Build Workflow — Submodules and Binary Naming

**Description:** Update `.github/workflows/release-build.yml` to checkout with `submodules: recursive` (required for treb-sol bindings at compile time) and rename the archive naming convention from `treb-{version}-{target}` to `treb-{os}-{arch}` matching the Go CLI distribution (e.g., `treb-linux-amd64`, `treb-darwin-arm64`). Add an `os_arch` field to each matrix entry to map Rust target triples to Go-style platform names. Include shell completion scripts in the archive alongside the binary.

**Acceptance Criteria:**
- All checkout steps use `submodules: recursive`
- Matrix includes an `os_arch` field: `linux-amd64`, `linux-arm64`, `darwin-amd64`, `darwin-arm64`
- Archives are named `treb-{tag}-{os_arch}.tar.gz` (e.g., `treb-v0.1.0-linux-amd64.tar.gz`)
- Checksum files match the new archive naming
- Each archive contains: `treb` binary, `completions/` directory with bash, zsh, fish, elvish scripts
- Shell completions are copied from the build output directory (`target/{target}/release/build/treb-cli-*/out/completions/`)
- The workflow file is valid YAML

**Files to modify:**
- `.github/workflows/release-build.yml`

---

### P17-US-003: Enhance Release Workflow with Structured Release Notes

**Description:** Update `.github/workflows/release.yml` to produce structured release notes that include foundry version, treb-sol commit, Rust version, and a consolidated SHA256 checksum file. Instead of relying solely on `--generate-notes`, build a release body that combines auto-generated notes with a build metadata section. Download a built binary, run `treb version --json` to extract metadata, and include it in the release body. Also produce a single `checksums.txt` file aggregating all per-platform checksums.

**Acceptance Criteria:**
- Release body includes a "Build Info" section with: foundry version, treb-sol commit, Rust toolchain version
- A consolidated `checksums.txt` file is uploaded as a release asset containing all platform SHA256 checksums
- Per-platform `.sha256` files are still uploaded individually
- Auto-generated release notes (from git log) are preserved in the body
- The release title follows format `treb {tag}` (e.g., `treb v0.1.0`)
- The workflow file is valid YAML

**Files to modify:**
- `.github/workflows/release.yml`

---

### P17-US-004: Finalize Installation Script

**Description:** Update `scripts/trebup` to use the correct GitHub repository coordinates, align archive naming with the new `treb-{tag}-{os_arch}` convention from P17-US-002, and add shell completion installation support. The script should detect the user's shell and offer to install completions to the appropriate directory. Add a `--help` flag and version display.

**Acceptance Criteria:**
- `TREB_REPO` default matches the actual GitHub organization/repo (update from placeholder `treb-rs/treb-cli-rs` if needed, or confirm it is correct)
- Archive name construction uses `treb-{version}-{os_arch}.tar.gz` format matching the release build output
- Platform detection maps to `linux-amd64`, `linux-arm64`, `darwin-amd64`, `darwin-arm64` (matching P17-US-002 `os_arch` values)
- Checksum file name matches the new archive naming convention
- After installing the binary, the script extracts and installs shell completions to the standard location for the detected shell (bash: `~/.local/share/bash-completion/completions/`, zsh: first writable `$fpath` dir or `~/.zsh/completions/`, fish: `~/.config/fish/completions/`) if the `completions/` directory is present in the archive
- Running `trebup --help` prints usage information
- The script remains POSIX sh compatible (no bashisms)
- The script works correctly on both Linux and macOS (verified by reading the logic; no live test required)

**Files to modify:**
- `scripts/trebup`

---

### P17-US-005: Add Cross.toml and CI Validation Test

**Description:** Add a `Cross.toml` configuration file for the `cross` tool used in Linux musl builds, ensuring any native dependencies (OpenSSL via vendored feature, etc.) build correctly in the cross-compilation container. Add a simple CI smoke test that builds the binary for the host target and verifies `treb version --json` produces valid JSON with all expected metadata fields. This validates the full build chain (submodule checkout, build.rs metadata, binary execution) in one assertion.

**Acceptance Criteria:**
- `Cross.toml` exists at the workspace root with appropriate configuration for musl targets (at minimum, set the Docker image or environment variables needed for a clean musl build)
- If no special cross configuration is needed (the default works), document this in a comment in the file and keep it minimal
- A new CI job `smoke` is added to `ci.yml` that runs after the `test` job, builds the binary (`cargo build --bin treb`), and executes `./target/debug/treb version --json` to verify output contains `version`, `commit`, `foundryVersion`, `trebSolCommit` fields
- The smoke test validates that the JSON is parseable and all metadata fields are non-empty strings (not `"unknown"`)
- `cargo check --workspace` passes after changes

**Files to modify:**
- `Cross.toml` (new file)
- `.github/workflows/ci.yml`

## Functional Requirements

- **FR-1:** CI workflow checks out treb-sol submodule recursively on all jobs.
- **FR-2:** CI test job installs foundry/anvil so integration tests requiring live anvil nodes can execute.
- **FR-3:** Release build workflow checks out treb-sol submodule recursively.
- **FR-4:** Release archives use Go-compatible naming: `treb-{tag}-{os}-{arch}.tar.gz` where os is `linux` or `darwin` and arch is `amd64` or `arm64`.
- **FR-5:** Each release archive contains the `treb` binary and a `completions/` directory with shell completion scripts for bash, zsh, fish, and elvish.
- **FR-6:** SHA256 checksums are generated per-platform and also aggregated into a single `checksums.txt` release asset.
- **FR-7:** GitHub release body includes foundry version, treb-sol commit, and Rust version extracted from the built binary.
- **FR-8:** The `trebup` installation script downloads the correct platform archive, verifies the SHA256 checksum, installs the binary, and optionally installs shell completions.
- **FR-9:** Cross-compilation for Linux musl targets is configured via `Cross.toml`.
- **FR-10:** A CI smoke test validates that the built binary produces correct `treb version --json` output with all metadata fields populated.

## Non-Goals

- **Windows support** — Not part of the Go CLI distribution model; can be added later if needed.
- **Homebrew formula or other package manager integration** — Future work; this phase covers direct binary distribution only.
- **Docker image builds** — Not in scope; users install the native binary.
- **Automated changelog generation** — Phase 18 covers `CHANGELOG.md`; this phase uses GitHub's auto-generated release notes.
- **Code signing or notarization** — macOS notarization and binary signing are desirable but out of scope for the initial release pipeline.
- **Nix flake or other declarative packaging** — Out of scope.

## Technical Considerations

### Dependencies
- **Phase 2 (Build Metadata):** `build.rs` already embeds `TREB_FOUNDRY_VERSION`, `TREB_SOL_COMMIT`, `TREB_GIT_COMMIT`, `TREB_BUILD_DATE`, `TREB_RUST_VERSION` — release notes and smoke test depend on this.
- **Phase 1 (treb-sol Submodule):** `.gitmodules` is configured; CI just needs `submodules: recursive` on checkout.

### Constraints
- **Nightly Rust toolchain** — `rust-toolchain.toml` specifies nightly; CI and release builds must use nightly.
- **Alloy/Foundry version pinning** — Workspace `Cargo.toml` pins alloy crates to v1.1.1 via `[patch.crates-io]`; `cargo check` in CI validates pin integrity.
- **Cross tool for Linux builds** — `cross` requires Docker; GitHub Actions ubuntu runners have Docker pre-installed.
- **macOS runners** — `macos-latest` on GitHub Actions is Apple Silicon (arm64); x86_64 builds use `rustup target add` for cross-compilation on the same runner.
- **Shell completions location** — Generated by `build.rs` into `$OUT_DIR/completions/`; the release build step must locate this directory within the target build artifacts (`target/{target}/release/build/treb-cli-*/out/completions/`).

### Integration Points
- **`treb version --json`** — Used by the release workflow to extract build metadata for release notes; schema is already defined and tested in Phase 2.
- **`foundry-track.yml`** — Existing workflow auto-detects foundry version bumps and creates PRs; no changes needed but the CI fixes (submodule checkout) will allow its validation steps to pass correctly.
- **`scripts/trebup`** — Must align archive naming with `release-build.yml` matrix changes; both must agree on the `{os}-{arch}` mapping.
