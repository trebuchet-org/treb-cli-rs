# PRD: Phase 1 - Repository Scaffold and Workspace Layout

## Introduction

Phase 1 bootstraps the Rust workspace for `treb-cli-rs`, a full reimplementation
of treb-cli (Go) built directly on foundry's Rust crates. This phase establishes
the foundry dependency chain, async runtime, error handling strategy, CLI skeleton,
and CI pipeline. Every subsequent phase builds on this foundation — getting the
foundry crate integration right here means all later phases work with real foundry
types from day one.

This is the first phase. There are no prior phases or existing code.

## Goals

1. **Compiling workspace with foundry crates**: `cargo check` succeeds with
   `foundry-config` and `foundry-common` as dependencies, proving the foundry
   integration works end-to-end.
2. **Working CLI entry point**: `cargo run -- --help` prints help text with
   version info and stub subcommands via clap.
3. **Established error and async patterns**: `treb-core` exports a `TrebError`
   enum via `thiserror`, and the binary runs on a `tokio` async runtime — setting
   the pattern for all future code.
4. **Alloy type alignment**: `Address`, `B256`, and `U256` are re-exported from
   `treb-core` at the exact versions foundry uses, preventing version conflicts
   in later phases.
5. **Green CI**: GitHub Actions runs `cargo check`, `cargo test`, `cargo clippy`,
   and `cargo fmt --check` on every push and PR.

## User Stories

### US-001: Cargo Workspace and Toolchain Configuration

**Description**: Create the Cargo workspace with two initial crates (`treb-cli`
binary and `treb-core` library) and all toolchain/linting configuration files.

**Changes**:
- Root `Cargo.toml` with `[workspace]` containing members `crates/treb-cli` and
  `crates/treb-core`
- `crates/treb-cli/Cargo.toml` — binary crate, depends on `treb-core`
- `crates/treb-cli/src/main.rs` — minimal `fn main()` that prints "treb"
- `crates/treb-core/Cargo.toml` — library crate
- `crates/treb-core/src/lib.rs` — empty lib root
- `rust-toolchain.toml` — pin to `nightly-2025-02-14` or later nightly that
  matches foundry's minimum (edition 2024, Rust 1.89+). Use nightly channel since
  foundry's rustfmt config requires nightly features.
- `.gitignore` — add `/target`, `Cargo.lock` (workspace is a library+binary,
  include lock file actually — binary crate means lock should be committed), `.env`
- `rustfmt.toml` — match foundry: `max_width = 100`, `imports_granularity = "Crate"`,
  `use_field_init_shorthand = true`, `use_small_heuristics = "Max"`,
  `wrap_comments = true`, `format_code_in_doc_comments = true`,
  `comment_width = 100`, `doc_comment_code_block_width = 100`
- `clippy.toml` — start minimal: `ignore-interior-mutability` for alloy bytes types

**Acceptance Criteria**:
- [ ] `cargo build` succeeds from workspace root
- [ ] `cargo run -p treb-cli` prints output and exits 0
- [ ] `cargo fmt --check` passes
- [ ] `cargo clippy --workspace` passes with no warnings
- [ ] `rust-toolchain.toml` specifies a nightly toolchain compatible with edition 2024
- [ ] All config files (`rustfmt.toml`, `clippy.toml`, `.gitignore`) exist at workspace root

---

### US-002: Foundry Crate Integration and Alloy Re-exports

**Description**: Add `foundry-config` and `foundry-common` as git dependencies
pinned to a specific foundry commit. Create alloy primitive re-exports in
`treb-core` so downstream crates use foundry-aligned types.

**Changes**:
- Root `Cargo.toml` `[workspace.dependencies]` — add `foundry-config` and
  `foundry-common` as git deps from `https://github.com/foundry-rs/foundry` with
  a pinned `rev`. Also add `alloy-primitives` at version `1.5.2` (matching foundry).
- `crates/treb-core/Cargo.toml` — depend on `foundry-config`, `foundry-common`,
  `alloy-primitives`
- `crates/treb-core/src/lib.rs` — add `pub mod primitives;`
- `crates/treb-core/src/primitives.rs` — re-export `Address`, `B256`, `U256`
  from `alloy_primitives`
- Pin to a recent foundry release tag or commit (e.g., the latest stable release).
  Document the pinned commit in a comment in root `Cargo.toml`.

**Acceptance Criteria**:
- [ ] `cargo check --workspace` succeeds with foundry crates resolving
- [ ] `treb_core::primitives::Address`, `treb_core::primitives::B256`, and
      `treb_core::primitives::U256` are importable and usable
