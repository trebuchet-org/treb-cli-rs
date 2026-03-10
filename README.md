# treb

Deployment orchestration CLI for Foundry projects. treb manages the full lifecycle of smart contract deployments — from script execution to registry tracking, verification, and multi-step pipelines — with in-process Foundry integration (no external `forge` binary required).

> **Migrating from the Go CLI?** See [MIGRATION.md](MIGRATION.md).
> **Full feature history:** See [CHANGELOG.md](CHANGELOG.md).

## Installation

### Using trebup (recommended)

```sh
curl -fsSL https://raw.githubusercontent.com/trebuchet-org/treb-cli-rs/main/scripts/trebup | sh
```

`trebup` downloads the latest release for your platform, verifies checksums, and auto-installs shell completions for bash, zsh, and fish.

| Variable | Default | Description |
|---|---|---|
| `TREB_VERSION` | latest | Install a specific release tag |
| `TREB_INSTALL_DIR` | `~/.local/bin` | Installation directory |

### Building from Source

Requires **Rust nightly** (a `rust-toolchain.toml` is included).

```sh
git clone --recurse-submodules https://github.com/trebuchet-org/treb-cli-rs.git
cd treb-cli-rs
cargo build --release
```

The binary is `target/release/treb-cli` (renamed to `treb` in release packaging).

## Quick Start

**1. Initialize a project** (requires an existing `foundry.toml`):

```sh
treb init
```

Example output:

```text
Initialized treb project at <PROJECT_ROOT>/.treb
Run `treb config show` to view your configuration.
```

This creates the `.treb/` directory for local state such as `config.local.json` and `registry.json`.

**2. Write a deploy script** (e.g., `script/Deploy.s.sol`):

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "forge-std/Script.sol";
import "../src/Counter.sol";

contract Deploy is Script {
    function run() public {
        vm.startBroadcast();
        new Counter();
        vm.stopBroadcast();
    }
}
```

**3. Dry run** (simulate without recording to registry):

```sh
treb run script/Deploy.s.sol --network sepolia --dry-run
```

Example output:

```text
No files changed, compilation skipped
🚧 [DRY RUN] No changes were written to the registry.
...
🔨 Compiling and executing script/Deploy.s.sol...
🧪 Simulating...
✅ Execution complete.
```

**4. Broadcast** (submit transactions and record deployments):

```sh
treb run script/Deploy.s.sol --network sepolia --broadcast
```

Example output:

```text
No files changed, compilation skipped
...
1 deployment recorded, 1 transaction, 160,053 gas used
🔨 Compiling and executing script/Deploy.s.sol...
📡 Broadcasting...
✅ Execution complete.
```

## Command Reference

| Command | Description | Key Flags |
|---|---|---|
| `run` | Execute a deployment script | `--broadcast`, `--dry-run`, `--network`, `--json`, `--verify` |
| `list` | List deployments in the registry | `--network`, `--namespace`, `--contract`, `--json` |
| `show` | Show details for a specific deployment | `--json` |
| `init` | Initialize a treb project | `--force` |
| `verify` | Verify contracts on block explorers | `--all`, `--verifier`, `--watch`, `--json` |
| `tag` | Manage deployment tags | `--add`, `--remove`, `--json` |
| `register` | Register deployments from a historical tx | `--tx-hash`, `--network`, `--json` |
| `sync` | Sync Safe transaction state | `--network`, `--clean`, `--json` |
| `version` | Print version information | `--json` |
| `networks` | List available networks | `--json` |
| `gen-deploy` | Generate deployment scripts from templates | `--strategy`, `--proxy`, `--json` |
| `compose` | Compose multi-step deployment pipelines | `--broadcast`, `--dry-run`, `--resume`, `--json` |
| `prune` | Remove stale or broken registry entries | `--dry-run`, `--check-onchain`, `--json` |
| `reset` | Reset registry state | `--network`, `--namespace`, `--json` |
| `completions` | Generate shell completion scripts | `bash`, `zsh`, `fish` |

### config

| Subcommand | Description |
|---|---|
| `config show` | Display resolved configuration (`--json`) |
| `config set` | Set a local configuration value |
| `config remove` | Reset a local configuration value to default |

### migrate

| Subcommand | Description |
|---|---|
| `migrate config` | Convert treb.toml v1 to v2 (`--dry-run`, `--json`) |

Registry store files upgrade automatically when a command rewrites them; there is no separate `migrate registry` subcommand.

### fork

| Subcommand | Description |
|---|---|
| `fork enter` | Snapshot registry and enter fork mode (`--network`) |
| `fork exit` | Restore registry and exit fork mode (`--network`, `--json`) |
| `fork revert` | Restore fork to last snapshot (`--network`, `--all`, `--json`) |
| `fork restart` | Reset Anvil node to fresh fork (`--network`, `--json`) |
| `fork status` | Show active fork status (`--json`) |
| `fork history` | Show fork lifecycle events (`--network`, `--json`) |
| `fork diff` | Show deployments changed since fork entered (`--network`, `--json`) |

### dev anvil

| Subcommand | Description |
|---|---|
| `dev anvil start` | Start a local Anvil node (`--network`, `--port`) |
| `dev anvil stop` | Remove stale tracked Anvil entries whose port is unreachable (`--network`, `--name`) |
| `dev anvil restart` | Restart an Anvil instance (`--network`, `--port`) |
| `dev anvil status` | Show Anvil node status (`--json`) |
| `dev anvil logs` | Display Anvil log output (`--follow`) |

## Configuration

treb uses root-level `treb.toml` (v2 format) for project configuration. The `.treb/` directory stores local state such as `config.local.json`, `registry.json`, fork snapshots, and Anvil metadata.

```toml
[accounts.deployer]
type = "private_key"
address = "0xDeployerAddr"
private_key = "${DEPLOYER_PRIVATE_KEY}"

[accounts.ledger_signer]
type = "ledger"
address = "0xLedgerAddr"
derivation_path = "m/44'/60'/0'/0/0"

[namespace.default]
profile = "default"

[namespace.default.senders]
deployer = "deployer"

[namespace.production]
profile = "optimized"

[namespace.production.senders]
deployer = "ledger_signer"

[fork]
setup = "script/ForkSetup.s.sol"
```

Account types: `private_key`, `ledger`, `safe` (Safe multisig), `oz_governor` (OpenZeppelin Governor).

## Environment Variables

| Variable | Effect |
|---|---|
| `NO_COLOR` | Disable colored output (any value) |
| `TREB_NON_INTERACTIVE` | Suppress interactive prompts (`true`) |
| `CI` | Enable CI mode — non-interactive (`true`) |

Non-interactive mode is also triggered when stdin or stdout is not a TTY (e.g., piped or redirected).

## JSON Output

Most commands support `--json` for machine-readable output:

- Keys are alphabetically sorted for deterministic output
- Errors produce `{"error": "message"}` on stderr with exit code 1
- `--json --broadcast` requires `--non-interactive` (safety constraint)

## License

MIT OR Apache-2.0
