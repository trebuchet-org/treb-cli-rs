# Master Plan: CLI Compatibility and Drop-in Parity with Go

Make the Rust `treb-cli` a true drop-in replacement for the Go `treb` by fixing registry compatibility, aligning command surface (names, flags, aliases, positional args), resolving broken env var handling, and adding the missing `addressbook` command. Based on side-by-side exploratory testing at `~/projects/mento-deployments-v2` with both CLIs installed.

**Discovery document:** `plans/2026-03-10-exploratory-testing-comparison.md`
**Reference codebase:** `../treb-cli` (Go CLI)
**Target codebase:** `treb-cli-rs` (Rust CLI)

**Scope:** This plan covers compatibility gaps where Rust is **behind** Go or **incompatible**. Features where Rust extends beyond Go (e.g., `--json` on more commands, `--dry-run` on prune, `version --json`) are intentional improvements and are **not** in scope for removal.

---

## Phase 1 -- Registry Format Coexistence

Critical blocker. The Rust CLI cannot read any Go-created `.treb/` directory because `registry.json` formats differ. Go writes a `SolidityRegistry` map (`{chainId: {namespace: {name: address}}}`), while Rust expects `RegistryMeta` (`{version, createdAt, updatedAt}`). This blocks `list`, `show`, `tag`, and all registry-dependent commands in any project previously used with the Go CLI.

The fix must allow the two CLIs to coexist on the same project — both reading and writing to the same `.treb/` directory without corrupting each other's data.

**Go source files to reference:**
- `internal/registry/registry.go` — Go registry format: SolidityRegistry map structure
- `internal/registry/deployments.go` — Go deployments.json format
- `internal/registry/transactions.go` — Go transactions.json format
- `internal/registry/safe.go` — Go safe-txs.json format

**Rust files to modify:**
- `crates/treb-registry/src/types.rs` — `RegistryMeta` deserialization (tolerate missing `version`)
- `crates/treb-registry/src/store/registry_meta.rs` (or equivalent) — load/save logic
- `crates/treb-registry/src/store/deployment.rs` — verify deployments.json compatibility
- `crates/treb-registry/src/store/transaction.rs` — verify transactions.json compatibility
- `crates/treb-registry/src/store/safe_tx.rs` — verify safe-txs.json compatibility

**Deliverables**
- Rust CLI loads successfully in a Go-created `.treb/` directory
- `registry.json` parsing either: (a) detects Go format and creates/uses a separate Rust meta file, or (b) makes `version` field optional with auto-migration
- `deployments.json`, `transactions.json`, `safe-txs.json` read correctly from Go-written files
- Rust writes remain compatible with Go CLI reads
- Integration test verifying round-trip: Go writes → Rust reads → Rust writes → Go reads
- All registry-dependent commands (`list`, `show`, `tag`, etc.) work in Go-created projects

**User stories:** 6
**Dependencies:** none

---

## Phase 2 -- Environment Variable Resolution and Config Display

The Rust CLI fails to resolve `${VAR}` patterns in foundry.toml RPC endpoints, causing `networks` to show errors for all networks and `config show` to display empty sender addresses. The Go CLI resolves these via `.env` file loading and environment variable expansion.

**Go source files to reference:**
- `internal/config/foundry.go` — foundry.toml parsing with `${VAR}` resolution
- `internal/config/env.go` — `.env` file loading
- `internal/cli/render/config.go` — config show with resolved sender addresses

**Rust files to modify:**
- `crates/treb-config/src/` — env var resolution for `${VAR}` patterns in foundry.toml
- `crates/treb-cli/src/commands/networks.rs` — display resolved chain IDs
- `crates/treb-cli/src/commands/config.rs` — display resolved sender addresses

**Deliverables**
- `${VAR}` env var patterns in foundry.toml resolved from environment and `.env` files
- `networks` command resolves and displays chain IDs (matching Go output)
- `config show` displays resolved sender addresses (not empty)
- `config show` sender format matches Go: inline `role  type  address` (not comfy_table)
- `networks --json` includes resolved `chainId` field when available
- Golden file updates for networks and config show