- [ ] A simple test in `treb-core` constructs an `Address::ZERO` and asserts it
      is zero
- [ ] `foundry_config::Config::load()` is callable (import compiles, no runtime test yet)
- [ ] The specific foundry git commit is documented in `Cargo.toml`
- [ ] `cargo clippy --workspace` passes

---

### US-003: Error Strategy and Async Runtime

**Description**: Establish the project-wide error handling pattern using
`thiserror` in `treb-core` and `anyhow` in `treb-cli`. Wire up the `tokio`
async runtime as the entry point.

**Changes**:
- Add `thiserror` and `anyhow` to `[workspace.dependencies]`
- Add `tokio` to `[workspace.dependencies]` with features `["full"]`
- `crates/treb-core/Cargo.toml` — depend on `thiserror`
- `crates/treb-core/src/error.rs` — define `TrebError` enum with variants:
  - `Config(String)` — configuration errors
  - `Registry(String)` — registry errors
  - `Forge(String)` — forge integration errors
  - `Io(#[from] std::io::Error)` — IO errors
  - Each variant derives `thiserror::Error` with display strings
- `crates/treb-core/src/lib.rs` — add `pub mod error;` and
  `pub type Result<T> = std::result::Result<T, error::TrebError>;`
- `crates/treb-cli/Cargo.toml` — depend on `anyhow`, `tokio`
- `crates/treb-cli/src/main.rs` — change to `#[tokio::main] async fn main() -> anyhow::Result<()>`

**Acceptance Criteria**:
- [ ] `treb_core::error::TrebError` is a public enum with at least `Config`,
      `Registry`, `Forge`, and `Io` variants
- [ ] `TrebError` implements `std::error::Error` and `Display` (via thiserror)
- [ ] `treb_core::Result<T>` is a public type alias using `TrebError`
- [ ] `std::io::Error` converts into `TrebError` via `From`
- [ ] `treb-cli` main function is `async` and returns `anyhow::Result<()>`
- [ ] A unit test in `treb-core` creates a `TrebError::Config` and formats it
- [ ] `cargo test --workspace` passes
- [ ] `cargo clippy --workspace` passes

---

### US-004: CLI Skeleton with Clap

**Description**: Build the clap-based CLI skeleton with version/help output and
stub subcommands for all planned treb commands. Subcommands print "not yet
implemented" and exit.

**Changes**:
- Add `clap` to `[workspace.dependencies]` with features `["derive"]`
- `crates/treb-cli/Cargo.toml` — depend on `clap`
- `crates/treb-cli/src/cli.rs` — define `Cli` struct with `#[derive(Parser)]`:
  - `#[command(name = "treb", version, about = "Smart contract deployment toolkit")]`
  - `#[command(subcommand)] command: Commands` enum
- `Commands` enum with stub variants: `Run`, `List` (alias `Ls`), `Show`, `Init`,
  `Config`, `Verify`, `Tag`, `Register`, `Sync`, `Version`, `Networks`, `GenDeploy`,
  `Compose`, `Prune`, `Reset`, `Migrate`, `Fork`, `Dev`
- Each variant is an empty struct for now (no fields)
- `crates/treb-cli/src/main.rs` — parse CLI args and match on command, printing
  `"treb <command>: not yet implemented"` for each subcommand
- Wire `Cli::parse()` into the async main

**Acceptance Criteria**:
- [ ] `cargo run -p treb-cli -- --help` prints help with all subcommands listed
- [ ] `cargo run -p treb-cli -- --version` prints version string
- [ ] `cargo run -p treb-cli -- run` prints "not yet implemented" message
- [ ] `cargo run -p treb-cli -- list` and `cargo run -p treb-cli -- ls` both work (alias)
- [ ] All planned subcommands are present in the `Commands` enum
- [ ] `cargo clippy --workspace` passes
- [ ] `cargo test --workspace` passes

---

### US-005: GitHub Actions CI and README

**Description**: Set up the CI pipeline and a placeholder README with build
instructions so contributors can get started.

**Changes**:
- `.github/workflows/ci.yml` — GitHub Actions workflow triggered on push and PR:
  - Job `check`: `cargo check --workspace`
  - Job `test`: `cargo test --workspace`
  - Job `clippy`: `cargo clippy --workspace -- -D warnings`
  - Job `fmt`: `cargo fmt --all -- --check`
  - Use `actions-rust-lang/setup-rust-toolchain@v1` with the toolchain from
    `rust-toolchain.toml`
  - Cache cargo registry and target dir via `Swatinem/rust-cache@v2`
