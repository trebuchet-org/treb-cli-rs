# PRD: Phase 7 - Fork Command Positional Arguments

## Introduction

The Go `treb` CLI accepts network as a positional argument for all fork subcommands (e.g., `fork enter sepolia`, `fork exit mainnet`). The Rust CLI currently requires `--network <NETWORK>` as a long flag on every fork subcommand. This phase adds positional argument support to all fork subcommands while keeping the `--network` flag for backward compatibility, achieving drop-in parity with Go.

Additionally, the Rust CLI uses `--rpc-url` on `fork enter` for specifying an explicit upstream URL. Go does not expose this as a flag, but for ergonomic parity with other Rust CLI commands (e.g., Foundry tools that accept `--url`), this phase adds `--url` as an alias.

This is Phase 7 of the CLI Compatibility and Drop-in Parity master plan. It is independent of other phases.

## Goals

1. **Positional parity**: All fork subcommands accept network as a positional argument matching Go CLI syntax (`fork enter <network>`, `fork exit [network]`, etc.)
2. **Backward compatibility**: The existing `--network` flag continues to work on all fork subcommands — no breaking changes
3. **Conflict safety**: Providing both positional network and `--network` flag produces a clear error
4. **Flag alias**: `--url` accepted as alias for `--rpc-url` on `fork enter`
5. **Shell completions**: `build.rs` stays in sync with all clap definition changes

## User Stories

### P7-US-001: Add Positional Network Argument and --url Alias to fork enter

**Description:** Update the `fork enter` subcommand to accept a required positional `<network>` argument in addition to the existing `--network` flag, and add `--url` as an alias for `--rpc-url`. Only one form of network specification should be accepted at a time.

**Files to modify:**
- `crates/treb-cli/src/commands/fork.rs` — Update `Enter` variant: add positional `network_pos: Option<String>`, keep `--network` as optional, add merge/conflict logic; add `#[arg(long, alias = "url")]` to `rpc_url`
- `crates/treb-cli/build.rs` — Mirror positional arg and `--url` alias in `build_fork()` enter subcommand
- `crates/treb-cli/src/main.rs` — No changes expected (dispatch uses existing `ForkSubcommand`)

**Implementation notes:**
- Clap approach: Define a positional `#[arg(value_name = "NETWORK")]` field (e.g., `network_pos: Option<String>`) and change `network` from required `String` to `#[arg(long)] network: Option<String>`. Add a helper function `resolve_network_arg(positional: Option<String>, flag: Option<String>) -> anyhow::Result<String>` that returns the network or errors on conflict/missing.
- For `enter`, network is **required** — error if neither positional nor `--network` is provided.
- The `--url` alias uses clap's `#[arg(long, alias = "url")]` on the existing `rpc_url` field.
- Mirror positional arg in `build.rs` using `.arg(Arg::new("network-positional").index(1).help("Network name"))` alongside the existing `--network` long flag.

**Acceptance criteria:**
- `treb fork enter sepolia` works (positional network)
- `treb fork enter --network sepolia` still works (flag form)
- `treb fork enter sepolia --network mainnet` errors with clear conflict message
- `treb fork enter` (no network) errors with clear missing-network message
- `treb fork enter sepolia --url http://localhost:8545` works (alias for --rpc-url)
- `treb fork enter sepolia --rpc-url http://localhost:8545` still works
- Typecheck passes: `cargo check -p treb-cli`
- `cargo clippy -p treb-cli --all-targets` passes

---

### P7-US-002: Add Optional Positional Network to Remaining Fork Subcommands

**Description:** Update `fork exit`, `fork revert`, `fork restart`, `fork history`, and `fork diff` to accept an optional positional `[network]` argument alongside the existing `--network` flag. Apply the same conflict-detection pattern from US-001.

**Files to modify:**
- `crates/treb-cli/src/commands/fork.rs` — Update `Exit`, `Revert`, `Restart`, `History`, `Diff` variants: add positional `network_pos: Option<String>`, change `network` to `Option<String>` where currently required, add merge logic using the shared `resolve_network_arg()` helper
- `crates/treb-cli/build.rs` — Mirror positional args in `build_fork()` for exit, revert, restart, history, diff subcommands

