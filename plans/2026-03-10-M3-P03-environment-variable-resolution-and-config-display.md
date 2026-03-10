# PRD: Phase 3 - Environment Variable Resolution and Config Display

## Introduction

The Rust CLI fails to resolve `${VAR}` patterns in foundry.toml RPC endpoints and sender configuration fields, causing the `networks` command to report "unresolved env var" for every network and `config show` to display empty sender addresses. The Go CLI resolves these by loading `.env` / `.env.local` files and expanding `${VAR}` patterns in foundry.toml values before any command uses them.

This phase adds env var expansion to foundry.toml RPC endpoints (and foundry.toml sender fields), wires the resolved endpoints into the `networks` command so chain IDs resolve correctly, and updates `config show` to display resolved sender addresses in an inline format matching the Go CLI.

## Goals

1. **Env var expansion in foundry.toml values**: `${VAR}` patterns in `[rpc_endpoints]`, `[etherscan]`, and `[profile.*.treb.senders.*]` sections are expanded from environment variables (loaded from `.env` / `.env.local`).
2. **Networks command resolves chain IDs**: `treb networks` successfully resolves and displays chain IDs for networks whose RPC URLs use `${VAR}` patterns, matching Go CLI output.
3. **Config show displays resolved sender addresses**: `treb config show` displays populated sender addresses instead of empty strings when addresses are defined via env vars in foundry.toml senders.
4. **Config show sender format matches Go**: Sender display uses inline `role  type  address` rows instead of a comfy_table box.
5. **JSON output includes chain IDs**: `treb networks --json` includes resolved `chainId` fields when env vars are set.

## User Stories

### P3-US-001: Add `${VAR}` Expansion to foundry.toml RPC Endpoints

**Description**: The `rpc_endpoints()` function in `treb-config` returns RPC URLs as-is from foundry.toml. Add env var expansion so that `${VAR}` patterns are resolved using `std::env::var()` (which reads values set by `load_dotenv()`). This is the foundational change that unblocks the `networks` command.

**Files to modify**:
- `crates/treb-config/src/foundry.rs` — expand env vars in `rpc_endpoints()` return values

**Acceptance criteria**:
- `rpc_endpoints()` returns resolved URLs when env vars are set (e.g., `${MAINNET_RPC_URL}` becomes `https://eth-mainnet.example.com`)
- Unset env vars expand to empty string (matching existing `expand_env_vars()` behavior)
- URLs without `${VAR}` patterns pass through unchanged
- Reuses the existing `expand_env_vars()` function from `trebfile.rs` (already exported at crate root)
- Unit tests: RPC URL with `${VAR}` expands correctly; URL without vars is unchanged; mixed URL with partial vars expands correctly
- `cargo check -p treb-config` passes
- `cargo test -p treb-config` passes

### P3-US-002: Add `${VAR}` Expansion to foundry.toml Sender and Etherscan Configs

**Description**: The Go CLI expands env vars in foundry.toml `[profile.*.treb.senders.*]` fields (address, private_key, safe, etc.) and `[etherscan]` fields (url, key). The Rust `extract_treb_senders_from_foundry()` function currently reads these as raw TOML strings without expansion. Apply `expand_sender_config_env_vars()` to each extracted sender config.

Note: treb.toml v2 account configs already get env var expansion in `load_treb_config_v2()`. This story covers only the foundry.toml fallback path.

**Files to modify**:
- `crates/treb-config/src/foundry.rs` — call `expand_sender_config_env_vars()` on each sender in `extract_treb_senders_from_foundry()`

**Acceptance criteria**:
- Sender configs extracted from foundry.toml have `${VAR}` patterns expanded in all string fields (address, private_key, safe, signer, derivation_path, governor, timelock, proposer)
- Unit test: foundry.toml with `address = "${MY_ADDR}"` resolves to the env var value
- Unit test: foundry.toml without env vars is unchanged
- Existing tests continue to pass (they use literal values, no env vars)
- `cargo check -p treb-config` passes
- `cargo test -p treb-config` passes

### P3-US-003: Wire `.env` Loading into the `networks` Command

