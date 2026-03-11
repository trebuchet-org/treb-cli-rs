# PRD: Phase 10 - Register and Sync Flag Alignment

## Introduction

Phase 10 aligns the `register` and `sync` commands with Go CLI behavior for production workflows. The key gap is that the Go CLI derives network and namespace from `treb.toml` / `config.local.json` (via `resolve_config`), while the Rust CLI requires explicit `--network` / `--rpc-url` flags on `register` and has minor help text differences on `sync --clean`. This phase adds config-driven fallback for network/namespace on `register`, aligns the `sync --clean` description with Go, and adds integration test coverage for both commands working in Go-created project configurations.

## Goals

1. **Register config fallback**: `treb register --tx-hash 0x...` works without `--network` when the project has a configured network in `treb.toml` or `config.local.json`, matching Go behavior.
2. **Register namespace fallback**: `treb register` derives namespace from config when `--namespace` is not provided, matching Go's config-driven namespace.
3. **Sync help parity**: `sync --clean` help description matches Go: "Remove invalid entries while syncing".
4. **Production workflow verification**: Both commands work correctly in Go-created projects (using `treb.toml` with `${VAR}` RPC endpoints and `.env` files).

## User Stories

### P10-US-001: Register Config-Driven Network Fallback

**Summary:** When `--network` and `--rpc-url` are both omitted on `register`, fall back to the configured network from `resolve_config()` instead of erroring.

**Changes:**
- In `crates/treb-cli/src/commands/register.rs`, update `run()` to call `treb_config::resolve_config()` when neither `--network` nor `--rpc-url` is provided. Use the resolved config's `network` field to look up the RPC endpoint from `foundry.toml [rpc_endpoints]`.
- Keep the existing `resolve_rpc_url()` path for when explicit flags are given (explicit flags override config).
- When config has no network set and no flags provided, error with a message matching Go: "no active network set in config, --network flag is required".

**Acceptance Criteria:**
- `treb register --tx-hash 0x... --network anvil-31337` still works (explicit flag path unchanged).
- `treb register --tx-hash 0x... --rpc-url http://...` still works (explicit URL path unchanged).
- `treb register --tx-hash 0x...` (no network flag) resolves network from `treb.toml` config when available.
- `treb register --tx-hash 0x...` errors clearly when config has no network and no flag is provided.
- `resolve_rpc_endpoints` is called with dotenv already loaded (via `resolve_config` or `resolve_rpc_endpoints` which calls `load_dotenv` internally).
- Typecheck passes: `cargo check -p treb-cli`.
- Unit test for fallback logic in `register.rs`.

### P10-US-002: Register Config-Driven Namespace Fallback

**Summary:** When `--namespace` is omitted on `register`, fall back to the configured namespace from `resolve_config()` instead of defaulting to `"default"`.

**Changes:**
- In `crates/treb-cli/src/commands/register.rs`, when `namespace` is `None`, use the namespace from `resolve_config()` instead of hardcoded `"default"`. This makes `register` respect `treb.toml` namespace settings like Go does.
- If `resolve_config()` was already called for network fallback (P10-US-001), reuse the same resolved config to extract namespace.
- If config resolution fails or returns `"default"`, the behavior is the same as today.

**Acceptance Criteria:**
- `treb register --namespace production ...` still overrides config namespace.
- `treb register --tx-hash 0x... --network anvil-31337` (no `--namespace`) uses the namespace from config resolution, or `"default"` if no config exists.
- Generated deployment IDs use the config-derived namespace when `--namespace` is not provided.
- Typecheck passes: `cargo check -p treb-cli`.
- Unit test for namespace resolution priority: explicit flag > config > "default".

### P10-US-003: Sync Clean Flag Description and Help Alignment

**Summary:** Align the `sync --clean` help text and the `sync` command `about` text with Go CLI.

**Changes:**
- In `crates/treb-cli/src/main.rs`, update the `Sync` variant's `clean` field help from `"Remove safe transactions not found on the service"` to `"Remove invalid entries while syncing"` to match Go.
- In `crates/treb-cli/build.rs`, update the `build_sync()` function's `clean` arg help to match.
- Refresh affected golden help snapshots.

**Acceptance Criteria:**
- `treb sync --help` shows `--clean` with description "Remove invalid entries while syncing".
- `build.rs` and `main.rs` descriptions match.
- Golden help snapshots updated: `help_sync` (if it exists) or root help if sync appears there.
- Typecheck passes: `cargo check -p treb-cli`.