- `README.md` — placeholder with:
  - Project name and one-line description
  - Prerequisites (Rust nightly, foundry knowledge)
  - Build instructions (`cargo build`)
  - Run instructions (`cargo run -p treb-cli -- --help`)
  - Test instructions (`cargo test --workspace`)
  - Note that this is a work-in-progress reimplementation

**Acceptance Criteria**:
- [ ] `.github/workflows/ci.yml` exists and is valid YAML
- [ ] CI workflow defines jobs for check, test, clippy, and fmt
- [ ] CI workflow uses rust-cache for faster builds
- [ ] `README.md` exists with build, run, and test instructions
- [ ] `cargo fmt --check` passes (README doesn't affect this, but ensures no
      regressions)
- [ ] `cargo clippy --workspace` passes

## Functional Requirements

- **FR-1**: The workspace root `Cargo.toml` defines a workspace with members
  `crates/treb-cli` and `crates/treb-core`, using shared `[workspace.dependencies]`.
- **FR-2**: `treb-cli` is a binary crate; `treb-core` is a library crate.
- **FR-3**: `foundry-config` and `foundry-common` compile as dependencies of
  `treb-core`, pinned to a specific foundry git commit.
- **FR-4**: `alloy-primitives` version matches foundry's pinned version (1.5.2).
  `Address`, `B256`, `U256` are re-exported from `treb-core::primitives`.
- **FR-5**: `treb-core` defines a `TrebError` enum using `thiserror` with at least
  `Config`, `Registry`, `Forge`, and `Io` variants.
- **FR-6**: `treb-cli` uses `#[tokio::main]` for its async entry point and returns
  `anyhow::Result<()>`.
- **FR-7**: The CLI uses `clap` derive API with subcommands for all planned treb
  commands. Unimplemented commands print a stub message.
- **FR-8**: `rust-toolchain.toml` pins a nightly toolchain compatible with Rust
  edition 2024 and foundry's requirements.
- **FR-9**: `rustfmt.toml` uses 100-char line width and crate-level import
  granularity (matching foundry).
- **FR-10**: GitHub Actions CI runs check, test, clippy, and fmt on every push/PR.

## Non-Goals

- **No domain types**: `Deployment`, `Transaction`, `Contract`, and other domain
  structs are deferred to Phase 2.
- **No configuration parsing**: `treb.toml`, `.env` loading, and config merging
  are Phase 3.
- **No registry system**: `.treb/` directory structure and JSON storage are Phase 4.
- **No forge script execution**: In-process compilation and script running are
  Phase 5.
- **No real command implementations**: All subcommands are stubs. Actual logic
  starts in Phase 9+.
- **No cross-compilation or release builds**: Release packaging is Phase 20.
- **No foundry submodule**: Use git dependencies with pinned rev rather than a
  submodule to keep the repo lightweight.

## Technical Considerations

### Foundry Dependency Strategy

Use `git` dependencies in `[workspace.dependencies]` pointing to
`https://github.com/foundry-rs/foundry` with a pinned `rev`. This avoids the
complexity of a git submodule while still locking to an exact commit. The pinned
commit should be a recent release tag (e.g., the latest nightly or stable release).

Example:
```toml
[workspace.dependencies]
foundry-config = { git = "https://github.com/foundry-rs/foundry", rev = "<commit>" }
foundry-common = { git = "https://github.com/foundry-rs/foundry", rev = "<commit>" }
```

### Rust Toolchain

Foundry uses edition 2024 and requires Rust 1.89+. Their `rustfmt.toml` uses
nightly-only features (`wrap_comments`, `format_code_in_doc_comments`,
`imports_granularity`, `format_macro_matchers`). Use a nightly toolchain.

### Compile Times

Foundry pulls in a large dependency tree (revm, alloy, solar, etc.). First build
will be slow. CI should use `Swatinem/rust-cache` aggressively. Local development
benefits from `sccache`.

### Workspace Layout

```
treb-cli-rs/
  Cargo.toml              # workspace root
  Cargo.lock
  rust-toolchain.toml
  rustfmt.toml
  clippy.toml
  .gitignore
  README.md
  .github/workflows/ci.yml
  crates/
    treb-cli/
      Cargo.toml
      src/
        main.rs
        cli.rs
    treb-core/
      Cargo.toml
      src/
        lib.rs
        error.rs
        primitives.rs
```

Future phases will add crates under `crates/` (e.g., `treb-config`, `treb-registry`,
`treb-forge`, `treb-verify`, `treb-safe`).
