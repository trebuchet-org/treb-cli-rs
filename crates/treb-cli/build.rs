use std::{path::Path, process::Command};

use clap::{Arg, ArgAction, Command as ClapCommand};
use clap_complete::{Shell, generate_to};

fn build_cli() -> ClapCommand {
    ClapCommand::new("treb")
        .about("treb — deployment orchestration for Foundry projects")
        .arg(
            Arg::new("no-color")
                .long("no-color")
                .action(ArgAction::SetTrue)
                .global(true)
                .help("Disable colored output"),
        )
        .arg(
            Arg::new("non-interactive")
                .long("non-interactive")
                .action(ArgAction::SetTrue)
                .global(true)
                .help("Skip interactive confirmation prompts"),
        )
        .subcommand(build_run())
        .subcommand(build_list())
        .subcommand(build_show())
        .subcommand(build_init())
        .subcommand(build_config())
        .subcommand(build_verify())
        .subcommand(build_registry())
        .subcommand(build_addressbook())
        .subcommand(build_version())
        .subcommand(build_networks())
        .subcommand(build_compose())
        .subcommand(build_queued())
        .subcommand(build_fork())
        .subcommand(build_dev())
        .subcommand(build_completion_cmd())
        .subcommand(build_completions_compat())
}

fn build_run() -> ClapCommand {
    ClapCommand::new("run")
        .about("Execute a deployment script")
        .arg(Arg::new("script").help("Path to the Forge script"))
        .arg(Arg::new("sig").long("sig").default_value("run()").help("Function signature to call"))
        .arg(Arg::new("args").long("args").num_args(1..).help("Arguments to pass to the script"))
        .arg(Arg::new("network").long("network").help("Network name or chain ID"))
        .arg(Arg::new("rpc-url").long("rpc-url").help("Explicit RPC URL (overrides network)"))
        .arg(Arg::new("namespace").long("namespace").help("Deployment namespace"))
        .arg(
            Arg::new("broadcast")
                .long("broadcast")
                .action(ArgAction::SetTrue)
                .help("Broadcast transactions to the network"),
        )
        .arg(
            Arg::new("slow")
                .long("slow")
                .action(ArgAction::SetTrue)
                .help("Send transactions one at a time"),
        )
        .arg(
            Arg::new("legacy")
                .long("legacy")
                .action(ArgAction::SetTrue)
                .help("Use legacy (pre-EIP-1559) transactions"),
        )
        .arg(
            Arg::new("verify")
                .long("verify")
                .action(ArgAction::SetTrue)
                .help("Verify deployed contracts on Etherscan"),
        )
        .arg(
            Arg::new("verbose")
                .long("verbose")
                .short('v')
                .action(ArgAction::Count)
                .help("Verbosity level (-v, -vv, -vvv)"),
        )
        .arg(
            Arg::new("resume")
                .long("resume")
                .action(ArgAction::SetTrue)
                .help("Resume a previous run, skipping already-completed transactions"),
        )
        .arg(Arg::new("json").long("json").action(ArgAction::SetTrue).help("Output as JSON"))
        .arg(Arg::new("env").long("env").num_args(1).help("Set environment variables (KEY=VALUE)"))
        .arg(Arg::new("target-contract").long("target-contract").help("Target contract to run"))
}

fn build_list() -> ClapCommand {
    ClapCommand::new("list")
        .about("List deployments in the registry")
        .alias("ls")
        .arg(
            Arg::new("network")
                .long("network")
                .short('n')
                .help("Filter by network name or chain ID"),
        )
        .arg(Arg::new("namespace").long("namespace").short('s').help("Filter by namespace"))
        .arg(
            Arg::new("type")
                .long("type")
                .help("Filter by deployment type (SINGLETON, PROXY, LIBRARY)"),
        )
        .arg(Arg::new("tag").long("tag").help("Filter by tag"))
        .arg(Arg::new("contract").long("contract").help("Filter by contract name"))
        .arg(Arg::new("label").long("label").help("Filter by deployment label"))
        .arg(
            Arg::new("fork")
                .long("fork")
                .action(ArgAction::SetTrue)
                .help("Show only fork deployments"),
        )
        .arg(
            Arg::new("no-fork")
                .long("no-fork")
                .action(ArgAction::SetTrue)
                .help("Hide fork deployments"),
        )
        .arg(Arg::new("json").long("json").action(ArgAction::SetTrue).help("Output as JSON"))
}

