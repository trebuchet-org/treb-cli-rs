# Migrating from the Go CLI to the Rust CLI

This guide covers everything you need to switch from the Go `treb-cli` to the Rust `treb-cli-rs`. Read through the breaking changes and behavioral differences before migrating, then follow the checklist at the bottom.

## Breaking Changes

### treb.toml v1 format no longer supported

The Go CLI used a `[ns.*]` namespace-based config format (v1). The Rust CLI requires v2 format, which separates account definitions (`[accounts.*]`) from namespace role mappings (`[namespace.*]`).

**v1 (Go CLI):**

```toml
[ns.default]
profile = "default"

[ns.default.senders.deployer]
type = "private_key"
address = "0xDeployerAddr"
private_key = "${DEPLOYER_PRIVATE_KEY}"
```

**v2 (Rust CLI):**

```toml
[accounts.deployer]
type = "private_key"
address = "0xDeployerAddr"
private_key = "${DEPLOYER_PRIVATE_KEY}"

[namespace.default]
profile = "default"

[namespace.default.senders]
deployer = "deployer"
```

Run `treb migrate config` to convert automatically. The command creates a timestamped backup of your existing `treb.toml` before writing the v2 format.

### foundry.toml sender configuration is deprecated

The Go CLI allowed defining senders in `foundry.toml` under `[profile.*.treb.senders.*]`. The Rust CLI still falls back to those entries at runtime when no `treb.toml` is present, but that path is deprecated. Run `treb migrate config` to write them into `treb.toml` v2 format. After migration, `treb.toml` becomes the primary runtime config, and `--cleanup-foundry` can remove the deprecated sections from `foundry.toml`.

### Binary name

Both CLIs install as `treb`. The install path may differ — the Rust CLI installs to `~/.local/bin/treb` by default via `trebup`. Ensure the new binary is first on your `PATH`.

## Dropped Features

Full feature parity achieved. All Go CLI commands are available in the Rust CLI with equivalent or improved functionality.

## Behavioral Differences

### In-process forge execution

The Rust CLI runs Foundry's compiler and script executor in-process — no external `forge` binary is needed on `PATH`. This is faster but means forge compilation output is emitted directly rather than captured from a subprocess.

### JSON key sorting

All `--json` output uses alphabetically sorted keys for deterministic output. The Go CLI may have emitted keys in insertion order. If your scripts parse JSON positionally (e.g., with `head` or `cut`), switch to a proper JSON parser like `jq`.

### `treb run --json` stdout

When using `--json`, the final machine-readable JSON object is written to stdout. Forge compilation output may appear before the JSON object on stdout. Strip everything before the first `{` before passing the payload to `jq`, for example:

```sh
treb run script/Deploy.s.sol --json \
  | awk 'BEGIN{json=0} { if (!json) { i = index($0, "{"); if (i) { json = 1; print substr($0, i); } } else print }' \
  | jq .
```

Errors produce `{"error": "message"}` on stderr with exit code 1.

### `--json --broadcast` requires `--non-interactive`

Broadcasting with JSON output requires explicitly opting into non-interactive mode. This safety constraint prevents interactive confirmation prompts from corrupting JSON output. Pass `--non-interactive`, or set `TREB_NON_INTERACTIVE=true` or `CI=true`.

### Non-interactive mode detection

The Rust CLI enters non-interactive mode when **any** of these conditions hold:

1. `--non-interactive` flag is passed
2. `TREB_NON_INTERACTIVE=true` environment variable (case-insensitive)
3. `CI=true` environment variable (case-insensitive)
4. Standard input is not a TTY (piped or redirected)
5. Standard output is not a TTY (redirected)

The Go CLI may not have detected all of these conditions. CI pipelines and scripts that rely on TTY detection should work without changes, but verify with `--dry-run` first.

### Deployment ID format

Deployment IDs use the format `namespace/chainId/ContractName` (without label) or `namespace/chainId/ContractName:label` (with label). This matches the Go CLI format.

### `treb init` lazy file creation

`treb init` creates the `.treb/` directory and `config.local.json` but does not create `deployments.json` or other registry files until the first deployment. The Go CLI may have created empty registry files immediately. This is transparent to normal usage — commands that read the registry treat a missing file as empty.

