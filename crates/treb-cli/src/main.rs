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
    Run,
    /// List deployments
    #[command(alias = "ls")]
    List,
    /// Show deployment details
    Show,
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
    /// Verify deployed contracts
    Verify,
    /// Tag a deployment snapshot
    Tag,
    /// Register a contract in the registry
    Register,
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
        Commands::Run => println!("run: not yet implemented"),
        Commands::List => println!("list: not yet implemented"),
        Commands::Show => println!("show: not yet implemented"),
        Commands::Init { force } => commands::init::run(force).await?,
        Commands::Config { subcommand } => match subcommand {
            ConfigSubcommand::Show { json } => commands::config::show(json).await?,
            ConfigSubcommand::Set { key, value } => commands::config::set(&key, &value).await?,
            ConfigSubcommand::Remove { key } => commands::config::remove(&key).await?,
        },
        Commands::Verify => println!("verify: not yet implemented"),
        Commands::Tag => println!("tag: not yet implemented"),
        Commands::Register => println!("register: not yet implemented"),
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