**Implementation notes:**
- For `exit`, `revert`, `restart`: network becomes optional (currently required `String` → `Option<String>`). When neither positional nor `--network` is provided AND `--all` is not set, produce a clear error: `"network required: provide as positional argument or --network flag"`. When `--all` is set, network is not needed.
- For `history`: network is already `Option<String>` — just add the positional form.
- For `diff`: network is currently required — change to `Option<String>` and require at least positional or `--network` (same as `enter`).
- Reuse `resolve_network_arg()` from US-001 for all subcommands. For subcommands where network is truly optional (history, exit/revert with --all), use a variant like `resolve_optional_network_arg()` that returns `Option<String>`.
- Update the `run()` dispatch match arms to call the merge helper before passing to `run_*` functions.

**Acceptance criteria:**
- `treb fork exit sepolia` works (positional)
- `treb fork exit --network sepolia` still works (flag form)
- `treb fork exit sepolia --network mainnet` errors with conflict message
- `treb fork exit --all` works without network
- `treb fork revert mainnet`, `treb fork restart mainnet`, `treb fork diff mainnet` all work positionally
- `treb fork history` works without network (shows all history)
- `treb fork history sepolia` works (filters by network)
- Typecheck passes: `cargo check -p treb-cli`
- `cargo clippy -p treb-cli --all-targets` passes

---

### P7-US-003: Update Golden Tests and Help Snapshots

**Description:** Refresh all fork-related golden test snapshots to reflect the new positional argument syntax in help output and any command output changes. Help text will now show `<NETWORK>` or `[NETWORK]` positional args and the `--url` alias.

**Files to modify:**
- `crates/treb-cli/tests/golden/fork_*/` — All ~32 fork golden test directories need command invocations checked; help-related ones need snapshot updates
- `crates/treb-cli/tests/golden/help_root/` — Root help output if fork description changes
- `crates/treb-cli/tests/fork_integration.rs` — Update `seed_fork_status()` test invocations if they use `--network` (keep them working; optionally add positional variants)
- `crates/treb-cli/tests/integration_fork.rs` — Update any `args(["fork", "exit", "--network", ...])` patterns
- `crates/treb-cli/tests/integration_help.rs` — Add `help_fork_enter` golden test if not present

**Implementation notes:**
- Run `UPDATE_GOLDEN=1 cargo test -p treb-cli` to auto-refresh all golden snapshots.
- Existing golden tests that use `--network` flag form should continue to pass unchanged — only help output snapshots will differ.
- Add a dedicated `help_fork_enter` golden snapshot in `integration_help.rs` to lock the new positional + flag + alias help surface (following Phase 5/6 pattern of per-command help goldens).
- Verify that `fork_enter_success`, `fork_exit_success`, etc. golden command headers still match after clap definition changes.

**Acceptance criteria:**
- `cargo test -p treb-cli --test fork_integration` passes
- `cargo test -p treb-cli --test integration_fork` passes
- `cargo test -p treb-cli --test integration_help` passes
- All fork golden snapshots are current (no diff with `UPDATE_GOLDEN=1`)
- A `help_fork_enter` golden test exists showing positional `<NETWORK>`, `--network`, `--rpc-url`/`--url`
- `cargo test -p treb-cli` passes with no failures

---

### P7-US-004: Add Integration Tests for Positional Argument Behavior

**Description:** Add focused integration tests verifying that positional arguments, flag arguments, conflict detection, and the `--url` alias all work correctly at the subprocess level.

**Files to modify:**
- `crates/treb-cli/tests/integration_fork.rs` — Add new test functions for positional arg forms, conflict errors, and alias behavior

**Implementation notes:**
- Follow the existing `integration_fork.rs` pattern: `make_project()` to set up temp dir, then `treb().args([...]).assert()` for subprocess assertions.
- Test cases to add:
  1. `fork enter <network>` positional form (succeeds with appropriate setup)
  2. `fork enter <network> --network <other>` conflict (stderr contains conflict error)
  3. `fork enter` with no network (stderr contains missing-network error)
  4. `fork exit <network>` positional form
  5. `fork exit <network> --network <other>` conflict
  6. `fork enter <network> --url <url>` alias works (same behavior as `--rpc-url`)
  7. `fork history` without network (shows empty history, not an error)
  8. `fork history <network>` positional form
- Use `NO_COLOR=1` env for stable output assertions.
- These are parser-level and error-path tests — no live Anvil needed (fork enter will fail at RPC resolution, which is fine for testing arg parsing succeeds).

**Acceptance criteria:**
- All new tests pass: `cargo test -p treb-cli --test integration_fork`
- Conflict detection tests verify stderr contains "both positional" or similar conflict message
- Missing-network tests verify stderr contains "network required" or similar
- `--url` alias test verifies identical behavior to `--rpc-url`
- No regressions in existing fork integration tests

---

### P7-US-005: Add CLI Compatibility Alias Tests for Positional vs Flag Forms