### Reset scoping

`treb reset` supports `--network` and `--namespace` filters:

- `--network <CHAIN_ID>`: filters deployments, transactions, safe transactions, and governor proposals by chain ID
- `--namespace <NS>`: filters deployments by namespace; associated transactions are only removed if all their linked deployments are being removed
- Both filters can be combined

A timestamped backup is created under `.treb/backups/` before any deletion.

### Fork snapshot behavior

`fork enter` snapshots these registry files: `deployments.json`, `transactions.json`, `safe-txs.json`, `governor-txs.json`, and `lookup.json`. Files that do not exist at snapshot time are skipped. On `fork exit`, missing snapshot files cause the corresponding registry file to be deleted (restoring the "absent" state).

## Improvements

### In-process Foundry

No external `forge` binary dependency. Compilation and script execution run in-process, reducing overhead and simplifying installation.

### Governor sender support

New `oz_governor` account type enables creating governance proposals directly via `treb run`. Configure a governor account in `treb.toml`:

```toml
[accounts.gov]
type = "oz_governor"
governor = "0xGovernorAddr"
timelock = "0xTimelockAddr"
proposer = "proposer_account"
```

### Safe multisig integration

New `safe` account type proposes transactions to the Safe Transaction Service. Use `treb sync` to check proposal status.

```toml
[accounts.treasury]
type = "safe"
safe = "0xSafeAddr"
signer = "deployer"
```

### Shell completions

`trebup` auto-installs shell completions for bash, zsh, and fish. To generate manually:

```sh
treb completions bash  # or zsh, fish
```

### `treb version --json`

Outputs structured build metadata for reproducibility:

```json
{
  "version": "0.1.0",
  "commit": "abc1234",
  "date": "2026-03-08",
  "rustVersion": "1.85.0-nightly",
  "forgeVersion": "1.5.1",
  "foundryVersion": "1.5.1",
  "trebSolCommit": "def5678"
}
```

### Cross-platform releases

Pre-built binaries available for linux-amd64, linux-arm64, darwin-amd64, and darwin-arm64 via `trebup` or GitHub Releases.

### Dev Anvil management

New `treb dev anvil` subcommands manage local Anvil nodes:

```sh
treb dev anvil start --network mainnet --port 8545
treb dev anvil status
treb dev anvil stop --network mainnet
```

### Fork mode

Full fork lifecycle management with `treb fork enter`, `fork exit`, `fork revert`, `fork restart`, `fork status`, `fork history`, and `fork diff`.

## Registry Compatibility

The Rust CLI reads and writes the same `.treb/` directory structure. Registry files (`deployments.json`, `transactions.json`, etc.) are forward-compatible — the Rust CLI may add new fields that the Go CLI will ignore via serde defaults. The Rust CLI also introduces new registry files (`safe-txs.json`, `governor-txs.json`, `lookup.json`, `fork.json`) that the Go CLI does not use.

The `registry.json` metadata file differs between CLIs: the Go CLI stores a `SolidityRegistry` map while the Rust CLI ignores that file. This does not affect deployment data.

Registry store files upgrade automatically when the Rust CLI rewrites them. There is no separate `treb migrate registry` step.

## Migration Checklist

1. **Install the Rust CLI**

   ```sh
   curl -fsSL https://raw.githubusercontent.com/trebuchet-org/treb-cli-rs/main/scripts/trebup | sh
   ```

2. **Migrate config** (if using v1 treb.toml or foundry.toml senders)

   ```sh
   treb migrate config --dry-run    # preview changes
   treb migrate config              # apply conversion
   treb migrate config --cleanup-foundry  # also remove deprecated foundry.toml sections
   ```

3. **Verify deployments**

   ```sh
   treb list                        # confirm all deployments are visible
   ```

4. **Test with dry run**

   ```sh
   treb run script/Deploy.s.sol --network sepolia --dry-run
   ```

6. **Update CI scripts** — if your CI uses `--json --broadcast`, add `--non-interactive` (or set `CI=true`, which most CI providers do automatically)