fn build_show() -> ClapCommand {
    ClapCommand::new("show")
        .about("Show detailed information about a specific deployment")
        .arg(Arg::new("namespace").long("namespace").help("Deployment namespace"))
        .arg(Arg::new("network").long("network").help("Network name or chain ID"))
        .arg(
            Arg::new("no-fork")
                .long("no-fork")
                .action(ArgAction::SetTrue)
                .help("Hide fork deployments"),
        )
        .arg(Arg::new("deployment").help("Deployment identifier; omit to select interactively"))
        .arg(Arg::new("json").long("json").action(ArgAction::SetTrue).help("Output as JSON"))
}

fn build_init() -> ClapCommand {
    ClapCommand::new("init").about("Initialize a treb project").arg(
        Arg::new("force")
            .long("force")
            .action(ArgAction::SetTrue)
            .help("Overwrite local config even if already initialized"),
    )
}

fn build_config() -> ClapCommand {
    ClapCommand::new("config")
        .about("Manage treb configuration")
        .subcommand(
            ClapCommand::new("show").about("Display the resolved configuration").arg(
                Arg::new("json").long("json").action(ArgAction::SetTrue).help("Output as JSON"),
            ),
        )
        .subcommand(
            ClapCommand::new("set")
                .about("Set a local configuration value")
                .arg(Arg::new("key").help("Configuration key"))
                .arg(Arg::new("value").help("Value to set")),
        )
        .subcommand(
            ClapCommand::new("remove")
                .about("Remove (reset) a local configuration value")
                .arg(Arg::new("key").help("Configuration key")),
        )
}

fn build_verify() -> ClapCommand {
    ClapCommand::new("verify")
        .about("Verify deployed contracts on block explorers")
        .arg(Arg::new("deployment").help("Deployment identifier; omit to select interactively"))
        .arg(Arg::new("namespace").long("namespace").help("Deployment namespace"))
        .arg(Arg::new("network").long("network").short('n').help("Network name or chain ID"))
        .arg(
            Arg::new("all")
                .long("all")
                .action(ArgAction::SetTrue)
                .help("Verify all unverified deployments"),
        )
        .arg(
            Arg::new("verifier")
                .long("verifier")
                .default_value("etherscan")
                .help("Verification provider"),
        )
        .arg(
            Arg::new("etherscan")
                .long("etherscan")
                .short('e')
                .action(ArgAction::SetTrue)
                .help("Verify on Etherscan"),
        )
        .arg(
            Arg::new("blockscout")
                .long("blockscout")
                .short('b')
                .action(ArgAction::SetTrue)
                .help("Verify on Blockscout"),
        )
        .arg(
            Arg::new("sourcify")
                .long("sourcify")
                .short('s')
                .action(ArgAction::SetTrue)
                .help("Verify on Sourcify"),
        )
        .arg(
            Arg::new("verifier-url")
                .long("verifier-url")
                .visible_alias("blockscout-verifier-url")
                .help("Verifier API URL override"),
        )
        .arg(
            Arg::new("contract-path")
                .long("contract-path")
                .help("Contract path override (e.g. ./src/Counter.sol:Counter)"),
        )
        .arg(
            Arg::new("debug")
                .long("debug")
                .action(ArgAction::SetTrue)
                .help("Print the forge verify command before execution"),
        )
        .arg(Arg::new("verifier-api-key").long("verifier-api-key").help("Verifier API key"))
        .arg(
            Arg::new("force")
                .long("force")
                .action(ArgAction::SetTrue)
                .help("Re-verify already verified contracts"),
        )
        .arg(
            Arg::new("watch")
                .long("watch")
                .action(ArgAction::SetTrue)
                .help("Watch verification status until confirmed"),
        )
        .arg(
            Arg::new("retries").long("retries").default_value("5").help("Number of retry attempts"),
        )
        .arg(
            Arg::new("delay")
                .long("delay")
                .default_value("5")
                .help("Delay in seconds between retries"),
        )
        .arg(Arg::new("json").long("json").action(ArgAction::SetTrue).help("Output as JSON"))
}