**User stories:** 6
**Dependencies:** none

---

## Phase 3 -- Command Naming, Aliases, and Structure

Align command names, subcommand structure, and aliases with the Go CLI to ensure scripts and documentation targeting the Go CLI work unmodified with the Rust CLI.

| Current Rust | Go | Change needed |
|-------------|-----|---------------|
| `gen-deploy` | `gen deploy` | Restructure as nested subcommand |
| `completions` | `completion` | Rename (singular) |
| `config` (requires subcommand) | `config` (shows config) | Default to `show` when no subcommand |
| `list` (no alias) | `list` / `ls` | Add `ls` alias |
| No `gen` alias | `gen` / `generate` | Add `generate` alias |

**Rust files to modify:**
- `crates/treb-cli/src/main.rs` — command definitions, aliases, default subcommands
- `crates/treb-cli/src/commands/gen_deploy.rs` — restructure as `gen deploy` with gen parent
- `crates/treb-cli/src/commands/config.rs` — default behavior when no subcommand given

**Deliverables**
- `treb gen deploy <artifact>` works (nested subcommand, matching Go)
- `treb generate deploy <artifact>` works (alias, matching Go)
- `treb gen-deploy <artifact>` still works (backward compat via alias or hidden command)
- `treb completion <shell>` works (renamed from `completions`)
- `treb config` (no subcommand) shows current config (same as `config show`)
- `treb ls` alias for `treb list`
- Golden file updates for help output

**User stories:** 6
**Dependencies:** none

---

## Phase 4 -- Global Non-Interactive Flag and Short Flags

The Go CLI has `--non-interactive` as a global flag inherited by every command. The Rust CLI only has it on `run` and `compose`. Several commands also lack the `-s`/`-n` short flags that Go provides.

**Go source files to reference:**
- `internal/cli/root.go` — global `--non-interactive` flag definition
- `internal/cli/list.go` — `-s`/`--namespace`, `-n`/`--network` short flags
- `internal/cli/tag.go` — `-s`/`--namespace`, `-n`/`--network` short flags

**Rust files to modify:**
- `crates/treb-cli/src/main.rs` — add `--non-interactive` as global CLI option
- `crates/treb-cli/src/commands/list.rs` — add `-s` for namespace, `-n` for network
- `crates/treb-cli/src/commands/tag.rs` — add `-s` for namespace, `-n` for network
- `crates/treb-cli/src/commands/show.rs` — add namespace, network flags (see Phase 5)
- All command run() functions — read global non-interactive flag

**Deliverables**
- `--non-interactive` accepted on every command (global flag)
- Global non-interactive detection: flag OR `TREB_NON_INTERACTIVE=1` OR `CI=true` OR non-TTY
- `-s` short flag for `--namespace` on `list`, `tag`, and other commands matching Go
- `-n` short flag for `--network` on `list`, `tag`, and other commands matching Go
- Verify no clashes between `-s`/`-n` and existing short flags (e.g., `-s` on verify = `--sourcify`)
- Golden file updates for help output

**User stories:** 5
**Dependencies:** none

---

## Phase 5 -- Deployment Query Flags (show, list, tag)

The Go CLI's `show`, `list`, and `tag` commands have filtering/scoping flags that the Rust CLI lacks. These are needed for multi-namespace and multi-network workflows.

**Go source files to reference:**
- `internal/cli/show.go` — `--namespace`, `--network`, `--no-fork` flags
- `internal/cli/list.go` — `-s`/`-n` short flags, `--tag` filter (Rust already has this)
- `internal/cli/tag.go` — `-s`/`--namespace`, `-n`/`--network` flags

**Rust files to modify:**
- `crates/treb-cli/src/commands/show.rs` — add `--namespace`, `--network`, `--no-fork`
- `crates/treb-cli/src/commands/tag.rs` — add `--namespace`, `--network` (if not already present)
- Command execution paths — wire new flags into registry queries