**Description**: The `networks` command currently loads foundry.toml directly without calling `load_dotenv()` first. As a result, env vars defined only in `.env` / `.env.local` are not available when RPC URLs are expanded. Call `load_dotenv()` before loading foundry config, matching the Go CLI's behavior where `.env` files are loaded before any foundry.toml parsing.

**Files to modify**:
- `crates/treb-cli/src/commands/networks.rs` — call `treb_config::load_dotenv()` before `load_foundry_config()`

**Acceptance criteria**:
- `treb networks` resolves chain IDs for networks whose RPC URLs use `${VAR}` when the var is defined in `.env`
- The `has_unresolved_env_vars()` check no longer triggers for URLs that have been expanded
- URLs that remain unresolved (env var not in `.env` and not in environment) still show "unresolved env var" status
- Integration test: project with `.env` containing `TEST_RPC_URL=http://localhost:8545` and foundry.toml with `test = "${TEST_RPC_URL}"` — `networks` shows the resolved URL (does not show "unresolved env var")
- `cargo check -p treb-cli` passes
- `cargo test -p treb-cli` passes

### P3-US-004: Update `config show` to Display Resolved Sender Addresses with Inline Format

**Description**: Currently `config show` renders senders using a `comfy_table` box (`build_table`). The Go CLI does not use a table for config display. Replace the table with inline formatted rows: `  role  type  address` with column-aligned spacing (using simple `format!` with fixed padding, not `comfy_table`). The sender addresses should already be resolved because `resolve_config()` calls `load_dotenv()` and env var expansion occurs during config loading.

**Files to modify**:
- `crates/treb-cli/src/commands/config.rs` — replace the `build_table()` / `print_table()` block with inline formatted sender rows

**Acceptance criteria**:
- Senders display as aligned inline text, one per line, with role, type, and address columns
- Format: `  <role>  <type>  <address>` with reasonable column alignment (e.g., widest role name + 2 spaces padding)
- Empty address fields display as empty (no placeholder text)
- Output is sorted alphabetically by role name (existing behavior preserved)
- JSON output (`config show --json`) is unchanged
- `cargo check -p treb-cli` passes
- `cargo test -p treb-cli` passes

### P3-US-005: Update `networks --json` to Include Resolved Chain IDs

**Description**: Verify that `networks --json` correctly includes the `chainId` field when env vars are resolved. This should work automatically from US-001 and US-003, but needs explicit verification and a golden test update.

The current `NetworkInfo` struct already has `chain_id: Option<u64>` with `skip_serializing_if = "Option::is_none"`. After env var resolution, previously-unresolvable endpoints will now return chain IDs, so the JSON output will include `chainId` fields that were previously absent.

**Files to modify**:
- `crates/treb-cli/tests/golden/networks_unresolved_env_vars/commands.golden` — update expected output
- `crates/treb-cli/tests/golden/networks_unresolved_json/commands.golden` — update expected output
- Other golden files as needed based on test failures

**Acceptance criteria**:
- `networks --json` includes `"chainId": <number>` for resolved endpoints
- `networks --json` omits `chainId` for unresolved endpoints (existing behavior)
- `networks` text output shows `Chain ID: <number>` for resolved endpoints
- Golden test files updated to reflect new output format
- All golden tests pass with `cargo test -p treb-cli`

### P3-US-006: Update `config show` Golden Tests and Add Integration Test with Env Var Resolution

**Description**: Update golden test files for `config show` to reflect the new inline sender format (from US-004). Add an integration test that verifies end-to-end env var resolution: `.env` file with sender address var, treb.toml referencing it via `${VAR}`, and `config show` displaying the resolved address.

**Files to modify**:
- `crates/treb-cli/tests/golden/config_show_default/commands.golden` — update sender format from table to inline
- `crates/treb-cli/tests/golden/config_show_json/commands.golden` — verify unchanged (JSON format should not change)
- `crates/treb-cli/tests/golden/config_set_show_round_trip/commands.golden` — update if it includes sender display
- Other golden files as needed
- `crates/treb-cli/tests/cli_config.rs` or new test file — add integration test for env var resolution