### P10-US-004: Integration Tests for Config-Driven Register and Sync

**Summary:** Add integration tests verifying `register` and `sync` work correctly in Go-style project configurations where network comes from config rather than explicit flags.

**Changes:**
- In `crates/treb-cli/tests/integration_register.rs`, add tests that:
  - Set up a project with `treb.toml` config containing a network (via `pre_setup_hook`), then run `register --tx-hash 0x...` without `--network` — verify it succeeds using the config network.
  - Set up a project with no network in config and no `--network` flag — verify the error message.
  - Set up a project with a custom namespace in config — verify the deployment ID uses that namespace.
- In `crates/treb-cli/tests/integration_sync.rs`, add a test verifying `sync --clean` flag acceptance and help text.
- In `crates/treb-cli/tests/cli_compatibility_aliases.rs`, add register/sync entries verifying flag acceptance parity.

**Acceptance Criteria:**
- At least one test proves `register` resolves network from config (Anvil-backed, similar to existing `register_basic` pattern).
- At least one test proves `register` errors correctly when config has no network and no flag.
- At least one test proves namespace comes from config when `--namespace` is omitted.
- Sync clean flag test passes.
- All existing register and sync tests continue to pass.
- `cargo test -p treb-cli --test integration_register` passes.
- `cargo test -p treb-cli --test integration_sync` passes.

## Functional Requirements

**FR-1:** `register` resolves network from `resolve_config()` when neither `--network` nor `--rpc-url` is explicitly provided.

**FR-2:** `register` resolves namespace from `resolve_config()` when `--namespace` is not explicitly provided, falling back to `"default"` only when config has no namespace.

**FR-3:** Explicit `--network`, `--rpc-url`, and `--namespace` flags always take precedence over config-derived values.

**FR-4:** `register` without config network and without explicit network flag produces a clear error matching Go's wording.

**FR-5:** `sync --clean` help text reads "Remove invalid entries while syncing" in both `main.rs` and `build.rs`.

**FR-6:** Both `register` and `sync` work in projects with Go-style `treb.toml` and `.env` files containing `${VAR}` RPC endpoints.

**FR-7:** `register` calls `load_dotenv()` (either directly or via `resolve_config()` / `resolve_rpc_endpoints()`) before resolving config, ensuring `.env`-backed RPC endpoints expand correctly.

## Non-Goals

- **Interactive prompting for register**: Go's `register` has interactive multi-contract selection and per-contract prompts. This is a larger feature not covered by flag alignment.
- **Fork mode detection for register/sync**: Go blocks `sync` in fork mode and adds a note for `register`. This is a behavioral difference not in the flag alignment scope.
- **Removing `--network` from sync**: Rust's `sync --network` filter is an intentional extension over Go (which has no filter). It stays as-is.
- **Removing `--rpc-url` from register**: Rust's `--rpc-url` is an intentional extension that provides a direct override path. It stays as-is.
- **Adding `--deployment-type` to Go**: Rust's `--deployment-type` flag on `register` is an intentional extension. No changes needed.

## Technical Considerations

**Config resolution approach:** The simplest path is to call `treb_config::resolve_config()` at the top of `register::run()` when no explicit network/namespace flags are provided. This gives both network and namespace in one call and already handles dotenv loading. The resolved config's `network` field maps to the `foundry.toml [rpc_endpoints]` key.

**Existing `resolve_rpc_url` refactoring:** The current `resolve_rpc_url()` function requires either `--rpc-url` or `--network`. With config fallback, the network comes from a third source. The cleanest approach is to add a `config_network: Option<String>` parameter or resolve it before calling the function, inserting the config-derived network as the fallback when the explicit flag is `None`.

**`resolve_rpc_endpoints` already loads dotenv:** `treb_config::resolve_rpc_endpoints()` calls `load_dotenv()` internally (confirmed in `foundry.rs:142`), so the RPC URL path already handles `.env` expansion. The config-driven path via `resolve_config()` also loads dotenv. No extra dotenv call is needed.

**Test fixture pattern:** Config-driven register tests need a project with both `foundry.toml` (with `[rpc_endpoints]`) and `treb.toml` (with a network set). Use `pre_setup_hook` to write these files before `treb init`, following the pattern from Phase 3's `config_show_resolves_dotenv_sender_address` golden test. The Anvil RPC URL can be injected into the `.env` file.

**Backward compatibility:** All changes are additive — existing flag-driven workflows are completely unchanged. The new config fallback only activates when flags are absent.
