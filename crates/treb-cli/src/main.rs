use std::{
    env,
    ffi::{OsStr, OsString},
};

use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand};
use treb_cli::{commands, output, ui};
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

/// Smart contract deployment orchestrator for Foundry
#[derive(Parser)]
#[command(
    name = "treb",
    bin_name = "treb",
    version,
    about,
    long_about = "Trebuchet (treb) orchestrates Foundry script execution for deterministic smart contract deployments using CreateX factory contracts."
)]
struct Cli {
    /// Disable colored output (also respected via NO_COLOR env var)
    #[arg(long, global = true)]
    no_color: bool,

    /// Skip interactive prompts (also enabled via TREB_NON_INTERACTIVE=1, CI=true, or non-TTY
    /// stdin/stdout)
    #[arg(long, global = true)]
    non_interactive: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a Foundry script with treb infrastructure
    ///
    /// Run a Foundry script with automatic sender configuration and event tracking.
    ///
    /// This command executes Foundry scripts while:
    /// - Automatically configuring senders based on your treb configuration
    /// - Parsing deployment events from script execution
    #[command(verbatim_doc_comment)]
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
        /// Broadcast transactions to the network (requires --non-interactive when used with
        /// --json)
        #[arg(long)]
        broadcast: bool,
        /// Simulate execution without making changes
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
        /// Show verbose output
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
    },
    /// List deployments from registry
    ///
    /// List all deployments from the registry.
    ///
    /// The list can be filtered by namespace, chain ID, contract name, label, or deployment type.
    ///
    /// In fork mode, deployments added during the fork are marked with [fork].
    #[command(alias = "ls")]
    List {
        /// Network name or chain ID
        #[arg(long, short = 'n')]
        network: Option<String>,
        /// Deployment namespace
        #[arg(long, short = 's')]
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
    /// Show detailed deployment information from registry
    ///
    /// Show detailed information about a specific deployment.
    ///
    /// You can specify deployments using:
    /// - Contract name: "Counter"
    /// - Contract with label: "Counter:v2"
    #[command(verbatim_doc_comment)]
    Show {
        /// Deployment identifier (full ID, name, address, name:label, or namespace/name); omit to
        /// select interactively
        deployment: Option<String>,
        /// Deployment namespace
        #[arg(long)]
        namespace: Option<String>,
        /// Network name or chain ID
        #[arg(long)]
        network: Option<String>,
        /// Hide fork deployments
        #[arg(long)]
        no_fork: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Initialize treb in a Foundry project
    ///
    /// Initialize treb in an existing Foundry project by installing dependencies
    /// and creating the deployment registry.
    Init {
        /// Overwrite local config even if already initialized
        #[arg(long)]
        force: bool,
    },
    /// Manage treb local config
    ///
    /// Manage treb local config stored in .treb/config.local.json
    ///
    /// The config defines default values for namespace and network that are used
    /// when these flags are not explicitly provided.
    ///
    /// When no subcommand is provided, behaves like `treb config show`.
    #[command(override_usage = "treb config [OPTIONS] [COMMAND]")]
    Config {
        #[command(subcommand)]
        subcommand: ConfigSubcommand,
    },
    /// Verify contracts on block explorers
    ///
    /// Verify contracts on block explorers (Etherscan, Blockscout, and Sourcify)
    /// and update registry status.
    ///
    /// Examples:
    ///   treb verify Counter                      # Verify specific contract (all verifiers)
    ///   treb verify Counter -e                   # Verify on Etherscan only
    #[command(verbatim_doc_comment)]
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
        #[arg(long, short = 'e')]
        etherscan: bool,
        /// Verify on Blockscout
        #[arg(long, short = 'b')]
        blockscout: bool,
        /// Verify on Sourcify
        #[arg(long, short = 's')]
        sourcify: bool,
        /// Verifier API URL override
        #[arg(long, visible_alias = "blockscout-verifier-url")]
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
    /// Manage deployment tags
    ///
    /// Add or remove version tags on deployments. Without flags, shows current tags.
    ///
    /// Examples:
    ///   treb tag Counter:v1                  # Show current tags
    #[command(verbatim_doc_comment)]
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
        /// Network name or chain ID
        #[arg(long, short = 'n')]
        network: Option<String>,
        /// Deployment namespace
        #[arg(long, short = 's')]
        namespace: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Register an existing contract deployment in the registry
    ///
    /// Register a contract that was deployed outside of treb so it can be used
    /// with registry lookups.
    ///
    /// This command allows you to add existing deployments to the treb registry.
    /// You can provide either:
    /// - A transaction hash (and treb will trace the transaction to find all contract creations)
    /// - Explicit parameters (address, contract path, transaction hash)
    #[command(verbatim_doc_comment)]
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
    /// Sync registry with on-chain state
    ///
    /// Update deployment registry with latest on-chain information. Checks
    /// pending Safe transactions and updates their execution status.
    ///
    /// This command will:
    /// - Check all pending Safe transactions for execution status
    #[command(verbatim_doc_comment)]
    Sync {
        /// Network name or chain ID
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
    /// Print the version number of treb
    Version {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// List available networks from foundry.toml
    ///
    /// List all networks configured in the [rpc_endpoints] section of foundry.toml.
    ///
    /// This command shows all available networks and attempts to fetch their chain IDs.
    Networks {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Generate deployment scripts
    #[command(visible_alias = "generate")]
    Gen {
        #[command(subcommand)]
        subcommand: GenSubcommand,
    },
    #[command(name = "gen-deploy", hide = true)]
    GenDeployCompat(GenDeployArgs),
    /// Execute orchestrated deployments from a YAML configuration
    ///
    /// Execute multiple deployment scripts in dependency order based on a YAML
    /// configuration file.
    ///
    /// The composes file defines components, their deployment scripts, dependencies,
    /// and environment variables. Treb will build a dependency graph and execute
    /// scripts in the correct order.
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
        /// Broadcast transactions to the network (requires --non-interactive when used with
        /// --json)
        #[arg(long)]
        broadcast: bool,
        /// Simulate execution without making changes
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
    },
    /// Prune registry entries that no longer exist on-chain
    ///
    /// Prune registry entries that no longer exist on-chain.
    ///
    /// This command checks all deployments, transactions, and safe transactions
    /// against the blockchain and removes entries that no longer exist. This is
    /// useful for cleaning up after test deployments on local or virtual networks.
    Prune(commands::prune::PruneArgs),
    /// Reset registry entries for the current namespace and network
    ///
    /// Reset registry entries for the current namespace and network.
    ///
    /// This command deletes all deployments, transactions, and safe transactions
    /// matching the current namespace and network from the registry. This is useful
    /// for cleaning up and starting fresh on a given namespace/network combination.
    Reset(commands::reset::ResetArgs),
    /// Migrate config to new treb.toml accounts/namespace format
    ///
    /// Migrate treb sender configuration from foundry.toml [profile.*.treb.*]
    /// sections into the new treb.toml format with [accounts.*] and [namespace.*]
    /// sections.
    ///
    /// This command will:
    /// 1. Read all [profile.*.treb.*] sections from foundry.toml
    /// 2. Deduplicate identical sender configs into shared accounts
    /// 3. Map profile names to namespaces with role->account mappings
    /// 4. Show a preview of the generated treb.toml
    /// 5. Ask for confirmation before writing
    #[command(verbatim_doc_comment)]
    Migrate {
        #[command(subcommand)]
        subcommand: commands::migrate::MigrateSubcommand,
    },
    /// Manage network fork mode
    ///
    /// Fork mode lets you test deployment scripts against local forks of live
    /// networks with snapshot/revert workflow.
    Fork {
        #[command(subcommand)]
        subcommand: commands::fork::ForkSubcommand,
    },
    /// Development utilities
    ///
    /// Development utilities for troubleshooting treb configuration and environment.
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
    ///   treb completion bash >> ~/.bashrc
    ///   treb completion zsh > ~/.zsh/completions/_treb
    ///   treb completion fish > ~/.config/fish/completions/treb.fish
    #[command(verbatim_doc_comment)]
    Completion {
        /// Shell type: bash, zsh, fish, elvish, or powershell
        shell: String,
    },
    #[command(name = "completions", hide = true)]
    CompletionCompat {
        /// Shell type: bash, zsh, fish, elvish, or powershell
        shell: String,
    },
}

#[derive(Subcommand)]
enum ConfigSubcommand {
    /// Show the resolved configuration
    Show {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Set a config value
    Set {
        /// Configuration key (namespace, network)
        key: String,
        /// Value to set
        value: String,
    },
    /// Remove a config value
    Remove {
        /// Configuration key (namespace, network)
        key: String,
    },
}

#[derive(Subcommand)]
enum GenSubcommand {
    /// Generate deployment scripts for contracts and libraries.
    ///
    /// This command creates template scripts using treb-sol's base contracts.
    /// The generated scripts handle both direct deployments and common proxy patterns.
    #[command(verbatim_doc_comment)]
    Deploy(GenDeployArgs),
}

#[derive(Args)]
struct GenDeployArgs {
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
            | Commands::Compose { json, .. } => *json,
            Commands::Gen { subcommand } => gen_subcommand_json_flag(subcommand),
            Commands::GenDeployCompat(args) => args.json,
            Commands::Config { subcommand } => {
                matches!(subcommand, ConfigSubcommand::Show { json: true })
            }
            Commands::Prune(args) => args.json,
            Commands::Reset(args) => args.json,
            Commands::Fork { subcommand } => fork_subcommand_json_flag(subcommand),
            Commands::Dev { subcommand } => dev_subcommand_json_flag(subcommand),
            Commands::Migrate { subcommand } => migrate_subcommand_json_flag(subcommand),
            Commands::Init { .. }
            | Commands::Completion { .. }
            | Commands::CompletionCompat { .. } => false,
        }
    }
}

fn gen_subcommand_json_flag(subcommand: &GenSubcommand) -> bool {
    match subcommand {
        GenSubcommand::Deploy(args) => args.json,
    }
}

fn fork_subcommand_json_flag(subcommand: &commands::fork::ForkSubcommand) -> bool {
    match subcommand {
        commands::fork::ForkSubcommand::Status { json, .. }
        | commands::fork::ForkSubcommand::History { json, .. }
        | commands::fork::ForkSubcommand::Diff { json, .. }
        | commands::fork::ForkSubcommand::Enter { json, .. }
        | commands::fork::ForkSubcommand::Exit { json, .. }
        | commands::fork::ForkSubcommand::Revert { json, .. }
        | commands::fork::ForkSubcommand::Restart { json, .. } => *json,
    }
}

fn migrate_subcommand_json_flag(subcommand: &commands::migrate::MigrateSubcommand) -> bool {
    match subcommand {
        commands::migrate::MigrateSubcommand::Config { json, .. } => *json,
    }
}

fn dev_subcommand_json_flag(subcommand: &commands::dev::DevSubcommand) -> bool {
    match subcommand {
        commands::dev::DevSubcommand::Anvil { subcommand } => {
            anvil_subcommand_json_flag(subcommand)
        }
    }
}

fn anvil_subcommand_json_flag(subcommand: &commands::dev::AnvilSubcommand) -> bool {
    match subcommand {
        commands::dev::AnvilSubcommand::Status { json, .. } => *json,
        commands::dev::AnvilSubcommand::Start { .. }
        | commands::dev::AnvilSubcommand::Stop { .. }
        | commands::dev::AnvilSubcommand::Restart { .. }
        | commands::dev::AnvilSubcommand::Logs { .. } => false,
    }
}

/// Build a CLI command with grouped subcommands for help display.
///
/// Clap doesn't natively support multiple subcommand groups. This function takes the
/// derived `Cli::command()`, hides all subcommands from the default rendering, and
/// injects a custom grouped help text via `after_help` with a custom `help_template`.
fn build_grouped_command() -> clap::Command {
    let mut cmd = Cli::command().bin_name("treb");

    // Build grouped help text from subcommand metadata before hiding them
    let grouped_help = build_grouped_help(&cmd);

    // Hide all subcommands from default {subcommands} rendering
    let names: Vec<String> = cmd.get_subcommands().map(|s| s.get_name().to_string()).collect();
    for name in &names {
        cmd = cmd.mut_subcommand(name, |s| s.hide(true));
    }

    cmd.after_help(grouped_help).override_usage("treb [OPTIONS] <COMMAND>").help_template(
        "Smart contract deployment orchestrator for Foundry\n\
         \n\
         {usage-heading} {usage}\
         {after-help}\n\
         \nOptions:\n\
         {options}\n\
         Use \"treb [command] --help\" for more information about a command.\n",
    )
}

fn build_grouped_help(cmd: &clap::Command) -> String {
    const BUILTIN_HELP_SUBCOMMAND_ABOUT: &str =
        "Print this message or the help of the given subcommand(s)";

    let mut s = String::new();

    fn write_group(s: &mut String, cmd: &clap::Command, heading: &str, names: &[&str]) {
        s.push_str(heading);
        s.push('\n');
        for name in names {
            if let Some(sub) = cmd.find_subcommand(name) {
                let mut about = sub.get_about().map(|a| a.to_string()).unwrap_or_default();
                if *name == "gen" {
                    about.push_str(" (alias: generate)");
                }
                s.push_str(&format!("  {name:<14}{about}\n"));
            } else if *name == "help" && !cmd.is_disable_help_subcommand_set() {
                s.push_str(&format!("  {name:<14}{BUILTIN_HELP_SUBCOMMAND_ABOUT}\n"));
            }
        }
    }

    write_group(
        &mut s,
        cmd,
        "Main Commands:",
        &["init", "list", "show", "gen", "run", "verify", "compose", "fork"],
    );
    s.push('\n');
    write_group(
        &mut s,
        cmd,
        "Management Commands:",
        &["sync", "tag", "register", "dev", "networks", "prune", "reset", "config", "migrate"],
    );
    s.push('\n');
    write_group(&mut s, cmd, "Additional Commands:", &["version", "completion", "help"]);

    // Remove trailing newline so the template controls spacing
    if s.ends_with('\n') {
        s.pop();
    }

    s
}

fn argv_requests_flag(flag: &str) -> bool {
    let prefix = format!("{flag}=");
    env::args_os().skip(1).take_while(|arg| arg != "--").any(|arg| {
        let arg = arg.to_string_lossy();
        arg == flag || arg.starts_with(&prefix)
    })
}

fn normalize_cli_args<I, T>(args: I) -> Vec<OsString>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let mut args: Vec<OsString> = args.into_iter().map(Into::into).collect();

    if let Some(index) = config_show_insertion_index(&args) {
        args.insert(index, OsString::from("show"));
    }

    args
}

fn config_show_insertion_index(args: &[OsString]) -> Option<usize> {
    let mut config_index = 1;
    while let Some(arg) = args.get(config_index) {
        if arg == OsStr::new("--") {
            return None;
        }

        if matches!(arg.to_str(), Some("-h" | "--help" | "-V" | "--version")) {
            return None;
        }

        if !arg.to_string_lossy().starts_with('-') {
            break;
        }

        config_index += 1;
    }

    if args.get(config_index).map(OsString::as_os_str) != Some(OsStr::new("config")) {
        return None;
    }

    let rest = &args[(config_index + 1)..];
    if rest.is_empty() {
        return Some(config_index + 1);
    }

    if rest.iter().any(|arg| matches!(arg.to_str(), Some("-h" | "--help"))) {
        return None;
    }

    (!rest.iter().any(|arg| !arg.to_string_lossy().starts_with('-'))).then_some(config_index + 1)
}

fn parse_cli_from<I, T>(args: I) -> Result<Cli, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let matches = build_grouped_command().try_get_matches_from(normalize_cli_args(args))?;
    Ok(Cli::from_arg_matches(&matches).expect("bug: derive/builder arg mismatch"))
}