**Acceptance criteria**:
- All `config show` golden tests reflect inline sender format
- JSON golden tests are unchanged
- Integration test seeds a `.env` file with `TEST_ADDR=0x1234...`, treb.toml with `address = "${TEST_ADDR}"`, runs `config show`, and verifies the address appears in output
- `UPDATE_GOLDEN=1 cargo test -p treb-cli` updates all affected golden files
- `cargo test -p treb-cli` passes
- `cargo clippy --workspace --all-targets` passes

## Functional Requirements

- **FR-1**: `${VAR_NAME}` patterns in foundry.toml `[rpc_endpoints]` values are expanded using `std::env::var()` before any command reads them.
- **FR-2**: `${VAR_NAME}` patterns in foundry.toml `[profile.*.treb.senders.*]` fields are expanded before the config resolver uses them.
- **FR-3**: `.env` and `.env.local` files are loaded before foundry.toml parsing in all code paths (both `resolve_config()` and direct `load_foundry_config()` + `rpc_endpoints()` in the `networks` command).
- **FR-4**: The `networks` command resolves and displays chain IDs via `eth_chainId` RPC calls for endpoints whose URLs were previously blocked by `${VAR}` patterns.
- **FR-5**: The `config show` command displays sender information as inline aligned text (role, type, address) instead of a comfy_table box.
- **FR-6**: The `networks --json` output includes `chainId` for resolved endpoints and omits it for unresolved ones.
- **FR-7**: Unset env vars expand to empty string (consistent with existing `expand_env_vars()` behavior), not an error.
- **FR-8**: The existing `expand_env_vars()` function in `trebfile.rs` is reused — no duplicate implementation.

## Non-Goals

- **Chain ID caching**: The Go CLI caches chain IDs in `cache/chainIds.json`. This phase does not implement caching — chain IDs are fetched fresh each time. Caching can be added in a future phase if needed.
- **Etherscan field expansion**: While Go expands `[etherscan]` URL and key fields, the Rust CLI does not currently use these fields in the `networks` or `config` commands. Expansion can be added when the verify command is updated (Phase 8).
- **`$VAR` bare syntax**: Only `${VAR}` (braced) syntax is supported, matching the existing `expand_env_vars()` implementation. Go's `os.ExpandEnv()` also supports bare `$VAR`, but the Rust CLI's convention is braced-only.
- **RPC URL migration** (`treb migrate config --rpc`): The Go CLI has helpers to migrate hardcoded RPC URLs to `${VAR}` patterns. This is out of scope.
- **Network resolution caching to disk**: No `.treb/chainIds.json` or `cache/chainIds.json` file.
- **Config show `--json` format changes**: The JSON output structure of `config show` is not changed.

## Technical Considerations

### Dependencies
- **dotenvy** crate (already a dependency via `treb-config`) — used for `.env` file loading.
- **reqwest** (already a dependency via `treb-cli`) — used for `eth_chainId` RPC calls.
- No new crate dependencies needed.

### Integration points
- `expand_env_vars()` in `crates/treb-config/src/trebfile.rs` is the single implementation for `${VAR}` expansion. Both treb.toml and foundry.toml expansion use this function.
- `load_dotenv()` in `crates/treb-config/src/env.rs` must be called before any config parsing that relies on env var values. Currently called in `resolve_config()` but NOT in the `networks` command path.
- The `networks` command bypasses `resolve_config()` and calls `load_foundry_config()` + `rpc_endpoints()` directly. This is the code path that needs `.env` loading added.

### Testing
- Unit tests in `treb-config` can use `std::env::set_var` / `remove_var` with unique var names to avoid race conditions (existing pattern in `trebfile.rs` tests).
- Integration tests for `networks` with live RPC resolution require network access — use the existing `has_unresolved_env_vars()` detection to verify expansion happened, or use a mock endpoint pattern.
- Golden tests will need `UPDATE_GOLDEN=1` to regenerate after format changes.

### Backward compatibility
- The `config show` output format change (table to inline) is a visual-only change that affects human output. JSON output is unchanged.
- Env var expansion is additive — previously literal URLs continue to work. Previously unresolvable `${VAR}` URLs now resolve if the var is set.