fn build_registry() -> ClapCommand {
    ClapCommand::new("registry")
        .about("Manage the deployment registry")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(build_registry_sync())
        .subcommand(build_registry_prune())
        .subcommand(build_registry_tag())
        .subcommand(build_registry_add())
        .subcommand(build_registry_drop())
}

fn build_registry_sync() -> ClapCommand {
    ClapCommand::new("sync")
        .about("Sync safe transaction state from the Safe Transaction Service")
        .arg(Arg::new("network").long("network").help("Filter sync to a specific network"))
        .arg(
            Arg::new("clean")
                .long("clean")
                .action(ArgAction::SetTrue)
                .help("Remove invalid entries while syncing"),
        )
        .arg(
            Arg::new("debug")
                .long("debug")
                .action(ArgAction::SetTrue)
                .help("Print raw API responses to stderr"),
        )
        .arg(Arg::new("json").long("json").action(ArgAction::SetTrue).help("Output as JSON"))
}

fn build_registry_prune() -> ClapCommand {
    ClapCommand::new("prune")
        .about("Remove stale or broken registry entries")
        .arg(
            Arg::new("dry-run")
                .long("dry-run")
                .action(ArgAction::SetTrue)
                .help("Report prune candidates without deleting anything"),
        )
        .arg(
            Arg::new("include-pending")
                .long("include-pending")
                .action(ArgAction::SetTrue)
                .help("Include pending transactions in the prune scan"),
        )
        .arg(
            Arg::new("network")
                .long("network")
                .help("Filter candidates to a specific network (by chain ID)"),
        )
        .arg(
            Arg::new("yes")
                .long("yes")
                .short('y')
                .action(ArgAction::SetTrue)
                .help("Skip confirmation prompt"),
        )
        .arg(Arg::new("json").long("json").action(ArgAction::SetTrue).help("Output as JSON"))
}

fn build_registry_tag() -> ClapCommand {
    ClapCommand::new("tag")
        .about("Manage tags on a deployment")
        .arg(Arg::new("deployment").help("Deployment identifier; omit to select interactively"))
        .arg(Arg::new("add").long("add").help("Add a tag to the deployment"))
        .arg(Arg::new("remove").long("remove").help("Remove a tag from the deployment"))
        .arg(Arg::new("network").long("network").short('n').help("Network name or chain ID"))
        .arg(Arg::new("namespace").long("namespace").short('s').help("Deployment namespace"))
        .arg(Arg::new("json").long("json").action(ArgAction::SetTrue).help("Output as JSON"))
}

fn build_registry_drop() -> ClapCommand {
    ClapCommand::new("drop")
        .about("Drop registry entries by query, network, or namespace")
        .arg(Arg::new("query").help("Deployment query (contract name, label, or ID)"))
        .arg(
            Arg::new("network")
                .long("network")
                .short('n')
                .help("Network name or chain ID"),
        )
        .arg(Arg::new("namespace").long("namespace").short('s').help("Deployment namespace"))
        .arg(
            Arg::new("yes")
                .long("yes")
                .short('y')
                .action(ArgAction::SetTrue)
                .help("Skip confirmation prompt"),
        )
        .arg(Arg::new("json").long("json").action(ArgAction::SetTrue).help("Output as JSON"))
}

