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
    /// Sync deployment state from on-chain data
    Sync,
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
    GenDeploy,
    /// Compose multi-step deployment pipelines
    Compose,
    /// Remove stale deployment artifacts
    Prune,
    /// Reset deployment state
    Reset,
    /// Run database migrations
    Migrate,
    /// Fork a network for local testing
    Fork,
    /// Start a local development environment
    Dev,
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
        Commands::Sync => println!("sync: not yet implemented"),
        Commands::Version { json } => commands::version::run(json).await?,
        Commands::Networks { json } => commands::networks::run(json).await?,
        Commands::GenDeploy => println!("gen-deploy: not yet implemented"),
        Commands::Compose => println!("compose: not yet implemented"),
        Commands::Prune => println!("prune: not yet implemented"),
        Commands::Reset => println!("reset: not yet implemented"),
        Commands::Migrate => println!("migrate: not yet implemented"),
        Commands::Fork => println!("fork: not yet implemented"),
        Commands::Dev => println!("dev: not yet implemented"),
    }

    Ok(())
}
