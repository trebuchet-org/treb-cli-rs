mod commands;
mod output;

use clap::{Parser, Subcommand};

/// treb — deployment orchestration for Foundry projects
#[derive(Parser)]
#[command(name = "treb", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Execute a deployment script
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
    Show {
        /// Deployment identifier (full ID, name, address, name:label, or namespace/name)
        deployment: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Initialize a treb project
    Init {
        /// Overwrite local config even if already initialized
        #[arg(long)]
        force: bool,
    },
    /// Manage treb configuration
    Config {
        #[command(subcommand)]
        subcommand: ConfigSubcommand,
    },
    /// Verify deployed contracts on block explorers
    Verify {
        /// Deployment identifier (full ID, name, address, name:label, or namespace/name)
        deployment: Option<String>,
        /// Verify all unverified deployments
        #[arg(long)]
        all: bool,
        /// Verification provider (etherscan, sourcify, blockscout)
        #[arg(long, default_value = "etherscan")]
        verifier: String,
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
    Tag {
        /// Deployment identifier (full ID, name, address, name:label, or namespace/name)
        deployment: String,
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
        /// Skip post-registration verification
        #[arg(long)]
        skip_verify: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Sync safe transaction state from the Safe Transaction Service
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
    /// Remove stale deployment artifacts
    Prune,
    /// Reset deployment state
    Reset,
    /// Run database migrations
    Migrate,
    /// Fork a network for local testing
    Fork {
        #[command(subcommand)]
        subcommand: commands::fork::ForkSubcommand,
    },
    /// Start a local development environment
    Dev {
        #[command(subcommand)]
        subcommand: commands::dev::DevSubcommand,
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

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
                json,
                env,
                target_contract,
                non_interactive,
            )
            .await?
        }
        Commands::List { network, namespace, r#type, tag, contract, label, fork, no_fork, json } => {
            commands::list::run(network, namespace, r#type, tag, contract, label, fork, no_fork, json).await?
        }
        Commands::Show { deployment, json } => {
            commands::show::run(&deployment, json).await?
        }
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
            verifier_url,
            verifier_api_key,
            force,
            watch,
            retries,
            delay,
            json,
        } => {
            commands::verify::run(
                deployment,
                all,
                &verifier,
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
            commands::tag::run(&deployment, add, remove, json).await?
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
                json,
                env,
                non_interactive,
            )
            .await?
        }
        Commands::Prune => println!("prune: not yet implemented"),
        Commands::Reset => println!("reset: not yet implemented"),
        Commands::Migrate => println!("migrate: not yet implemented"),
        Commands::Fork { subcommand } => commands::fork::run(subcommand).await?,
        Commands::Dev { subcommand } => commands::dev::run(subcommand).await?,
    }

    Ok(())
}
