# PRD: Phase 6 - Deployment Query Flags (show, list, tag)

## Introduction

Phase 6 adds deployment query and scoping flags to the `show`, `list`, and `tag` commands to match the Go CLI's filtering capabilities. The Go CLI allows users to scope deployment lookups by namespace, network, and fork status — critical for multi-namespace and multi-network workflows (e.g., `mento-deployments-v2` which has `mainnet/`, `baklava/`, and `fork/` namespaces across multiple chains).

The Rust CLI's `list` command already has full filtering support (namespace, network, type, tag, contract, label, fork/no-fork) wired end-to-end. The `show` command lacks `--namespace`, `--network`, and `--no-fork` flags entirely. The `tag` command has `--namespace`/`-s` and `--network`/`-n` defined in clap (added in Phase 5) but not wired into runtime behavior.

**Depends on:** Phase 2 (Go registry coexistence — registry must load Go-created data).

## Goals

1. **`show` command parity**: Add `--namespace`, `--network`, and `--no-fork` flags to `show`, matching Go's `internal/cli/show.go` behavior.
2. **`tag` command wiring**: Wire the existing `--namespace`/`--network` clap fields into `tag`'s runtime handler so tag operations are scoped to a specific namespace/network context.
3. **Filter correctness**: All new flags properly filter registry queries using the same case-insensitive matching and fork-detection patterns already established in `list`.
4. **Shell completion parity**: All new flags mirrored in `build.rs` so generated completions include them.
5. **Test coverage**: Integration tests verifying flag behavior against seeded registry data (both `seed_registry()` and `seed_go_compat_registry()` fixtures).

## User Stories

### P6-US-001: Add --namespace, --network, and --no-fork Flags to show Command

**Description:** Define `--namespace`, `--network`, and `--no-fork` clap fields on the `Show` variant in `main.rs` and mirror them in `build.rs` for shell completions. This is parser-only — no runtime wiring yet.

**Files to modify:**
- `crates/treb-cli/src/main.rs` — Add `namespace: Option<String>`, `network: Option<String>`, `no_fork: bool` fields to the `Show` variant. Note: Go does NOT use short flags on `show` (unlike `list`/`tag`), so no `-s`/`-n` here.
- `crates/treb-cli/build.rs` — Add `--namespace`, `--network`, `--no-fork` args to `build_show()`.

**Acceptance criteria:**
- `treb show --namespace mainnet --network 42220 --no-fork MyContract` parses without error.
- `treb show --namespace mainnet MyContract` parses (each flag is optional).
- `treb show --no-fork MyContract` parses.
- `treb show MyContract` still works (backward compat, no flags required).
- `parse_cli_from(...)` unit test in `main.rs` pins the new flag parsing.
- Typecheck passes: `cargo check -p treb-cli`.

---

### P6-US-002: Wire show Flags into Deployment Resolution and Filtering

**Description:** Pass the new `--namespace`, `--network`, and `--no-fork` flags through to `show`'s runtime handler. These flags should filter the candidate deployment set before resolution, narrowing the scope of the 5-strategy resolve cascade in `resolve.rs`.

**Implementation approach:**
- Update the `Commands::Show` match arm in `main.rs` to destructure and pass `namespace`, `network`, `no_fork` to `commands::show::run()`.
- Update `show::run()` signature to accept the new parameters.
- Before calling `resolve_deployment()`, pre-filter the registry's deployment list:
  - `--namespace`: Case-insensitive namespace match on deployment ID prefix.
  - `--network`: Match chain ID (numeric) or named network via the same `network_matches()` pattern used in `list.rs`.
  - `--no-fork`: Exclude deployments whose namespace starts with `fork/`.
