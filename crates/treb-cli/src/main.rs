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
    Init,
    /// Manage treb configuration
    Config,
    /// Verify deployed contracts
    Verify,
    /// Tag a deployment snapshot
    Tag,
    /// Register a contract in the registry
    Register,
    /// Sync deployment state from on-chain data
    Sync,
    /// Print version information
    Version,
    /// List available networks
    Networks,
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run => println!("run: not yet implemented"),
        Commands::List => println!("list: not yet implemented"),
        Commands::Show => println!("show: not yet implemented"),
        Commands::Init => println!("init: not yet implemented"),
        Commands::Config => println!("config: not yet implemented"),
        Commands::Verify => println!("verify: not yet implemented"),
        Commands::Tag => println!("tag: not yet implemented"),
        Commands::Register => println!("register: not yet implemented"),
        Commands::Sync => println!("sync: not yet implemented"),
        Commands::Version => println!("treb v{}", env!("CARGO_PKG_VERSION")),
        Commands::Networks => println!("networks: not yet implemented"),
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
