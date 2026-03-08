mod commands;
mod output;
mod ui;

use clap::{CommandFactory, Parser, Subcommand};
use treb_core::types::DeploymentType;

/// Parse a deployment type string (case-insensitive).
fn parse_deployment_type(s: &str) -> Result<DeploymentType, String> {
    match s.to_lowercase().as_str() {
        "proxy" => Ok(DeploymentType::Proxy),
        "singleton" => Ok(DeploymentType::Singleton),
        "library" => Ok(DeploymentType::Library),
        "unknown" => Ok(DeploymentType::Unknown),
        _ => Err(format!(
            "invalid deployment type '{s}'; valid values: proxy, singleton, library, unknown"
        )),
    }
}

/// treb — deployment orchestration for Foundry projects
#[derive(Parser)]
#[command(name = "treb", version, about)]
struct Cli {
    /// Disable colored output (also respected via NO_COLOR env var)
    #[arg(long, global = true)]
    no_color: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Execute a deployment script
    ///
    /// Runs a Forge script and records deployments to the treb registry.
    /// Supports dry-run mode, broadcast, legacy transactions, and interactive
    /// network selection when no --network flag is provided.
    Run {
        /// Path to the Forge script (e.g., script/Deploy.s.sol)
        script: String,
        /// Function signature to call
        #[arg(long, default_value = "run()")]
        sig: String,
        /// Arguments to pass to the script function
        #[arg(long, num_args = 1..)]
        args: Vec<String>,
        /// Network name or chain ID
        #[arg(long)]
        network: Option<String>,
        /// Explicit RPC URL (overrides network)
        #[arg(long)]
        rpc_url: Option<String>,
        /// Deployment namespace
        #[arg(long)]
        namespace: Option<String>,
        /// Broadcast transactions to the network
        #[arg(long)]
        broadcast: bool,
        /// Simulate execution without recording to registry
        #[arg(long)]
        dry_run: bool,
        /// Send transactions one at a time
        #[arg(long)]
        slow: bool,
        /// Use legacy (pre-EIP-1559) transactions
        #[arg(long)]
        legacy: bool,
        /// Verify deployed contracts on Etherscan
        #[arg(long)]
        verify: bool,
        /// Show verbose output (labeled addresses, gas, config source)
        #[arg(long, short)]
        verbose: bool,
        /// Enable Forge debugger
        #[arg(long)]
        debug: bool,
        /// Print the equivalent forge script command and exit without executing
        #[arg(long)]
        dump_command: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
        /// Set environment variables (KEY=VALUE)
        #[arg(long, num_args = 1)]
        env: Vec<String>,
        /// Target contract to run (when multiple contracts in script)
        #[arg(long)]
        target_contract: Option<String>,
        /// Skip interactive confirmation prompts
        #[arg(long)]
        non_interactive: bool,
    },
    /// List deployments in the registry
    ///
    /// Displays all deployments stored in the treb registry with optional
    /// filters for network, namespace, type, tag, contract, and label.
    /// Alias: `ls`.
    #[command(alias = "ls")]
    List {
        /// Filter by network name or chain ID
        #[arg(long)]
        network: Option<String>,
        /// Filter by namespace
        #[arg(long)]
        namespace: Option<String>,
        /// Filter by deployment type (SINGLETON, PROXY, LIBRARY)
        #[arg(long, value_name = "TYPE")]
        r#type: Option<String>,
        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,
        /// Filter by contract name
        #[arg(long)]
        contract: Option<String>,
        /// Filter by deployment label
        #[arg(long)]
        label: Option<String>,
        /// Show only fork deployments (namespace starts with fork/)
        #[arg(long)]
        fork: bool,
        /// Hide fork deployments
        #[arg(long)]
        no_fork: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show detailed information about a specific deployment
    ///
    /// Displays the full deployment record including address, contract name,
    /// network, namespace, tags, verification status, and transaction details.
    /// Omit the deployment argument to select interactively with a fuzzy search.
    Show {
        /// Deployment identifier (full ID, name, address, name:label, or namespace/name); omit to
        /// select interactively
        deployment: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Initialize a treb project
    ///
    /// Creates the `.treb/` directory structure and writes a default `treb.toml`
    /// configuration file. Must be run inside a Foundry project (requires
    /// `foundry.toml`). Use `--force` to reinitialize an existing project.
    Init {
        /// Overwrite local config even if already initialized
        #[arg(long)]
        force: bool,
    },
    /// Manage treb configuration
    ///
    /// View or modify treb's local configuration (`.treb/treb.toml`). Supports
    /// `show` to display resolved config, `set` to update a key, and `remove`
    /// to reset a key to its default value.
    Config {
        #[command(subcommand)]
        subcommand: ConfigSubcommand,
    },
    /// Verify deployed contracts on block explorers
    ///
    /// Submits contract source code to a verification provider (Etherscan,
    /// Sourcify, or Blockscout). Use `--all` to verify all unverified
    /// deployments in the registry. Omit the deployment argument to select
    /// interactively.
    Verify {
        /// Deployment identifier (full ID, name, address, name:label, or namespace/name)
        deployment: Option<String>,
        /// Verify all unverified deployments
        #[arg(long)]
        all: bool,
        /// Verification provider (etherscan, sourcify, blockscout)
        #[arg(long, default_value = "etherscan")]
        verifier: String,
        /// Verify on Etherscan
        #[arg(long)]
        etherscan: bool,
        /// Verify on Blockscout
        #[arg(long)]
        blockscout: bool,
        /// Verify on Sourcify
        #[arg(long)]
        sourcify: bool,
        /// Verifier API URL override
        #[arg(long)]
        verifier_url: Option<String>,
        /// Verifier API key
        #[arg(long)]
        verifier_api_key: Option<String>,
        /// Re-verify already verified contracts
        #[arg(long)]
        force: bool,
        /// Watch verification status until confirmed
        #[arg(long)]
        watch: bool,
        /// Number of retry attempts
        #[arg(long, default_value = "5")]
        retries: u32,
        /// Delay in seconds between retries
        #[arg(long, default_value = "5")]
        delay: u64,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Manage tags on a deployment
    ///
    /// Add or remove semantic version tags (e.g., `v1.0.0`) on a deployment
    /// record. Tags can be used to filter results with `treb list --tag`.
    /// Omit the deployment argument to select interactively.
    Tag {
        /// Deployment identifier (full ID, name, address, name:label, or namespace/name); omit to
        /// select interactively
        deployment: Option<String>,
        /// Add a tag to the deployment
        #[arg(long, conflicts_with = "remove")]
        add: Option<String>,
        /// Remove a tag from the deployment
        #[arg(long, conflicts_with = "add")]
        remove: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Register deployments from a historical transaction
    ///
    /// Traces an on-chain transaction to discover contract creations and adds
    /// them to the treb registry. Useful for importing deployments that were
    /// made outside of treb or from an older deployment script.
    Register {
        /// Transaction hash to trace for contract creations
        #[arg(long)]
        tx_hash: String,
        /// Network name or chain ID
        #[arg(long)]
        network: Option<String>,
        /// Explicit RPC URL (overrides network)
        #[arg(long)]
        rpc_url: Option<String>,
        /// Filter to a specific deployed address
        #[arg(long)]
        address: Option<String>,
        /// Contract artifact name to match
        #[arg(long)]
        contract: Option<String>,
        /// Contract name to narrow artifact matching
        #[arg(long)]
        contract_name: Option<String>,
        /// Label for registered deployments
        #[arg(long)]
        label: Option<String>,
        /// Deployment namespace
        #[arg(long)]
        namespace: Option<String>,
        /// Deployment type (proxy, singleton, library, unknown)
        #[arg(long, value_parser = parse_deployment_type)]
        deployment_type: Option<treb_core::types::DeploymentType>,
        /// Skip post-registration verification
        #[arg(long)]
        skip_verify: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Sync safe transaction state from the Safe Transaction Service
    ///
    /// Fetches the latest transaction status from the Safe Transaction Service
    /// API and updates the local registry with confirmations, rejections, and
    /// execution results for pending Safe multisig transactions.
    Sync {
        /// Filter sync to a specific network name or chain ID
        #[arg(long)]
        network: Option<String>,
        /// Remove safe transactions not found on the service
        #[arg(long)]
        clean: bool,
        /// Print raw API responses to stderr
        #[arg(long)]
        debug: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Print version information
    Version {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// List available networks
    Networks {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Generate deployment scripts from templates
    ///
    /// Scaffolds a Forge deployment script for a contract using a built-in
    /// template. Supports create, create2, and create3 strategies, as well as
    /// proxy patterns (ERC1967, UUPS, transparent, beacon).
    GenDeploy {
        /// Contract name or artifact identifier (e.g., Counter or src/Counter.sol:Counter)
        artifact: String,
        /// Deployment strategy: create, create2, create3
        #[arg(long)]
        strategy: Option<String>,
        /// Proxy pattern: erc1967, uups, transparent, beacon
        #[arg(long)]
        proxy: Option<String>,
        /// Custom proxy contract name (for non-standard proxy implementations)
        #[arg(long)]
        proxy_contract: Option<String>,
        /// Output file path (default: script/Deploy<Name>.s.sol)
        #[arg(long)]
        output: Option<String>,
        /// Output as JSON instead of writing a file
        #[arg(long)]
        json: bool,
    },
    /// Compose multi-step deployment pipelines
    ///
    /// Executes a YAML-defined multi-step deployment pipeline. Each step can
    /// run a Forge script, deploy libraries, or invoke arbitrary forge commands.
    /// Use `--dry-run` to preview the execution plan without running it.
    Compose {
        /// Path to the compose YAML file
        file: String,
        /// Network name or chain ID
        #[arg(long)]
        network: Option<String>,
        /// Explicit RPC URL (overrides network)
        #[arg(long)]
        rpc_url: Option<String>,
        /// Deployment namespace
        #[arg(long)]
        namespace: Option<String>,
        /// Foundry profile override
        #[arg(long)]
        profile: Option<String>,
        /// Broadcast transactions to the network
        #[arg(long)]
        broadcast: bool,
        /// Print execution plan without running
        #[arg(long)]
        dry_run: bool,
        /// Skip already-completed components
        #[arg(long)]
        resume: bool,
        /// Verify contracts after deployment
        #[arg(long)]
        verify: bool,
        /// Send transactions one at a time
        #[arg(long)]
        slow: bool,
        /// Use legacy (pre-EIP-1559) transactions
        #[arg(long)]
        legacy: bool,
        /// Show verbose output
        #[arg(long, short)]
        verbose: bool,
        /// Save per-component debug logs to .treb/debug-compose-<timestamp>/
        #[arg(long)]
        debug: bool,
        /// Print per-component forge commands and exit without executing
        #[arg(long)]
        dump_command: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
        /// Set environment variables (KEY=VALUE, repeatable)
        #[arg(long, num_args = 1)]
        env: Vec<String>,
        /// Skip interactive confirmation prompts
        #[arg(long)]
        non_interactive: bool,
    },
    /// Remove stale or broken registry entries
    ///
    /// Scans the deployment registry for broken cross-references (e.g., a
    /// deployment pointing to a missing transaction) and removes them. Creates
    /// a timestamped backup before any destructive operation.
    Prune(commands::prune::PruneArgs),
    /// Reset registry state (with optional scope filters)
    ///
    /// Clears all deployments and transactions from the registry, optionally
    /// scoped to a specific network or namespace. Creates a timestamped backup
    /// before removing data.
    Reset(commands::reset::ResetArgs),
    /// Migrate config or registry to a newer format
    ///
    /// Handles forward migrations for both the `treb.toml` config file (v1→v2)
    /// and the deployment registry schema. Use `--dry-run` to preview changes
    /// without modifying any files.
    Migrate {
        #[command(subcommand)]
        subcommand: commands::migrate::MigrateSubcommand,
    },
    /// Fork a network for local testing
    ///
    /// Manages fork mode lifecycle: `enter` snapshots the registry and starts
    /// a fork, `exit` restores the registry, `revert` and `restart` manage
    /// snapshot state, and `status`/`history`/`diff` provide observability.
    Fork {
        #[command(subcommand)]
        subcommand: commands::fork::ForkSubcommand,
    },
    /// Start a local development environment
    Dev {
        #[command(subcommand)]
        subcommand: commands::dev::DevSubcommand,
    },
    /// Generate shell completion scripts
    ///
    /// Outputs a shell completion script for the specified shell to stdout.
    /// Source the output in your shell profile to enable tab-completion for
    /// all treb commands and flags.
    ///
    /// Examples:
    ///   treb completions bash >> ~/.bashrc
    ///   treb completions zsh > ~/.zsh/completions/_treb
    ///   treb completions fish > ~/.config/fish/completions/treb.fish
    Completions {
        /// Shell type: bash, zsh, fish, elvish, or powershell
        shell: String,
    },
}

#[derive(Subcommand)]
enum ConfigSubcommand {
    /// Display the resolved configuration
    Show {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Set a local configuration value
    Set {
        /// Configuration key (namespace, network)
        key: String,
        /// Value to set
        value: String,
    },
    /// Remove (reset) a local configuration value to its default
    Remove {
        /// Configuration key (namespace, network)
        key: String,
    },
}

impl Commands {
    /// Returns `true` when the parsed subcommand includes `--json`.
    fn json_flag(&self) -> bool {
        match self {
            Commands::Run { json, .. }
            | Commands::List { json, .. }
            | Commands::Show { json, .. }
            | Commands::Verify { json, .. }
            | Commands::Tag { json, .. }
            | Commands::Register { json, .. }
            | Commands::Sync { json, .. }
            | Commands::Version { json, .. }
            | Commands::Networks { json, .. }
            | Commands::GenDeploy { json, .. }
            | Commands::Compose { json, .. } => *json,
            Commands::Config { subcommand } => {
                matches!(subcommand, ConfigSubcommand::Show { json: true })
            }
            Commands::Prune(args) => args.json,
            Commands::Reset(args) => args.json,
            Commands::Fork { subcommand } => subcommand.json_flag(),
            Commands::Dev { subcommand } => subcommand.json_flag(),
            Commands::Migrate { subcommand } => subcommand.json_flag(),
            Commands::Init { .. } | Commands::Completions { .. } => false,
        }
    }
}

impl commands::fork::ForkSubcommand {
    fn json_flag(&self) -> bool {
        match self {
            Self::Status { json, .. }
            | Self::History { json, .. }
            | Self::Diff { json, .. }
            | Self::Enter { json, .. }
            | Self::Exit { json, .. }
            | Self::Revert { json, .. }
            | Self::Restart { json, .. } => *json,
        }
    }
}

impl commands::migrate::MigrateSubcommand {
    fn json_flag(&self) -> bool {
        match self {
            Self::Config { json, .. } => *json,
            Self::Registry { .. } => false,
        }
    }
}

impl commands::dev::DevSubcommand {
    fn json_flag(&self) -> bool {
        match self {
            Self::Anvil { subcommand } => subcommand.json_flag(),
        }
    }
}

impl commands::dev::AnvilSubcommand {
    fn json_flag(&self) -> bool {
        match self {
            Self::Status { json, .. } => *json,
            Self::Start { .. } | Self::Stop { .. } | Self::Restart { .. } | Self::Logs { .. } => {
                false
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Apply color settings before any output is produced.
    ui::color::color_enabled(cli.no_color);

    let json = cli.command.json_flag();

    if let Err(err) = run(cli).await {
        if json {
            output::print_json_error(&format!("{err:#}"));
        } else {
            // Reproduce the exact format that `main() -> anyhow::Result<()>` uses:
            // "Error: <debug repr>" which includes "Caused by:" chains.
            eprintln!("Error: {err:?}");
        }
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Commands::Run {
            script,
            sig,
            args,
            network,
            rpc_url,
            namespace,
            broadcast,
            dry_run,
            slow,
            legacy,
            verify,
            verbose,
            debug,
            dump_command,
            json,
            env,
            target_contract,
            non_interactive,
        } => {
            commands::run::run(
                &script,
                &sig,
                args,
                network,
                rpc_url,
                namespace,
                broadcast,
                dry_run,
                slow,
                legacy,
                verify,
                verbose,
                debug,
                dump_command,
                json,
                env,
                target_contract,
                non_interactive,
            )
            .await?
        }
        Commands::List {
            network,
            namespace,
            r#type,
            tag,
            contract,
            label,
            fork,
            no_fork,
            json,
        } => {
            commands::list::run(
                network, namespace, r#type, tag, contract, label, fork, no_fork, json,
            )
            .await?
        }
        Commands::Show { deployment, json } => commands::show::run(deployment, json).await?,
        Commands::Init { force } => commands::init::run(force).await?,
        Commands::Config { subcommand } => match subcommand {
            ConfigSubcommand::Show { json } => commands::config::show(json).await?,
            ConfigSubcommand::Set { key, value } => commands::config::set(&key, &value).await?,
            ConfigSubcommand::Remove { key } => commands::config::remove(&key).await?,
        },
        Commands::Verify {
            deployment,
            all,
            verifier,
            etherscan,
            blockscout,
            sourcify,
            verifier_url,
            verifier_api_key,
            force,
            watch,
            retries,
            delay,
            json,
        } => {
            // Shorthand flags override --verifier when any are specified.
            let verifiers = if etherscan || blockscout || sourcify {
                let mut v = Vec::new();
                if etherscan {
                    v.push("etherscan".to_string());
                }
                if blockscout {
                    v.push("blockscout".to_string());
                }
                if sourcify {
                    v.push("sourcify".to_string());
                }
                v
            } else {
                vec![verifier]
            };

            commands::verify::run(
                deployment,
                all,
                &verifiers,
                verifier_url,
                verifier_api_key,
                force,
                watch,
                retries,
                delay,
                json,
            )
            .await?
        }
        Commands::Tag { deployment, add, remove, json } => {
            commands::tag::run(deployment, add, remove, json).await?
        }
        Commands::Register {
            tx_hash,
            network,
            rpc_url,
            address,
            contract,
            contract_name,
            label,
            namespace,
            deployment_type,
            skip_verify,
            json,
        } => {
            commands::register::run(
                &tx_hash,
                network,
                rpc_url,
                address,
                contract,
                contract_name,
                label,
                namespace,
                deployment_type,
                skip_verify,
                json,
            )
            .await?
        }
        Commands::Sync { network, clean, debug, json } => {
            commands::sync::run(network, clean, debug, json).await?
        }
        Commands::Version { json } => commands::version::run(json).await?,
        Commands::Networks { json } => commands::networks::run(json).await?,
        Commands::GenDeploy { artifact, strategy, proxy, proxy_contract, output, json } => {
            commands::gen_deploy::run(
                &artifact,
                strategy.as_deref(),
                proxy.as_deref(),
                proxy_contract.as_deref(),
                output.as_deref(),
                json,
            )
            .await?
        }
        Commands::Compose {
            file,
            network,
            rpc_url,
            namespace,
            profile,
            broadcast,
            dry_run,
            resume,
            verify,
            slow,
            legacy,
            verbose,
            debug,
            dump_command,
            json,
            env,
            non_interactive,
        } => {
            commands::compose::run(
                file,
                network,
                rpc_url,
                namespace,
                profile,
                broadcast,
                dry_run,
                resume,
                verify,
                slow,
                legacy,
                verbose,
                debug,
                dump_command,
                json,
                env,
                non_interactive,
            )
            .await?
        }
        Commands::Prune(args) => commands::prune::run(args).await?,
        Commands::Reset(args) => commands::reset::run(args).await?,
        Commands::Migrate { subcommand } => commands::migrate::run(subcommand).await?,
        Commands::Fork { subcommand } => commands::fork::run(subcommand).await?,
        Commands::Dev { subcommand } => commands::dev::run(subcommand).await?,
        Commands::Completions { shell } => {
            use clap_complete::{Shell, generate};
            use std::{io, str::FromStr};

            let shell = Shell::from_str(&shell).map_err(|_| {
                anyhow::anyhow!(
                    "unsupported shell '{}'; supported shells: bash, zsh, fish, elvish, powershell",
                    shell
                )
            })?;
            let mut cmd = Cli::command();
            generate(shell, &mut cmd, "treb", &mut io::stdout());
        }
    }

    Ok(())
}