**Deliverables**
- `show --namespace <NS>` scopes lookup to specific namespace
- `show --network <NET>` scopes lookup to specific network/chain
- `show --no-fork` skips fork-added deployments
- `tag --namespace <NS>` and `tag --network <NET>` scope tag operations
- All new flags properly filter registry queries
- Golden file updates

**User stories:** 5
**Dependencies:** Phase 1 (registry must be loadable)

---

## Phase 6 -- Fork Command Positional Arguments

The Go CLI accepts network as a positional argument for fork subcommands (`fork enter <network>`, `fork exit [network]`, etc.). The Rust CLI requires `--network` flag. For drop-in parity, Rust should accept both forms.

Also, Go uses `--url` for external fork connection while Rust uses `--rpc-url`. Both should be accepted.

**Go source files to reference:**
- `internal/cli/fork.go` — all fork subcommand definitions with positional network arg

**Rust files to modify:**
- `crates/treb-cli/src/commands/fork.rs` — update clap definitions for all fork subcommands
- Fork enter: accept positional `<network>` OR `--network <NETWORK>`
- Fork exit/revert/restart/history/diff: accept positional `[network]` OR `--network <NETWORK>`

**Deliverables**
- `fork enter <network>` works (positional, matching Go)
- `fork enter --network <network>` still works (backward compat)
- `fork exit [network]` works (optional positional, matching Go)
- Same for `fork revert`, `fork restart`, `fork history`, `fork diff`
- `fork enter --url <URL>` accepted as alias for `--rpc-url`
- Conflict detection: error if both positional and `--network` provided
- Golden file updates for help output

**User stories:** 5
**Dependencies:** none

---

## Phase 7 -- Verify Command Flag Parity

The verify command is missing several flags needed for production verification workflows. The Go CLI supports namespace/network scoping, debug mode, and manual contract path specification.

**Go source files to reference:**
- `internal/cli/verify.go` — full flag set including `--namespace`, `--network`, `--debug`, `--contract-path`, `--blockscout-verifier-url`
- Short flags: `-e` (etherscan), `-b` (blockscout), `-s` (sourcify), `-n` (network)

**Rust files to modify:**
- `crates/treb-cli/src/commands/verify.rs` — add missing flags and short flags

**Deliverables**
- `--namespace <NS>` filter for verification scope
- `--network <NET>` / `-n` for network selection
- `--debug` flag for showing forge verify commands
- `--contract-path <PATH>` for manual contract path specification (e.g., `./src/Counter.sol:Counter`)
- `-e` short for `--etherscan`, `-b` for `--blockscout`, `-s` for `--sourcify`
- `--blockscout-verifier-url` accepted as alias for `--verifier-url` when using blockscout
- Golden file updates

**User stories:** 5
**Dependencies:** none

---

## Phase 8 -- Addressbook Command

Implement the `addressbook` command that is entirely missing from the Rust CLI. The addressbook manages named addresses scoped to chain ID, stored in `.treb/addressbook.json`. The Go CLI provides `list`, `set`, `remove` subcommands with `ab` alias.

**Go source files to reference:**
- `internal/cli/addressbook.go` — command definitions and handlers
- `internal/cli/render/addressbook.go` — rendering (name-address pairs, scoped by chain)
- `internal/registry/addressbook.go` — store implementation

**Rust files to create/modify:**
- `crates/treb-registry/src/store/addressbook.rs` — new store module
- `crates/treb-registry/src/store/mod.rs` — register addressbook store
- `crates/treb-registry/src/registry.rs` — add addressbook to Registry facade
- `crates/treb-cli/src/commands/addressbook.rs` — new command module
- `crates/treb-cli/src/commands/mod.rs` — register addressbook command
- `crates/treb-cli/src/main.rs` — add `addressbook` command with `ab` alias in Management Commands group

**Deliverables**
- `treb addressbook list` / `treb ab ls` — lists entries for current chain (name-address pairs)
- `treb addressbook set <name> <address>` / `treb ab set` — adds/updates entry
- `treb addressbook remove <name>` / `treb ab remove` — removes entry
- Entries scoped to chain ID (from `--network` flag or config)
- `--json` output on `list`
- `--namespace` and `--network` flags on all subcommands
- Human output matches Go format: `  NAME  ADDRESS` aligned columns
- `addressbook` (no subcommand) defaults to `list` (matching Go)
- Store reads/writes `.treb/addressbook.json` compatible with Go format
- Golden file tests

