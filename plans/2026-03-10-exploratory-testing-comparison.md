# Exploratory Testing: treb (Go) vs treb-cli (Rust)

Tested at `~/projects/mento-deployments-v2` with both binaries installed.

---

## 1. Top-Level Help Differences

### Description
| Aspect | Go | Rust |
|--------|------|------|
| Description | "Trebuchet (treb) orchestrates Foundry script execution for deterministic smart contract deployments using CreateX factory contracts." | "Smart contract deployment orchestrator for Foundry" |
| Usage line | `treb [command]` | `treb [OPTIONS] <COMMAND>` |

### Global Flags
| Flag | Go | Rust |
|------|------|------|
| `--non-interactive` | Global flag on all commands | Per-command (on `run`, `compose`); not global |
| `--no-color` | Not supported (uses `NO_COLOR` env only) | Global flag on all commands |
| `-V, --version` | Not supported (`treb version` only) | Supported via clap |
| `--help` | Shows full help always | `-h` = summary, `--help` = full |

### Command Naming
| Feature | Go | Rust |
|---------|------|------|
| Gen deploy | `treb gen deploy` (subcommand) | `treb gen-deploy` (flat hyphenated) |
| Completions | `treb completion` (singular) | `treb completions` (plural) |
| Config (no args) | Shows current config | Requires subcommand (`config show`) |

### Command Aliases
| Alias | Go | Rust |
|-------|------|------|
| `list` → `ls` | Supported | Not tested (registry parse fails) |
| `gen` → `generate` | Supported | N/A (different structure) |
| `addressbook` → `ab` | Supported | N/A (command missing) |

---

## 2. Missing Commands in Rust

| Command | Status | Notes |
|---------|--------|-------|
| `addressbook` / `ab` | **Missing entirely** | Go has `list`, `set`, `remove` subcommands |
| `show --json` | N/A | Go doesn't have `--json` on `show` either |

---

## 3. Structural Differences

### `migrate` Command
| Aspect | Go | Rust |
|--------|------|------|
| Structure | Flat command (no subcommands) | Has subcommands: `config`, `registry` |
| Scope | Config migration only | Config migration + registry schema migration |

### `gen` / `gen-deploy`
| Aspect | Go | Rust |
|--------|------|------|
| Structure | `treb gen deploy <artifact>` (nested) | `treb gen-deploy <artifact>` (flat) |
| Aliases | `gen` / `generate` | None |
| Flags | `--proxy` (boolean), `--proxy-contract`, `--strategy`, `--script-path` | `--proxy <pattern>` (takes value: erc1967, uups, etc.), `--proxy-contract`, `--strategy`, `--output`, `--json` |

### `config` Command
| Aspect | Go | Rust |
|--------|------|------|
| No subcommand | Shows current config | Error: requires subcommand |
| Subcommands | `set`, `remove` | `show`, `set`, `remove` |

### `fork enter`
| Aspect | Go | Rust |
|--------|------|------|
| Network arg | Positional: `fork enter <network>` | Flag: `fork enter --network <NETWORK>` |
| External fork | `--url` flag | `--rpc-url` flag |
| Block number | Not available | `--fork-block-number` flag |
| JSON output | Not available | `--json` flag |

### `fork exit`
| Aspect | Go | Rust |
|--------|------|------|
| Network arg | Positional: `fork exit [network]` | Flag: `fork exit --network <NETWORK>` |
| `--all` flag | Supported | Supported |

### `fork revert`
| Aspect | Go | Rust |
|--------|------|------|
| Network arg | Positional: `fork revert [network]` | Flag: `fork revert --network <NETWORK>` |

### `fork diff`
| Aspect | Go | Rust |
|--------|------|------|
| Network arg | Positional: `fork diff [network]` | Flag: `fork diff --network <NETWORK>` |

### `show`
| Aspect | Go | Rust |
|--------|------|------|
| Argument | Required: `<deployment>` | Optional: `[DEPLOYMENT]` (interactive select if omitted) |
| `--namespace` | Supported | Not present |
| `--network` | Supported | Not present |
| `--no-fork` | Supported | Not present |
| `--json` | Not supported | Supported |

### `list` Short Flags
| Flag | Go | Rust |
|------|------|------|
| `-s` | `--namespace` | Not available |
| `-n` | `--network` | Not available |
| `--tag` | Not present | Supported (extra filter) |

### `verify` Flags
| Flag | Go | Rust |
|------|------|------|
| `-e` | `--etherscan` | `--etherscan` |
| `-b` | `--blockscout` | `--blockscout` |
| `-s` | `--sourcify` | `--sourcify` |
| `--namespace` | Supported | Not present |
| `--network` / `-n` | Supported | Not present |
| `--debug` | Supported | Not present |
| `--contract-path` | Supported | Not present |
| `--blockscout-verifier-url` | Supported | `--verifier-url` (generic) |
| `--verifier` | Not present | Supported (default: etherscan) |
| `--verifier-api-key` | Not present | Supported |
| `--watch` | Not present | Supported |
| `--retries` | Not present | Supported (default: 5) |
| `--delay` | Not present | Supported (default: 5) |
| `--json` | Not present | Supported |