fn build_addressbook() -> ClapCommand {
    ClapCommand::new("addressbook")
        .about("Manage named addresses scoped by chain ID")
        .alias("ab")
        .arg(Arg::new("network").long("network").short('n').help("Network name or chain ID"))
        .arg(Arg::new("namespace").long("namespace").short('s').help("Deployment namespace"))
        .subcommand(
            ClapCommand::new("set")
                .about("Set an addressbook entry for the current chain")
                .arg(Arg::new("name").required(true).help("Entry name"))
                .arg(Arg::new("address").required(true).help("Contract or account address")),
        )
        .subcommand(
            ClapCommand::new("remove")
                .about("Remove an addressbook entry for the current chain")
                .arg(Arg::new("name").required(true).help("Entry name")),
        )
        .subcommand(
            ClapCommand::new("list")
                .about("List addressbook entries for the current chain")
                .alias("ls")
                .arg(
                    Arg::new("json").long("json").action(ArgAction::SetTrue).help("Output as JSON"),
                ),
        )
}

fn build_registry_add() -> ClapCommand {
    ClapCommand::new("add")
        .about("Register deployments from a historical transaction")
        .arg(
            Arg::new("tx-hash")
                .long("tx-hash")
                .required(true)
                .help("Transaction hash to trace for contract creations"),
        )
        .arg(Arg::new("network").long("network").help("Network name or chain ID"))
        .arg(Arg::new("rpc-url").long("rpc-url").help("Explicit RPC URL (overrides network)"))
        .arg(Arg::new("address").long("address").help("Filter to a specific deployed address"))
        .arg(Arg::new("contract").long("contract").help("Contract artifact name to match"))
        .arg(
            Arg::new("contract-name")
                .long("contract-name")
                .help("Contract name to narrow matching"),
        )
        .arg(Arg::new("label").long("label").help("Label for registered deployments"))
        .arg(Arg::new("namespace").long("namespace").help("Deployment namespace"))
        .arg(
            Arg::new("skip-verify")
                .long("skip-verify")
                .action(ArgAction::SetTrue)
                .help("Skip post-registration verification"),
        )
        .arg(Arg::new("json").long("json").action(ArgAction::SetTrue).help("Output as JSON"))
}


fn build_version() -> ClapCommand {
    ClapCommand::new("version")
        .about("Print version information")
        .arg(Arg::new("json").long("json").action(ArgAction::SetTrue).help("Output as JSON"))
}

fn build_networks() -> ClapCommand {
    ClapCommand::new("networks")
        .about("List available networks")
        .arg(Arg::new("json").long("json").action(ArgAction::SetTrue).help("Output as JSON"))
}

fn build_compose() -> ClapCommand {
    ClapCommand::new("compose")
        .about("Compose multi-step deployment pipelines")
        .arg(Arg::new("file").help("Path to the compose YAML file"))
        .arg(Arg::new("network").long("network").help("Network name or chain ID"))
        .arg(Arg::new("rpc-url").long("rpc-url").help("Explicit RPC URL (overrides network)"))
        .arg(Arg::new("namespace").long("namespace").help("Deployment namespace"))
        .arg(Arg::new("profile").long("profile").help("Foundry profile override"))
        .arg(
            Arg::new("broadcast")
                .long("broadcast")
                .action(ArgAction::SetTrue)
                .help("Broadcast transactions to the network"),
        )
        .arg(
            Arg::new("resume")
                .long("resume")
                .action(ArgAction::SetTrue)
                .help("Skip already-completed components"),
        )
        .arg(
            Arg::new("verify")
                .long("verify")
                .action(ArgAction::SetTrue)
                .help("Verify contracts after deployment"),
        )
        .arg(
            Arg::new("slow")
                .long("slow")
                .action(ArgAction::SetTrue)
                .help("Send transactions one at a time"),
        )
        .arg(
            Arg::new("legacy")
                .long("legacy")
                .action(ArgAction::SetTrue)
                .help("Use legacy (pre-EIP-1559) transactions"),
        )
        .arg(
            Arg::new("verbose")
                .long("verbose")
                .short('v')
                .action(ArgAction::Count)
                .help("Verbosity level (-v, -vv, -vvv)"),
        )
        .arg(Arg::new("json").long("json").action(ArgAction::SetTrue).help("Output as JSON"))
        .arg(Arg::new("env").long("env").num_args(1).help("Set environment variables (KEY=VALUE)"))
}