**User stories:** 7
**Dependencies:** Phase 1 (registry store patterns), Phase 2 (network resolution)

---

## Phase 9 -- Register and Sync Flag Alignment

Minor flag differences on `register` and `sync` that affect production workflows. The Go CLI derives network/namespace from config while the Rust CLI may require explicit flags.

**Go source files to reference:**
- `internal/cli/register.go` — `--contract-name` separate from `--contract`, no `--rpc-url` (uses config)
- `internal/cli/sync.go` — no `--network` flag (uses config), `--clean` removes invalid entries

**Rust files to modify:**
- `crates/treb-cli/src/commands/register.rs` — verify flag behavior matches Go
- `crates/treb-cli/src/commands/sync.rs` — verify network resolution from config

**Deliverables**
- `register` falls back to configured network when `--network` not provided
- `register --contract-name` behavior matches Go (separate from `--contract`)
- `sync` uses configured network when `--network` not provided (matching Go)
- `sync --clean` flag description matches Go ("Remove invalid entries while syncing")
- Verify `register` and `sync` work without explicit `--network` in Go-created projects

**User stories:** 4
**Dependencies:** Phase 1 (registry), Phase 2 (config resolution)

---

## Phase 10 -- Error Messages, Version Format, and Help Text

Polish remaining surface-level differences: error message format, version string format, and help text consistency.

**Differences to address:**
- Go: `Error: unknown command "foo" for "treb"` — Rust: clap default `error: unrecognized subcommand 'foo'`
- Go: `treb nightly-41-gc72d1b1` — Rust: `treb 0.1.0` (version string scheme)
- Go: `Use "treb [command] --help"` footer — Rust: clap default footer
- Go: full help on `-h` — Rust: summary on `-h`, full on `--help`

**Rust files to modify:**
- `crates/treb-cli/src/main.rs` — custom error handler for clap errors, version string format
- `crates/treb-cli/src/commands/version.rs` — version string format alignment
- `crates/treb-cli/build.rs` — version string construction

**Deliverables**
- Version format matches Go: `treb <version-tag>` with git-describe style versioning
- Error messages for unknown commands match Go format
- Help footer `Use "treb [command] --help"` consistent across all commands
- `-h` behavior review: document whether summary vs full help is intentional (clap convention vs Go cobra convention)
- Unknown flag errors match Go format where feasible

**User stories:** 4
**Dependencies:** none

---

## Dependency Graph (ASCII)

```
Phase 1 (registry compat) ──┬──> Phase 5 (show/list/tag flags)
                             ├──> Phase 8 (addressbook)
                             └──> Phase 9 (register/sync flags)

Phase 2 (env var resolution) ──> Phase 8 (addressbook)
                              └──> Phase 9 (register/sync flags)

Phase 3 (naming/aliases)         [independent]
Phase 4 (global flags)           [independent]
Phase 6 (fork positional args)   [independent]
Phase 7 (verify flags)           [independent]
Phase 10 (error/version format)  [independent]
```

---

## Summary Table

| Phase | Title | Stories | Depends On |
|------:|-------|--------:|------------|
| 1 | Registry Format Coexistence | 6 | -- |
| 2 | Environment Variable Resolution and Config Display | 6 | -- |
| 3 | Command Naming, Aliases, and Structure | 6 | -- |
| 4 | Global Non-Interactive Flag and Short Flags | 5 | -- |
| 5 | Deployment Query Flags (show, list, tag) | 5 | 1 |
| 6 | Fork Command Positional Arguments | 5 | -- |
| 7 | Verify Command Flag Parity | 5 | -- |
| 8 | Addressbook Command | 7 | 1, 2 |
| 9 | Register and Sync Flag Alignment | 4 | 1, 2 |
| 10 | Error Messages, Version Format, and Help Text | 4 | -- |
| **Total** | | **53** | |