**Description:** Add byte-for-byte stdout/stderr comparison tests in `cli_compatibility_aliases.rs` proving that positional and flag forms produce identical output for all fork subcommands.

**Files to modify:**
- `crates/treb-cli/tests/cli_compatibility_aliases.rs` — Add fork positional/flag parity test functions

**Implementation notes:**
- Follow the established pattern in `cli_compatibility_aliases.rs`: run both invocation forms against the same project state and assert identical stdout.
- For commands that need fork state (exit, revert, diff): seed fork state in a temp dir using the `seed_fork_status()` helper from `fork_integration.rs`, or use error output comparison (both forms should produce the same error when fork state doesn't exist).
- Test pairs:
  1. `fork enter sepolia` vs `fork enter --network sepolia` (both will error on missing RPC — compare error output)
  2. `fork exit sepolia` vs `fork exit --network sepolia` (both will error on not-forked — compare error output)
  3. `fork history sepolia` vs `fork history --network sepolia` (both show empty/error — compare output)
  4. `fork enter sepolia --url <url>` vs `fork enter sepolia --rpc-url <url>` (compare error output)
- Use `NO_COLOR=1` to keep output deterministic.

**Acceptance criteria:**
- All parity tests pass: `cargo test -p treb-cli --test cli_compatibility_aliases`
- Each test asserts byte-for-byte identical stdout between positional and flag forms
- Each test asserts byte-for-byte identical stderr between positional and flag forms
- `--url` and `--rpc-url` produce identical output
- No regressions in existing alias compatibility tests

## Functional Requirements

- **FR-1:** `fork enter` accepts a required positional `<network>` argument as the first positional parameter.
- **FR-2:** `fork exit`, `fork revert`, `fork restart`, `fork history`, and `fork diff` accept an optional positional `[network]` argument as the first positional parameter.
- **FR-3:** All fork subcommands continue to accept `--network <NETWORK>` as a long flag.
- **FR-4:** Providing both a positional network argument and `--network` flag produces a user-facing error message.
- **FR-5:** `fork enter` accepts `--url` as an alias for `--rpc-url`.
- **FR-6:** `fork status` is unchanged (no network argument).
- **FR-7:** Shell completions in `build.rs` reflect positional arguments and the `--url` alias.
- **FR-8:** For `fork exit` and `fork revert`, when `--all` is set, network is not required.
- **FR-9:** For `fork history`, omitting network shows all history entries (existing behavior preserved).
- **FR-10:** Help output for all fork subcommands shows both positional and flag forms.

## Non-Goals

- **Config-based network fallback**: Go's fork commands fall back to `config.Network.Name` when no network is specified. Implementing config-based default network resolution is out of scope — this phase focuses on positional argument acceptance.
- **Short flags**: No `-n` short flag for `--network` on fork subcommands (fork uses positional args as the Go-compatible shorthand).
- **New fork subcommands**: No new fork functionality — only argument/flag changes to existing subcommands.
- **E2E workflow test changes**: The `e2e_fork_workflow.rs` tests use `--network` flag form and don't need to change (backward compat ensures they still pass).
- **Positional args for non-network parameters**: `--fork-block-number`, `--all`, `--json` remain flag-only.

## Technical Considerations

### Clap Positional + Flag Coexistence

Clap supports having both a positional argument and a named flag for the same logical parameter, but they must be separate fields. The recommended pattern:

```rust
Enter {
    /// Network name
    #[arg(value_name = "NETWORK")]
    network_pos: Option<String>,
    /// Network name (flag form)
    #[arg(long)]
    network: Option<String>,
    // ...
}
```

A merge function resolves the two fields into one value, erroring on conflict. This keeps the clap definition clean and the conflict logic explicit.

### build.rs Synchronization

Every clap change in `fork.rs` must be mirrored in `build.rs`'s `build_fork()` function. Positional args use `.arg(Arg::new("NETWORK").index(1))` in the manual builder. The `--url` alias uses `.alias("url")` on the `rpc-url` arg.

### Golden Test Impact

The fork command family has ~32 golden test directories. Most use `--network` flag form in their command invocations, so command output won't change. Only help-related goldens will change due to the new positional argument appearing in usage lines. Run `UPDATE_GOLDEN=1 cargo test -p treb-cli` to batch-refresh.

### Existing Test Compatibility

All existing tests use `--network` flag form. Since the flag is preserved as backward-compatible, no existing test should break from the clap definition changes alone. The only breakage vector is if changing `network: String` to `network: Option<String>` on some variants requires updating match arms — this is handled in US-002.