fn build_queued() -> ClapCommand {
    ClapCommand::new("queued")
        .about("List queued Safe/Governor operations")
        .arg(
            Arg::new("network")
                .long("network")
                .help("Network name or chain ID"),
        )
        .arg(
            Arg::new("json")
                .long("json")
                .action(ArgAction::SetTrue)
                .help("Output as JSON"),
        )
}

fn build_fork() -> ClapCommand {
    ClapCommand::new("fork")
        .about("Fork networks for local testing")
        .subcommand(
            ClapCommand::new("enter")
                .about("Enter fork mode: snapshot registry, start Anvil for all networks")
                .arg(
                    Arg::new("network")
                        .long("network")
                        .value_delimiter(',')
                        .help("Fork specific networks (comma-separated)"),
                )
                .arg(
                    Arg::new("rpc-url")
                        .long("rpc-url")
                        .help("Upstream RPC URL (only with single --network)"),
                )
                .arg(
                    Arg::new("fork-block-number")
                        .long("fork-block-number")
                        .help("Fork at a specific block number"),
                ),
        )
        .subcommand(
            ClapCommand::new("exit")
                .about("Exit fork mode: stop all Anvils, restore registry"),
        )
        .subcommand(
            ClapCommand::new("revert")
                .about("Revert all forks to their initial state"),
        )
        .subcommand(
            ClapCommand::new("restart")
                .about("Restart a fork from a new block")
                .arg(
                    Arg::new("network")
                        .long("network")
                        .help("Network name to restart"),
                )
                .arg(
                    Arg::new("fork-block-number")
                        .long("fork-block-number")
                        .help("Fork block number to reset to"),
                ),
        )
        .subcommand(
            ClapCommand::new("status").about("Show active fork status").arg(
                Arg::new("json").long("json").action(ArgAction::SetTrue).help("Output as JSON"),
            ),
        )
        .subcommand(
            ClapCommand::new("history")
                .about("Show fork history")
                .arg(
                    Arg::new("network")
                        .long("network")
                        .help("Filter by network name"),
                )
                .arg(
                    Arg::new("json").long("json").action(ArgAction::SetTrue).help("Output as JSON"),
                ),
        )
        .subcommand(
            ClapCommand::new("logs")
                .about("Tail Anvil log files for active forks")
                .arg(
                    Arg::new("follow")
                        .long("follow")
                        .short('f')
                        .action(ArgAction::SetTrue)
                        .help("Continuously tail log files"),
                )
                .arg(
                    Arg::new("network")
                        .long("network")
                        .help("Filter to a specific network"),
                ),
        )
        .subcommand(
            ClapCommand::new("exec")
                .about("Execute queued Safe/Governor items on the active fork")
                .arg(Arg::new("query").help("Safe tx hash or proposal ID to execute"))
                .arg(
                    Arg::new("all")
                        .long("all")
                        .action(ArgAction::SetTrue)
                        .help("Execute all queued items"),
                )
                .arg(
                    Arg::new("network")
                        .long("network")
                        .help("Network name or chain ID"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .action(ArgAction::SetTrue)
                        .help("Output as JSON"),
                ),
        )
}

fn build_dev() -> ClapCommand {
    ClapCommand::new("dev").about("Start a local development environment").subcommand(
        ClapCommand::new("anvil")
            .about("Manage local Anvil development nodes")
            .subcommand(
                ClapCommand::new("start")
                    .about("Start a local Anvil node in the foreground")
                    .arg(Arg::new("network").long("network").help("Network name"))
                    .arg(Arg::new("port").long("port").help("Port to listen on (default: 8545)"))
                    .arg(
                        Arg::new("fork-block-number")
                            .long("fork-block-number")
                            .help("Block number to fork from"),
                    ),
            )
            .subcommand(
                ClapCommand::new("stop")
                    .about("Clean up stale Anvil entries in fork state")
                    .arg(Arg::new("network").long("network").help("Network name to stop")),
            )
            .subcommand(
                ClapCommand::new("restart")
                    .about("Restart an Anvil node")
                    .arg(Arg::new("network").long("network").help("Network name to restart")),
            )
            .subcommand(ClapCommand::new("status").about("Show Anvil node status").arg(
                Arg::new("json").long("json").action(ArgAction::SetTrue).help("Output as JSON"),
            )),
    )
}

fn build_completion_cmd() -> ClapCommand {
    ClapCommand::new("completion").about("Generate shell completion scripts").arg(
        Arg::new("shell").required(true).help("Shell type (bash, zsh, fish, elvish, powershell)"),
    )
}

fn build_completions_compat() -> ClapCommand {
    ClapCommand::new("completions").hide(true).about("Generate shell completion scripts").arg(
        Arg::new("shell").required(true).help("Shell type (bash, zsh, fish, elvish, powershell)"),
    )
}

fn assert_bash_completion_contains_legacy_subcommand(path: &Path) {
    let script = std::fs::read_to_string(path).unwrap_or_else(|err| {
        panic!("failed to read generated bash completion script {}: {err}", path.display())
    });
    assert!(
        script.contains("completions"),
        "generated bash completion script {} is missing the legacy 'completions' subcommand",
        path.display()
    );
}

fn workspace_foundry_version() -> Option<String> {
    let manifest = std::fs::read_to_string("../../Cargo.toml").ok()?;
    let manifest: toml::Value = toml::from_str(&manifest).ok()?;

    manifest
        .get("workspace")?
        .get("dependencies")?
        .get("foundry-config")?
        .get("tag")?
        .as_str()
        .map(str::to_owned)
}

fn treb_version() -> String {
    let fallback = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "unknown".to_string());

    Command::new("git")
        .args(["describe", "--tags", "--always", "--dirty"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback)
}

fn main() {
    // Re-run if git state changes.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs");
    println!("cargo:rerun-if-changed=../../.git/packed-refs");
    println!("cargo:rerun-if-changed=src/main.rs");
    // Re-run when treb-sol submodule pointer changes.
    println!("cargo:rerun-if-changed=../../lib/treb-sol");
    // Re-run when workspace Cargo.toml changes (foundry version pinning).
    println!("cargo:rerun-if-changed=../../Cargo.toml");

    // Git commit hash.
    let git_commit = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=TREB_GIT_COMMIT={git_commit}");

    let treb_version = treb_version();
    println!("cargo:rustc-env=TREB_VERSION={treb_version}");

    // Build date (UTC, RFC3339) for formatting at render time.
    let build_date = Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=TREB_BUILD_DATE={build_date}");

    // Foundry version: extract the pinned git tag from workspace Cargo.toml.
    let foundry_version = workspace_foundry_version().unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=TREB_FOUNDRY_VERSION={foundry_version}");

    // treb-sol submodule commit hash.
    let treb_sol_commit = Command::new("git")
        .args(["-C", "../../lib/treb-sol", "rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=TREB_SOL_COMMIT={treb_sol_commit}");

    // Rust version.
    let rust_version = Command::new("rustc")
        .args(["--version"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=TREB_RUST_VERSION={rust_version}");

    // Generate shell completion scripts into $OUT_DIR/completions/.
    let outdir = match std::env::var_os("OUT_DIR") {
        Some(d) => d,
        None => return,
    };
    let completions_dir = std::path::Path::new(&outdir).join("completions");
    if std::fs::create_dir_all(&completions_dir).is_ok() {
        for shell in [Shell::Bash, Shell::Zsh, Shell::Fish, Shell::Elvish] {
            let mut cmd = build_cli();
            if let Ok(path) = generate_to(shell, &mut cmd, "treb", &completions_dir) {
                if matches!(shell, Shell::Bash) {
                    assert_bash_completion_contains_legacy_subcommand(&path);
                }
            }
        }
    }
}