#[tokio::main]
async fn main() {
    let no_color_requested = argv_requests_flag("--no-color");
    ui::color::color_enabled(no_color_requested);

    let json_requested = argv_requests_flag("--json");
    let cli = match parse_cli_from(env::args_os()) {
        Ok(cli) => cli,
        Err(err) => {
            if json_requested
                && !matches!(
                    err.kind(),
                    clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
                )
            {
                output::print_json_error(&err.to_string());
                std::process::exit(1);
            }

            err.print().expect("failed to print clap error");
            std::process::exit(err.exit_code());
        }
    };

    // Apply color settings before any output is produced.
    ui::color::color_enabled(cli.no_color);

    let json = cli.command.json_flag();

    if let Err(err) = run(cli).await {
        if json {
            output::print_json_error(&format!("{err:#}"));
        } else if !commands::verify::is_rendered_verify_failure(&err) {
            // Reproduce the exact format that `main() -> anyhow::Result<()>` uses:
            // "Error: <debug repr>" which includes "Caused by:" chains.
            eprintln!("Error: {err:?}");
        }
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let Cli { command, non_interactive, .. } = cli;

    match command {
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
        Commands::Show { deployment, namespace, network, no_fork, json } => {
            commands::show::run(deployment, namespace, network, no_fork, json, non_interactive)
                .await?
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
                non_interactive,
            )
            .await?
        }
        Commands::Tag { deployment, add, remove, network, namespace, json } => {
            commands::tag::run(deployment, add, remove, network, namespace, json, non_interactive)
                .await?
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
        Commands::Gen { subcommand } => match subcommand {
            GenSubcommand::Deploy(args) => run_gen_deploy_command(args).await?,
        },
        Commands::GenDeployCompat(args) => run_gen_deploy_command(args).await?,
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
        Commands::Prune(args) => commands::prune::run(args, non_interactive).await?,
        Commands::Reset(args) => commands::reset::run(args, non_interactive).await?,
        Commands::Migrate { subcommand } => {
            commands::migrate::run(subcommand, non_interactive).await?
        }
        Commands::Fork { subcommand } => commands::fork::run(subcommand).await?,
        Commands::Dev { subcommand } => commands::dev::run(subcommand).await?,
        Commands::Completion { shell } | Commands::CompletionCompat { shell } => {
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

async fn run_gen_deploy_command(args: GenDeployArgs) -> anyhow::Result<()> {
    commands::gen_deploy::run(
        &args.artifact,
        args.strategy.as_deref(),
        args.proxy.as_deref(),
        args.proxy_contract.as_deref(),
        args.output.as_deref(),
        args.json,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::migrate::MigrateSubcommand;

    #[test]
    fn gen_deploy_nested_command_sets_json_flag() {
        let cli = Cli::try_parse_from(["treb", "gen", "deploy", "Counter", "--json"]).unwrap();
        assert!(cli.command.json_flag());

        match cli.command {
            Commands::Gen { subcommand: GenSubcommand::Deploy(args) } => {
                assert_eq!(args.artifact, "Counter");
                assert!(args.json);
            }
            _ => panic!("expected nested gen deploy command"),
        }
    }

    #[test]
    fn gen_command_aliases_parse() {
        let cli = Cli::try_parse_from(["treb", "generate", "deploy", "Counter"]).unwrap();
        match cli.command {
            Commands::Gen { subcommand: GenSubcommand::Deploy(args) } => {
                assert_eq!(args.artifact, "Counter");
            }
            _ => panic!("expected generate alias to resolve to gen deploy"),
        }

        let compat = Cli::try_parse_from(["treb", "gen-deploy", "Counter", "--json"]).unwrap();
        match compat.command {
            Commands::GenDeployCompat(args) => {
                assert_eq!(args.artifact, "Counter");
                assert!(args.json);
            }
            _ => panic!("expected hidden gen-deploy compatibility command"),
        }
    }

    #[test]
    fn grouped_help_lists_gen_not_gen_deploy() {
        let help = build_grouped_help(&Cli::command());
        assert!(help.contains("  gen"));
        assert!(help.contains("(alias: generate)"));
        assert!(!help.contains("gen-deploy"));
    }

    #[test]
    fn completion_command_aliases_parse() {
        let cli = Cli::try_parse_from(["treb", "completion", "bash"]).unwrap();
        match cli.command {
            Commands::Completion { shell } => assert_eq!(shell, "bash"),
            _ => panic!("expected completion command"),
        }

        let compat = Cli::try_parse_from(["treb", "completions", "zsh"]).unwrap();
        match compat.command {
            Commands::CompletionCompat { shell } => assert_eq!(shell, "zsh"),
            _ => panic!("expected hidden completions compatibility command"),
        }
    }

    #[test]
    fn grouped_help_lists_completion_not_completions() {
        let help = build_grouped_help(&Cli::command());
        assert!(help.contains("  completion"));
        assert!(!help.contains("completions"));
    }

    #[test]
    fn config_command_defaults_to_show() {
        let cli = parse_cli_from(["treb", "config"]).unwrap();
        match cli.command {
            Commands::Config { subcommand: ConfigSubcommand::Show { json } } => assert!(!json),
            _ => panic!("expected bare config to normalize to config show"),
        }
    }

    #[test]
    fn config_command_defaults_to_show_with_json() {
        let cli = parse_cli_from(["treb", "config", "--json"]).unwrap();
        assert!(cli.command.json_flag());

        match cli.command {
            Commands::Config { subcommand: ConfigSubcommand::Show { json } } => assert!(json),
            _ => panic!("expected bare config --json to normalize to config show --json"),
        }
    }

    #[test]
    fn config_command_defaults_to_show_after_root_flag() {
        let cli = parse_cli_from(["treb", "--no-color", "config"]).unwrap();
        assert!(cli.no_color);

        match cli.command {
            Commands::Config { subcommand: ConfigSubcommand::Show { json } } => assert!(!json),
            _ => panic!("expected --no-color config to normalize to config show"),
        }
    }

    #[test]
    fn config_command_defaults_to_show_with_json_after_root_flag() {
        let cli = parse_cli_from(["treb", "--no-color", "config", "--json"]).unwrap();
        assert!(cli.no_color);
        assert!(cli.command.json_flag());

        match cli.command {
            Commands::Config { subcommand: ConfigSubcommand::Show { json } } => assert!(json),
            _ => panic!("expected --no-color config --json to normalize to config show --json"),
        }
    }

    #[test]
    fn config_help_does_not_normalize_to_show() {
        match parse_cli_from(["treb", "config", "--help"]) {
            Ok(_) => panic!("expected config --help to return clap help output"),
            Err(err) => assert_eq!(err.kind(), clap::error::ErrorKind::DisplayHelp),
        }
    }

    #[test]
    fn non_interactive_is_accepted_before_run_subcommand() {
        let cli =
            parse_cli_from(["treb", "--non-interactive", "run", "script/Deploy.s.sol"]).unwrap();
        assert!(cli.non_interactive);

        match cli.command {
            Commands::Run { script, .. } => assert_eq!(script, "script/Deploy.s.sol"),
            _ => panic!("expected run command"),
        }
    }

    #[test]
    fn non_interactive_is_accepted_after_run_subcommand() {
        let cli =
            parse_cli_from(["treb", "run", "--non-interactive", "script/Deploy.s.sol"]).unwrap();
        assert!(cli.non_interactive);

        match cli.command {
            Commands::Run { script, .. } => assert_eq!(script, "script/Deploy.s.sol"),
            _ => panic!("expected run command"),
        }
    }

    #[test]
    fn non_interactive_is_accepted_on_list() {
        let cli = parse_cli_from(["treb", "--non-interactive", "list"]).unwrap();
        assert!(cli.non_interactive);

        match cli.command {
            Commands::List { .. } => {}
            _ => panic!("expected list command"),
        }
    }

    #[test]
    fn non_interactive_is_accepted_before_migrate_subcommand() {
        let cli = parse_cli_from(["treb", "--non-interactive", "migrate", "config"]).unwrap();
        assert!(cli.non_interactive);

        match cli.command {
            Commands::Migrate { subcommand: MigrateSubcommand::Config { .. } } => {}
            _ => panic!("expected migrate config command"),
        }
    }

    #[test]
    fn list_network_short_flag_parses() {
        let cli = parse_cli_from(["treb", "list", "-n", "mainnet"]).unwrap();

        match cli.command {
            Commands::List { network, .. } => assert_eq!(network.as_deref(), Some("mainnet")),
            _ => panic!("expected list command"),
        }
    }

    #[test]
    fn list_namespace_short_flag_parses() {
        let cli = parse_cli_from(["treb", "list", "-s", "production"]).unwrap();

        match cli.command {
            Commands::List { namespace, .. } => {
                assert_eq!(namespace.as_deref(), Some("production"))
            }
            _ => panic!("expected list command"),
        }
    }

    #[test]
    fn list_alias_accepts_short_flags() {
        let cli = parse_cli_from(["treb", "ls", "-n", "mainnet", "-s", "production"]).unwrap();

        match cli.command {
            Commands::List { network, namespace, .. } => {
                assert_eq!(network.as_deref(), Some("mainnet"));
                assert_eq!(namespace.as_deref(), Some("production"));
            }
            _ => panic!("expected list command"),
        }
    }

    #[test]
    fn tag_short_flags_parse_without_wiring_behavior() {
        let cli = parse_cli_from(["treb", "tag", "Counter", "-n", "mainnet", "-s", "production"])
            .unwrap();

        match cli.command {
            Commands::Tag { deployment, network, namespace, .. } => {
                assert_eq!(deployment.as_deref(), Some("Counter"));
                assert_eq!(network.as_deref(), Some("mainnet"));
                assert_eq!(namespace.as_deref(), Some("production"));
            }
            _ => panic!("expected tag command"),
        }
    }

    #[test]
    fn show_long_query_flags_parse_without_runtime_wiring() {
        let cli = parse_cli_from([
            "treb",
            "show",
            "--namespace",
            "mainnet",
            "--network",
            "42220",
            "--no-fork",
            "Counter",
        ])
        .unwrap();

        match cli.command {
            Commands::Show { deployment, namespace, network, no_fork, .. } => {
                assert_eq!(deployment.as_deref(), Some("Counter"));
                assert_eq!(namespace.as_deref(), Some("mainnet"));
                assert_eq!(network.as_deref(), Some("42220"));
                assert!(no_fork);
            }
            _ => panic!("expected show command"),
        }
    }

    #[test]
    fn show_short_query_flags_are_not_accepted() {
        for args in [
            ["treb", "show", "-n", "mainnet", "Counter"],
            ["treb", "show", "-s", "prod", "Counter"],
        ] {
            match parse_cli_from(args) {
                Ok(_) => panic!("expected show short flags to be rejected"),
                Err(err) => assert_eq!(err.kind(), clap::error::ErrorKind::UnknownArgument),
            }
        }
    }

    #[test]
    fn list_help_includes_short_flags_for_network_and_namespace() {
        let mut cmd = Cli::command();
        let mut buffer = Vec::new();
        cmd.find_subcommand_mut("list").unwrap().write_long_help(&mut buffer).unwrap();
        let help = String::from_utf8(buffer).unwrap();

        assert!(help.contains("-n, --network"), "unexpected help output: {help}");
        assert!(help.contains("-s, --namespace"), "unexpected help output: {help}");
    }

    #[test]
    fn verify_etherscan_short_flag_parses() {
        let cli = parse_cli_from(["treb", "verify", "Counter", "-e"]).unwrap();

        match cli.command {
            Commands::Verify { deployment, etherscan, blockscout, sourcify, .. } => {
                assert_eq!(deployment.as_deref(), Some("Counter"));
                assert!(etherscan);
                assert!(!blockscout);
                assert!(!sourcify);
            }
            _ => panic!("expected verify command"),
        }
    }

    #[test]
    fn verify_blockscout_short_flag_parses() {
        let cli = parse_cli_from(["treb", "verify", "Counter", "-b"]).unwrap();

        match cli.command {
            Commands::Verify { deployment, etherscan, blockscout, sourcify, .. } => {
                assert_eq!(deployment.as_deref(), Some("Counter"));
                assert!(!etherscan);
                assert!(blockscout);
                assert!(!sourcify);
            }
            _ => panic!("expected verify command"),
        }
    }

    #[test]
    fn verify_sourcify_short_flag_parses() {
        let cli = parse_cli_from(["treb", "verify", "Counter", "-s"]).unwrap();

        match cli.command {
            Commands::Verify { deployment, etherscan, blockscout, sourcify, .. } => {
                assert_eq!(deployment.as_deref(), Some("Counter"));
                assert!(!etherscan);
                assert!(!blockscout);
                assert!(sourcify);
            }
            _ => panic!("expected verify command"),
        }
    }

    #[test]
    fn verify_combined_short_flags_parse() {
        let cli = parse_cli_from(["treb", "verify", "Counter", "-ebs"]).unwrap();

        match cli.command {
            Commands::Verify { deployment, etherscan, blockscout, sourcify, .. } => {
                assert_eq!(deployment.as_deref(), Some("Counter"));
                assert!(etherscan);
                assert!(blockscout);
                assert!(sourcify);
            }
            _ => panic!("expected verify command"),
        }
    }

    #[test]
    fn verify_blockscout_verifier_url_alias_parses() {
        let cli = parse_cli_from([
            "treb",
            "verify",
            "Counter",
            "--blockscout-verifier-url",
            "https://example.com/api",
        ])
        .unwrap();

        match cli.command {
            Commands::Verify { deployment, verifier_url, .. } => {
                assert_eq!(deployment.as_deref(), Some("Counter"));
                assert_eq!(verifier_url.as_deref(), Some("https://example.com/api"));
            }
            _ => panic!("expected verify command"),
        }
    }

    #[test]
    fn subcommand_help_uses_treb_bin_name() {
        for args in [
            ["treb-cli", "config", "--help"],
            ["treb-cli", "gen", "--help"],
            ["treb-cli", "completion", "--help"],
        ] {
            let err = build_grouped_command().try_get_matches_from(args).unwrap_err();
            assert_eq!(err.kind(), clap::error::ErrorKind::DisplayHelp);

            let output = err.to_string();
            assert!(output.contains("Usage: treb "), "unexpected help output: {output}");
            assert!(!output.contains("Usage: treb-cli "), "unexpected help output: {output}");
        }
    }
}
