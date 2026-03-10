use std::env;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
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
    version,
    about,
    long_about = "Trebuchet (treb) orchestrates Foundry script execution for deterministic smart contract deployments using CreateX factory contracts."
)]
struct Cli {
    /// Disable colored output (also respected via NO_COLOR env var)
    #[arg(long, global = true)]
    no_color: bool,

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
        /// Skip interactive prompts (also enabled via TREB_NON_INTERACTIVE=1, CI=true, or non-TTY
        /// stdin/stdout)
        #[arg(long)]
        non_interactive: bool,
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
        #[arg(long)]
        network: Option<String>,
        /// Deployment namespace
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
    ///
    /// Generate deployment scripts for contracts and libraries.
    ///
    /// This command creates template scripts using treb-sol's base contracts.
    /// The generated scripts handle both direct deployments and common proxy patterns.
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
        /// Skip interactive prompts (also enabled via TREB_NON_INTERACTIVE=1, CI=true, or non-TTY
        /// stdin/stdout)
        #[arg(long)]
        non_interactive: bool,
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
    ///   treb completions bash >> ~/.bashrc
    ///   treb completions zsh > ~/.zsh/completions/_treb
    ///   treb completions fish > ~/.config/fish/completions/treb.fish
    #[command(verbatim_doc_comment)]
    Completions {
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
            Commands::Fork { subcommand } => fork_subcommand_json_flag(subcommand),
            Commands::Dev { subcommand } => dev_subcommand_json_flag(subcommand),
            Commands::Migrate { subcommand } => migrate_subcommand_json_flag(subcommand),
            Commands::Init { .. } | Commands::Completions { .. } => false,
        }
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
        commands::migrate::MigrateSubcommand::Registry { .. } => false,
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
    let mut cmd = Cli::command();

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
                let about = sub.get_about().map(|a| a.to_string()).unwrap_or_default();
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
        &["init", "list", "show", "gen-deploy", "run", "verify", "compose", "fork"],
    );
    s.push('\n');
    write_group(
        &mut s,
        cmd,
        "Management Commands:",
        &["sync", "tag", "register", "dev", "networks", "prune", "reset", "config", "migrate"],
    );
    s.push('\n');
    write_group(&mut s, cmd, "Additional Commands:", &["version", "completions", "help"]);

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

#[tokio::main]
async fn main() {
    let no_color_requested = argv_requests_flag("--no-color");
    ui::color::color_enabled(no_color_requested);

    let json_requested = argv_requests_flag("--json");
    let cmd = build_grouped_command();
    let cli = match cmd.try_get_matches() {
        Ok(matches) => Cli::from_arg_matches(&matches).expect("bug: derive/builder arg mismatch"),
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
