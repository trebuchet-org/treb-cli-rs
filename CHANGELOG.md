# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.1.0]

### Added

- **Core commands**: `run`, `list` (alias `ls`), `show`, `init`, `version`, `networks` with full output parity with the Go CLI
- **Script execution**: In-process Foundry integration via `treb-forge` — no subprocess calls to `forge script`
- **Deployment recording**: Automatic deployment detection from ContractDeployed, SafeTransactionQueued, GovernorProposalCreated events with duplicate detection and resolution
- **Configuration**: `config show`, `config set`, `config remove` for treb.toml v2 format with `${VAR}` environment variable expansion
- **Migration**: `migrate config` (v1 → v2 with `--dry-run`, `--cleanup-foundry`) and `migrate registry` (schema version migrations)
- **Verification**: `verify` command supporting Etherscan, Blockscout, and Sourcify with `--all`, `--watch`, and retry options
- **Compose**: `compose` for multi-step deployment pipelines from YAML with dependency ordering, `--resume` for skipping completed components, and `--dry-run` execution plan preview
- **Fork mode**: `fork enter`, `fork exit`, `fork revert`, `fork restart`, `fork status`, `fork history`, `fork diff` — full registry snapshotting and EVM snapshot/revert coordination
- **Development tooling**: `dev anvil start`, `dev anvil stop`, `dev anvil restart`, `dev anvil status` — in-process Anvil node management with CreateX auto-deployment
- **Registry management**: `tag` (add/remove deployment tags), `register` (register historical deployments from transaction traces), `sync` (sync Safe transaction state from the Safe Transaction Service), `prune` (remove stale entries), `reset` (clear registry state with `--network`/`--namespace` scoping and timestamped backups)
- **Script generation**: `gen-deploy` for generating deployment scripts from templates with strategy and proxy pattern options
- **Safe multisig**: EIP-712 signing, transaction batching, and Safe Transaction Service integration via `treb-safe`
- **Governor proposals**: OpenZeppelin Governor and Timelock sender support with on-chain state tracking
- **treb-sol bindings**: Type-safe Rust bindings for treb Solidity interfaces (ITrebEvents, ICreateX, ProxyEvents) via alloy `sol!` macro
- **Output formatting**: Tree hierarchy, UTF-8 tables, color palette respecting `NO_COLOR`/`TERM=dumb`, `--json` flag with deterministic key-sorted output on all read commands
- **Non-interactive mode**: `--non-interactive` flag, `TREB_NON_INTERACTIVE`, `CI` env, stdin/stdout TTY detection; `--json --broadcast` requires `--non-interactive`
- **Shell completions**: Build-time generated completions for bash, zsh, fish, elvish; auto-installed by trebup
- **Build metadata**: `treb version --json` embeds git commit, build date, foundry version, treb-sol commit, rust version
- **Cross-platform releases**: Linux (x86_64, aarch64), macOS (x86_64, aarch64), Windows (x86_64) via GitHub Actions
- **Test suite**: 175 golden file snapshots, E2E workflow tests (deployment, fork, prune/reset, register, registry consistency), integration tests with in-process Anvil

### Changed

- **Configuration format**: treb.toml v1 replaced by v2 with `[accounts.*]` and `[namespaces.*]` sections; `treb migrate config` provides automated conversion
- **Registry schema**: Versioned schema with automatic migration detection on `Registry::open()`