- If resolution finds zero matches after filtering, return a clear error mentioning the active filters.
- If resolution finds multiple matches and interactive mode is available, the fuzzy selector should only show filtered candidates.
- Reuse filtering logic from `list.rs` where possible (consider extracting `network_matches()` to a shared location if it isn't already).

**Files to modify:**
- `crates/treb-cli/src/main.rs` — Update `Show` match arm to pass new fields.
- `crates/treb-cli/src/commands/show.rs` — Accept and apply filters before resolution.
- Potentially `crates/treb-cli/src/commands/resolve.rs` — If resolve needs to accept a pre-filtered deployment set or filter parameters.

**Acceptance criteria:**
- `treb show --namespace mainnet MyContract` resolves only within `mainnet/` namespace.
- `treb show --network 42220 MyContract` resolves only within chain 42220 deployments.
- `treb show --no-fork MyContract` skips any `fork/`-prefixed deployments.
- Flags compose: `--namespace mainnet --network 42220 --no-fork` applies all three filters.
- Error message includes active filter context when no match found.
- `--json` output still works with all new flags.
- Typecheck passes: `cargo check -p treb-cli`.

---

### P6-US-003: Wire tag --namespace and --network into Runtime Handler

**Description:** The `tag` command already has `--namespace`/`-s` and `--network`/`-n` defined in clap (Phase 5), but the match arm uses `..` to ignore them. Wire these into the runtime handler so tag operations (show/add/remove) are scoped to a specific namespace/network.

**Implementation approach:**
- Update the `Commands::Tag` match arm in `main.rs` to destructure `namespace` and `network` and pass them to `commands::tag::run()`.
- Update `tag::run()` signature to accept `namespace: Option<String>` and `network: Option<String>`.
- Apply the same pre-filtering approach as `show`: narrow the candidate set before deployment resolution.
- For the `add` and `remove` operations, the scoped resolution ensures the tag mutation targets the correct deployment when names are ambiguous across namespaces/networks.

**Files to modify:**
- `crates/treb-cli/src/main.rs` — Update `Tag` match arm to pass `namespace` and `network`.
- `crates/treb-cli/src/commands/tag.rs` — Accept and apply namespace/network filters before resolution.

**Acceptance criteria:**
- `treb tag --namespace mainnet MyContract` resolves MyContract only within `mainnet/`.
- `treb tag --add v2 -s mainnet -n 42220 MyContract` adds tag scoped to mainnet/42220.
- `treb tag --remove v2 --namespace mainnet MyContract` removes tag scoped to mainnet.
- When no flags provided, behavior is unchanged (full registry scope).
- Typecheck passes: `cargo check -p treb-cli`.

---

### P6-US-004: Update Golden Tests and Help Snapshots

**Description:** Update golden test snapshots affected by the new `show` flags. Add help snapshot coverage for `show` command's updated help text.

**Files to modify:**
- `crates/treb-cli/tests/golden/` — Update any `help_show*` snapshots and other affected golden files.
- `crates/treb-cli/tests/integration_help.rs` — Add or update `help_show` golden test if not present.
- Run `UPDATE_GOLDEN=1 cargo test -p treb-cli` to refresh snapshots.

**Acceptance criteria:**
- `show --help` output includes `--namespace`, `--network`, `--no-fork` in golden snapshot.
- All existing golden tests pass without unrelated changes.
- `cargo test -p treb-cli` passes (all golden comparisons match).

---

### P6-US-005: Add Integration Tests for Query Flag Filtering Behavior

**Description:** Add integration tests that verify the runtime filtering behavior of all new/wired flags against seeded registry data. Follow the subprocess test pattern from `cli_compatibility_aliases.rs` and `cli_go_registry_compat.rs`.

**Tests to add:**
1. **show --namespace filter**: Seed registry with deployments across multiple namespaces. Verify `show --namespace mainnet ContractName` resolves within mainnet only and errors when the contract exists in another namespace but not mainnet.
2. **show --network filter**: Seed registry, verify `show --network 42220 ContractName` scopes to chain 42220.
3. **show --no-fork filter**: Seed registry with a `fork/`-prefixed deployment. Verify `--no-fork` excludes it.
4. **show combined filters**: Verify `--namespace X --network Y --no-fork` composes correctly.
5. **tag --namespace filter**: Seed registry, run `tag --add tagname -s mainnet ContractName`, verify only the mainnet deployment is tagged.
6. **tag --network filter**: Similar scoped tag test with `--network`.
7. **list verification** (sanity): Confirm existing `list` filters still work (regression guard — no new logic needed, just a smoke test against seeded data).

**Files to create/modify:**
- `crates/treb-cli/tests/integration_show.rs` — New or extended test file for show flag tests.
- `crates/treb-cli/tests/integration_tag.rs` — Extend with namespace/network scoped tag tests.
- Use `seed_registry()` or `seed_go_compat_registry()` from `helpers/mod.rs` for fixture data.

**Acceptance criteria:**
- All new integration tests pass with `cargo test -p treb-cli`.
- Tests use subprocess execution (not in-process) to match established patterns.
- Tests verify both success and error cases (e.g., no match within filtered scope).
- Seeded registry data covers multi-namespace and multi-network scenarios.

## Functional Requirements

- **FR-1**: `show --namespace <NS>` filters candidate deployments to those whose namespace matches `<NS>` (case-insensitive) before resolution.
- **FR-2**: `show --network <NET>` filters candidate deployments to those whose chain ID matches `<NET>` (numeric chain ID or named network via alloy `Chain` enum) before resolution.
- **FR-3**: `show --no-fork` excludes deployments whose namespace starts with `fork/` from the candidate set.
- **FR-4**: All `show` filter flags are optional and compose with AND logic when multiple are provided.
- **FR-5**: `tag --namespace <NS>` and `tag --network <NET>` scope tag operations (show/add/remove) to deployments matching the namespace/network filter.
- **FR-6**: `show` does NOT have `-s`/`-n` short flags (matching Go — only `list` and `tag` have short flags for namespace/network).
- **FR-7**: `--no-fork` on `show` is mirrored in `build.rs` for shell completions.
- **FR-8**: When namespace/network filters result in zero candidates, the error message includes the active filter context (e.g., "no deployments found in namespace 'mainnet' on network '42220'").
- **FR-9**: Interactive fuzzy selector in `show` and `tag` (when no deployment argument provided) respects namespace/network/no-fork filters — only filtered candidates shown.

## Non-Goals

- **Not changing `list` command**: `list` already has full filtering support wired end-to-end from Phase 5. No modifications needed.
- **Not adding `--tag` filter to `show`**: Go's `show` does not have a `--tag` filter; it operates on a single deployment.
- **Not adding `--no-fork` to `tag`**: Go's `tag` command does not have `--no-fork`. Namespace/network scoping is sufficient for tag operations.
- **Not adding `--fork` to `show`**: Go's `show` only has `--no-fork` (exclusion), not `--fork` (inclusion-only). `list` already has both.
- **Not refactoring `resolve_deployment()`**: Keep changes minimal. Pre-filter the candidate set before passing to resolve rather than restructuring the resolve cascade.
- **Not adding config-derived namespace/network defaults**: Go derives defaults from `app.Config.Namespace`/`app.Config.Network`; this is a broader config integration that can be addressed separately.

## Technical Considerations

### Dependencies
- **Phase 2**: Registry must load Go-created data. This is complete — `seed_go_compat_registry()` and bare JSON read/write are working.
- **Phase 5**: `tag` clap fields (`--namespace`/`-s`, `--network`/`-n`) already defined; `build.rs` already has them. Phase 6 only needs to wire runtime behavior.

### Shared Filtering Logic
- `list.rs` has `network_matches()` (chain ID or named network matching) and `filter_deployments()` with the `DeploymentFilters` struct. Consider extracting `network_matches()` to a shared module (e.g., `commands/filters.rs` or alongside `resolve.rs`) so `show` and `tag` can reuse it without duplicating the alloy `Chain` enum lookup logic.
- Fork detection is a simple `namespace.starts_with("fork/")` check — no extraction needed, inline is fine.

### Resolution Flow Change
- Currently `show` calls `resolve_deployment(registry, lookup, query)` which searches the entire registry. The change is to pre-filter deployments before resolution, not to modify `resolve_deployment()` itself.
- If `resolve_deployment()` takes `&Registry` and searches internally, the simplest approach is to filter the deployment list first and pass a filtered view, or to add optional filter parameters.

### Test Fixtures
- `seed_registry()` writes from `treb-core/tests/fixtures/deployments_map.json` — contains deployments across `mainnet/42220/` namespace. Check if multiple namespaces/networks are present; if not, the integration tests may need to augment the fixture or construct test-specific data.
- `seed_go_compat_registry()` writes from `treb-registry/tests/fixtures/go-compat/deployments.json` — 13 deployments from real Go data.

### Build.rs Sync
- `show` needs `--namespace`, `--network`, `--no-fork` added to `build_show()` in `build.rs`.
- `tag` already has `--namespace`/`-s` and `--network`/`-n` in `build_tag()` from Phase 5.