### `register` Flags
| Flag | Go | Rust |
|------|------|------|
| `--contract-name` | Supported (separate from `--contract`) | Supported |
| `--address` | Optional (from trace) | Optional (filter) |
| `--rpc-url` | Not present | Supported |
| `--namespace` | Not present | Supported |
| `--deployment-type` | Not present | Supported |
| `--json` | Not present | Supported |

### `prune` Flags
| Flag | Go | Rust |
|------|------|------|
| `--include-pending` | Supported | Supported |
| `--network` | Required | Supported |
| `--dry-run` | Not present | Supported |
| `-y, --yes` | Not present | Supported |
| `--check-onchain` | Not present | Supported |
| `--rpc-url` | Not present | Supported |
| `--json` | Not present | Supported |

### `reset` Flags
| Flag | Go | Rust |
|------|------|------|
| `--network` | Not present (uses config) | Supported |
| `--namespace` | Not present (uses config) | Supported |
| `-y, --yes` | Not present | Supported |
| `--json` | Not present | Supported |

### `sync` Flags
| Flag | Go | Rust |
|------|------|------|
| `--network` | Not present (uses config) | Supported |
| `--json` | Not present | Supported |

---

## 4. Output Differences

### `version`
- Go: `treb nightly-41-gc72d1b1` format
- Rust: `treb 0.1.0` format
- Rust has `--json` flag (Go does not)

### `networks`
- Go resolves env vars and shows chain IDs successfully
- Rust shows "Error: unresolved env var" for all networks (doesn't resolve `${VAR}` from `.env` or similar)
- JSON output schemas differ (Rust includes `status` field with error text)

### `config` (show)
- Go: key-value format with emoji headers, senders shown as `role  type  address`
- Rust: same header format but senders in a table (comfy_table with box drawing chars), addresses are empty (not resolved)

### `fork status` (no active forks)
- Go: `No active forks` (no period)
- Rust: `No active forks.` (with period)

### `dev anvil status`
- Go: Shows specific anvil instance info (`anvil0`, PID file, log file paths)
- Rust: `No active Anvil instances.` (different approach - in-process vs subprocess)

### Error format (unknown command)
- Go: `Error: unknown command "nonexistent" for "treb"`
- Rust: `error: unrecognized subcommand 'nonexistent'` (clap default)

---

## 5. Registry Compatibility (Blocker)

**Critical issue**: The Rust CLI cannot read the Go CLI's `.treb/` directory.

The Go CLI's `registry.json` has format: `{chainId: {namespace: {name: address}}}` (SolidityRegistry map).
The Rust CLI expects: `{version: N, createdAt: "...", updatedAt: "..."}` (RegistryMeta).

The Rust registry parser requires a `version` field and fails with:
```
registry error: failed to parse .treb/registry.json: missing field `version` at line 756 column 1
```

This blocks ALL registry-dependent commands:
- `list` / `ls`
- `show`
- `tag`
- `fork` subcommands (when fork state exists)

The other store files (`deployments.json`, `transactions.json`, `safe-txs.json`) may also have format differences but couldn't be tested due to this blocker.

---

## 6. Summary of Action Items

### Critical
1. **Registry compatibility**: Rust must handle Go's `registry.json` format (either ignore unknown fields or support both schemas)

### Missing Features
2. **`addressbook` command**: Entirely missing from Rust CLI
3. **Env var resolution in `networks`**: Rust doesn't resolve `${VAR}` env vars from foundry.toml

### Parity Gaps
4. **`--non-interactive` as global flag**: Go has it global, Rust has it per-command
5. **`config` with no subcommand**: Go shows current config, Rust requires explicit `show`
6. **`gen deploy` vs `gen-deploy`**: Different command structure
7. **`completion` vs `completions`**: Naming difference
8. **Fork commands**: Go uses positional args for network, Rust uses `--network` flag
9. **`list` aliases**: Go has `ls`, needs testing in Rust
10. **`show` missing filters**: Rust lacks `--namespace`, `--network`, `--no-fork`

### Rust Has But Go Doesn't (Extras)
11. `version --json`
12. Various `--json` flags on commands Go lacks them (show, tag, sync, prune, reset, register, fork subcommands)
13. `--dry-run` on `prune`
14. `-y/--yes` on `prune` and `reset`
15. `--check-onchain` on `prune`
16. `migrate` has `config` and `registry` subcommands (Go is flat)
17. `fork enter --fork-block-number`
18. `verify --watch`, `--retries`, `--delay`, `--verifier-api-key`
