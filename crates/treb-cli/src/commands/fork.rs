//! `treb fork` subcommands — enter/exit fork mode, status, history, diff, revert, restart.

use std::{
    collections::{BTreeSet, HashMap},
    env,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, bail};
use chrono::Utc;
use clap::Subcommand;
use foundry_common::{
    Shell as FoundryShell,
    shell::{ColorChoice, OutputFormat, OutputMode, Verbosity},
};
use owo_colors::{OwoColorize, Style};
use serde::Serialize;
use tokio::net::TcpStream;
use treb_config::{ResolveOpts, load_local_config, resolve_config};
use treb_core::types::fork::{ForkEntry, ForkHistoryEntry};
use treb_forge::{
    createx::createx_deployed_bytecode, execute_script, script::build_script_config_with_senders,
    sender::resolve_all_senders,
};
use treb_registry::{
    DEPLOYMENTS_FILE, ForkStateStore, TRANSACTIONS_FILE, remove_snapshot, restore_registry,
    snapshot_registry,
};

use crate::{output, ui::color};

const TREB_DIR: &str = ".treb";
const SNAPSHOT_BASE: &str = "snapshots";
const CREATEX_ADDRESS: &str = "0xba5Ed099633D3B313e4D5F7bdc1305d3c28ba5Ed";

// ── Subcommand enum ───────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum ForkSubcommand {
    /// Enter fork mode for a network: snapshot registry and record fork state
    ///
    /// Snapshots the current registry and records an active fork entry in
    /// `fork.json`. Run `treb dev anvil start --network <name>` after
    /// this to start a local node pointing at the forked chain.
    Enter {
        /// Network name or chain ID
        #[arg(long)]
        network: String,
        /// Explicit RPC URL (overrides network)
        #[arg(long)]
        rpc_url: Option<String>,
        /// Fork at a specific block number
        #[arg(long)]
        fork_block_number: Option<u64>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Exit fork mode: restore registry from snapshot and remove fork state
    ///
    /// Restores the registry to the state it was in before `fork enter` and
    /// removes the fork entry and its snapshot directory.
    Exit {
        /// Network name or chain ID
        #[arg(long)]
        network: String,
        /// Exit all active forks
        #[arg(long)]
        all: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Revert the fork to its last snapshot
    ///
    /// Restores the registry from the snapshot taken when fork mode was entered,
    /// discarding any deployments made during the fork session.
    Revert {
        /// Network name or chain ID
        #[arg(long)]
        network: String,
        /// Revert all active forks
        #[arg(long)]
        all: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Restart the fork from a new block
    ///
    /// Resets the local Anvil node to a fresh fork at the given block number
    /// (or at the latest block if omitted) without exiting fork mode.
    Restart {
        /// Network name or chain ID
        #[arg(long)]
        network: String,
        /// Fork block number to reset to (uses latest if omitted)
        #[arg(long)]
        fork_block_number: Option<u64>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show active fork status
    ///
    /// Lists all currently active forks with their network name, chain ID,
    /// fork URL, and the Anvil RPC port if a local node is running.
    Status {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show fork history
    ///
    /// Displays the log of fork lifecycle events (enter, exit, revert, restart)
    /// for all networks or a specific one.
    History {
        /// Network name or chain ID
        #[arg(long)]
        network: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Diff current registry vs snapshot
    ///
    /// Shows deployments that were added or removed since fork mode was entered
    /// by comparing the current registry against the saved snapshot.
    Diff {
        /// Network name or chain ID
        #[arg(long)]
        network: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

// ── JSON output structs ──────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ForkExitJson {
    network: String,
    restored_entries: usize,
    cleaned_up: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ForkRevertJson {
    network: String,
    snapshot_id: Option<String>,
    new_snapshot_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ForkRestartJson {
    network: String,
    chain_id: u64,
    port: u16,
    rpc_url: String,
    snapshot_id: Option<String>,
}

#[derive(Default)]
struct RevertCommandSummary {
    snapshot_count: usize,
    commands: BTreeSet<String>,
}

impl RevertCommandSummary {
    fn record(&mut self, command: &str) {
        self.snapshot_count += 1;
        self.commands.insert(command.to_string());
    }

    fn renderable_command(&self) -> Option<&str> {
        if self.snapshot_count == 0 || self.commands.len() != 1 {
            return None;
        }

        let command = self.commands.iter().next()?;
        if command.is_empty() { None } else { Some(command.as_str()) }
    }
}

/// Restores Foundry's global shell after temporarily silencing it.
///
/// Fork setup scripts should not add forge compilation/broadcast chatter to
/// `fork restart` output, so suppress Foundry's shared shell while they run.
struct FoundryShellGuard {
    output_format: OutputFormat,
    output_mode: OutputMode,
    color_choice: ColorChoice,
    verbosity: Verbosity,
}

impl FoundryShellGuard {
    fn suppress() -> Self {
        let mut shell = FoundryShell::get();
        let previous = Self {
            output_format: shell.output_format(),
            output_mode: shell.output_mode(),
            color_choice: shell.color_choice(),
            verbosity: shell.verbosity(),
        };
        *shell = FoundryShell::empty();
        previous
    }
}

impl Drop for FoundryShellGuard {
    fn drop(&mut self) {
        *FoundryShell::get() = FoundryShell::new_with(
            self.output_format,
            self.output_mode,
            self.color_choice,
            self.verbosity,
        );
    }
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

pub async fn run(subcommand: ForkSubcommand) -> anyhow::Result<()> {
    match subcommand {
        ForkSubcommand::Enter { network, rpc_url, fork_block_number, json } => {
            run_enter(network, rpc_url, fork_block_number, json).await
        }
        ForkSubcommand::Exit { network, all, json } => run_exit(network, all, json).await,
        ForkSubcommand::Revert { network, all, json } => run_revert(network, all, json).await,
        ForkSubcommand::Restart { network, fork_block_number, json } => {
            run_restart(network, fork_block_number, json).await
        }
        ForkSubcommand::Status { json } => run_status(json).await,
        ForkSubcommand::History { network, json } => run_history(network, json).await,
        ForkSubcommand::Diff { network, json } => run_diff(network, json).await,
    }
}

// ── run_enter ─────────────────────────────────────────────────────────────────

/// Enter fork mode for a network.
///
/// Validates the project is initialized, resolves the upstream RPC URL,
/// snapshots the registry, and records a [`ForkEntry`] in `fork.json`.
///
/// The `rpc_url` and `port` fields in the stored [`ForkEntry`] are left empty
/// until `treb dev anvil start --network <network>` fills them in.
pub async fn run_enter(
    network: String,
    rpc_url_override: Option<String>,
    fork_block_number: Option<u64>,
    json: bool,
) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let treb_dir = cwd.join(TREB_DIR);

    ensure_treb_dir(&treb_dir)?;

    if json {
        bail!(
            "`fork enter --json` is not supported because port, rpcUrl, snapshotId, and pid are not known until `treb dev anvil start --network {}` runs",
            network
        );
    }

    // Load fork state and check not already forked (before any HTTP calls)
    let mut store = ForkStateStore::new(&treb_dir);
    store.load().context("failed to load fork state")?;

    if store.has_active_fork_network(&network) {
        bail!(
            "network '{}' is already forked; run `treb fork exit --network {}` first",
            network,
            network
        );
    }

    // Resolve upstream RPC URL
    let fork_url = resolve_fork_url(&cwd, &network, rpc_url_override)?;

    // Get chain_id from upstream
    let chain_id = fetch_chain_id(&fork_url)
        .await
        .with_context(|| format!("failed to get chain ID from RPC URL: {fork_url}"))?;

    // Create snapshot dir and snapshot registry
    let snapshot_dir = treb_dir.join(SNAPSHOT_BASE).join(&network);
    snapshot_registry(&treb_dir, &snapshot_dir).context("failed to snapshot registry")?;

    // Build and insert fork entry (rpc_url/port are placeholders until dev anvil start)
    let now = Utc::now();
    let entry = ForkEntry {
        network: network.clone(),
        instance_name: None,
        rpc_url: String::new(),
        port: 0,
        chain_id,
        fork_url: fork_url.clone(),
        fork_block_number,
        snapshot_dir: snapshot_dir.to_string_lossy().into_owned(),
        started_at: now,
        env_var_name: format!("ETH_RPC_URL_{}", network.to_uppercase()),
        original_rpc: fork_url,
        anvil_pid: 0,
        pid_file: String::new(),
        log_file: String::new(),
        entered_at: now,
        snapshots: vec![],
    };
    store.insert_active_fork(entry.clone()).context("failed to record fork entry")?;

    // Add history entry
    store
        .add_history(ForkHistoryEntry {
            action: "enter".into(),
            network: network.clone(),
            timestamp: Utc::now(),
            details: None,
        })
        .context("failed to record fork history")?;

    println!("Entered fork mode for network '{network}'.");
    render_fork_fields(&entry, false);
    println!();
    println!("Run 'treb dev anvil start --network {network}' to start a local Anvil node");
    println!("Run 'treb fork status' to check fork state");
    println!("Run 'treb fork exit' to stop fork and restore original state");

    Ok(())
}

// ── run_exit ──────────────────────────────────────────────────────────────────

/// Exit fork mode for a network.
///
/// Restores the registry from its snapshot, removes the snapshot directory,
/// and removes the [`ForkEntry`] from `fork.json`.
pub async fn run_exit(network: String, all: bool, json: bool) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let treb_dir = cwd.join(TREB_DIR);

    let mut store = ForkStateStore::new(&treb_dir);
    store.load().context("failed to load fork state")?;

    let networks: Vec<String> = if all { exit_ordered_networks(&store) } else { vec![network] };

    if networks.is_empty() {
        if json {
            output::print_json(&Vec::<ForkExitJson>::new())?;
        } else {
            println!("No active forks to exit.");
        }
        return Ok(());
    }

    let mut json_results: Vec<ForkExitJson> = Vec::new();
    let mut exited_networks: Vec<String> = Vec::new();

    for net in &networks {
        let entry = resolve_fork_session_entry(&store, net)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("network '{}' is not in fork mode", net))?;

        // Count restored entries before removing fork state
        let restored_entries = store.list_active_forks_for_network(net).len();

        store.remove_active_forks_for_network(net).context("failed to remove fork entry")?;

        // Restore registry from snapshot
        let snapshot_dir = PathBuf::from(&entry.snapshot_dir);
        restore_registry(&snapshot_dir, &treb_dir)
            .context("failed to restore registry from snapshot")?;

        // Remove snapshot dir
        remove_snapshot(&snapshot_dir).context("failed to remove snapshot directory")?;

        // Add history entry
        store
            .add_history(ForkHistoryEntry {
                action: "exit".into(),
                network: net.clone(),
                timestamp: Utc::now(),
                details: None,
            })
            .context("failed to record exit history")?;

        if json {
            json_results.push(ForkExitJson {
                network: net.clone(),
                restored_entries,
                cleaned_up: true,
            });
        } else {
            exited_networks.push(net.clone());
        }
    }

    if json {
        if all {
            output::print_json(&json_results)?;
        } else {
            output::print_json(&json_results.into_iter().next().unwrap())?;
        }
    } else {
        println!("Exited fork mode.");
        println!();
        for net in &exited_networks {
            println!("  - {net}: registry restored, fork cleaned up");
        }
    }

    Ok(())
}

// ── run_revert ────────────────────────────────────────────────────────────────

/// Revert one or all active forks to their initial state.
///
/// For each target network:
/// 1. Calls `evm_revert` via Anvil JSON-RPC to restore EVM state (if a snapshot ID is stored).
/// 2. Takes a new EVM snapshot to establish a fresh baseline.
/// 3. Restores registry files from the snapshot directory.
/// 4. Adds a revert history entry.
pub async fn run_revert(network: String, all: bool, json: bool) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let treb_dir = cwd.join(TREB_DIR);

    let mut store = ForkStateStore::new(&treb_dir);
    store.load().context("failed to load fork state")?;

    let networks: Vec<String> = if all { store.list_active_networks() } else { vec![network] };

    if networks.is_empty() {
        if json {
            output::print_json(&Vec::<ForkRevertJson>::new())?;
        } else {
            println!("No active forks to revert.");
        }
        return Ok(());
    }

    let client = reqwest::Client::builder().timeout(Duration::from_secs(10)).build()?;
    let mut json_results: Vec<ForkRevertJson> = Vec::new();

    // Track revert statistics for human output.
    let mut total_reverted: usize = 0;
    let mut total_remaining: usize = 0;
    let mut reverted_commands = RevertCommandSummary::default();

    for net in &networks {
        let session = resolve_fork_session_entry(&store, net)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("network '{}' is not in fork mode", net))?;
        let runtime_entries: Vec<ForkEntry> = if json {
            primary_tracked_anvil_entry(&store, net).into_iter().cloned().collect()
        } else {
            tracked_anvil_entries_for_network(&store, net).into_iter().cloned().collect()
        };
        if json && runtime_entries.is_empty() {
            bail!(
                "network '{}' has no tracked Anvil instances; cannot emit JSON revert result; start one first with `treb dev anvil start --network {}`",
                net,
                net
            );
        }

        let mut json_snapshot_id = None;
        let mut json_new_snapshot_id = None;
        let mut history_snapshots: Vec<(String, String)> = Vec::new();

        for entry in runtime_entries {
            let instance_name = entry.resolved_instance_name().to_string();
            let old_snapshot_id =
                entry.snapshots.last().map(|snapshot| snapshot.snapshot_id.clone());
            if !is_port_reachable(entry.port).await {
                bail!(
                    "Anvil node for network '{}' is not reachable at port {}; cannot revert",
                    net,
                    entry.port
                );
            }

            if let Some(last_snapshot) = entry.snapshots.last() {
                // Track reverted command and counts for human output.
                if !json && !all {
                    reverted_commands.record(&last_snapshot.command);
                }
                if !json {
                    total_reverted += 1;
                    total_remaining += entry.snapshots.len().saturating_sub(1);
                }

                let reverted = evm_revert_http(&client, &entry.rpc_url, &last_snapshot.snapshot_id)
                    .await
                    .with_context(|| {
                        format!(
                            "failed to revert EVM state for network '{}' (instance '{}')",
                            net, instance_name
                        )
                    })?;
                if !reverted {
                    bail!(
                        "EVM revert failed for network '{}' (instance '{}', snapshot ID: {})",
                        net,
                        instance_name,
                        last_snapshot.snapshot_id
                    );
                }
            } else if !json {
                output::print_warning_banner(
                    "\u{26a0}\u{fe0f}",
                    &format!(
                        "No EVM snapshots stored for network '{net}' (instance '{}'); skipping EVM revert (registry will still be restored).",
                        instance_name
                    ),
                );
            }

            let new_snapshot_id =
                evm_snapshot_http(&client, &entry.rpc_url).await.with_context(|| {
                    format!(
                        "failed to take new EVM snapshot for network '{}' (instance '{}')",
                        net, instance_name
                    )
                })?;

            let mut updated = entry.clone();
            let next_index = updated.snapshots.len() as u32;
            updated.snapshots.push(treb_core::types::fork::SnapshotEntry {
                index: next_index,
                snapshot_id: new_snapshot_id.clone(),
                command: "revert".into(),
                timestamp: Utc::now(),
            });
            store.update_active_fork(updated).context("failed to update fork entry")?;

            history_snapshots.push((instance_name, new_snapshot_id.clone()));
            if json {
                json_snapshot_id = old_snapshot_id;
                json_new_snapshot_id = Some(new_snapshot_id);
            }
        }

        let snapshot_dir = PathBuf::from(&session.snapshot_dir);
        restore_registry(&snapshot_dir, &treb_dir)
            .context("failed to restore registry from snapshot")?;

        // Add history entry.
        store
            .add_history(ForkHistoryEntry {
                action: "revert".into(),
                network: net.clone(),
                timestamp: Utc::now(),
                details: match history_snapshots.as_slice() {
                    [] => None,
                    [(_, snapshot_id)] => Some(format!("new EVM snapshot: {snapshot_id}")),
                    snapshots => Some(format!(
                        "new EVM snapshots: {}",
                        snapshots
                            .iter()
                            .map(|(instance_name, snapshot_id)| format!(
                                "{instance_name}={snapshot_id}"
                            ))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                },
            })
            .context("failed to record revert history")?;

        if json {
            json_results.push(ForkRevertJson {
                network: net.clone(),
                snapshot_id: json_snapshot_id,
                new_snapshot_id: json_new_snapshot_id,
            });
        }
    }

    if json {
        if all {
            output::print_json(&json_results)?;
        } else {
            let result = json_results
                .into_iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("missing fork revert result"))?;
            output::print_json(&result)?;
        }
    } else {
        // Print Go-matching revert output.
        let message = revert_success_message(&networks, all);
        println!("{message}");
        println!();
        if let Some(cmd) = reverted_commands.renderable_command() {
            println!("  {:<12}{cmd}", "Reverted:");
        }
        println!("  {:<12}{total_reverted} snapshot(s)", "Reverted:");
        println!("  {:<12}{total_remaining} snapshot(s)", "Remaining:");
    }

    Ok(())
}

// ── run_restart ───────────────────────────────────────────────────────────────

/// Restart an Anvil fork from a fresh block.
///
/// Calls `anvil_reset` to reinitialize the Anvil instance with the same fork URL
/// (optionally at a different block number), re-deploys the CreateX factory,
/// takes a new EVM snapshot, restores the registry, and adds a restart history entry.
pub async fn run_restart(
    network: String,
    fork_block_number: Option<u64>,
    json: bool,
) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let treb_dir = cwd.join(TREB_DIR);

    let mut store = ForkStateStore::new(&treb_dir);
    store.load().context("failed to load fork state")?;

    let session = resolve_fork_session_entry(&store, &network)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("network '{}' is not in fork mode", network))?;
    let runtime_entries: Vec<ForkEntry> = if json {
        primary_tracked_anvil_entry(&store, &network).into_iter().cloned().collect()
    } else {
        tracked_anvil_entries_for_network(&store, &network).into_iter().cloned().collect()
    };
    if runtime_entries.is_empty() {
        bail!(
            "network '{}' has no tracked Anvil instances; start one first with `treb dev anvil start --network {}`",
            network,
            network
        );
    }

    let client = reqwest::Client::builder().timeout(Duration::from_secs(10)).build()?;

    // Determine the block to reset to.
    let blk = fork_block_number.or(session.fork_block_number);
    let mut json_result = None;
    let mut history_snapshots: Vec<(String, String)> = Vec::new();
    let mut setup_executed = false;

    for entry in &runtime_entries {
        let instance_name = entry.resolved_instance_name().to_string();
        if !is_port_reachable(entry.port).await {
            bail!(
                "Anvil node for network '{}' is not reachable at port {}; cannot restart",
                network,
                entry.port
            );
        }

        anvil_reset_http(&client, &entry.rpc_url, &entry.fork_url, blk).await.with_context(
            || {
                format!(
                    "failed to reset Anvil for network '{}' (instance '{}')",
                    network, instance_name
                )
            },
        )?;

        deploy_createx_http(&client, &entry.rpc_url).await.with_context(|| {
            format!(
                "failed to re-deploy CreateX for network '{}' (instance '{}')",
                network, instance_name
            )
        })?;

        setup_executed |=
            execute_fork_setup_if_configured(&cwd, &network, &entry.rpc_url, entry.chain_id)
                .await
                .with_context(|| {
                    format!(
                        "failed to execute fork setup for network '{}' (instance '{}')",
                        network, instance_name
                    )
                })?;

        let snapshot_id = evm_snapshot_http(&client, &entry.rpc_url).await.with_context(|| {
            format!(
                "failed to take EVM snapshot for network '{}' (instance '{}')",
                network, instance_name
            )
        })?;

        let mut updated = entry.clone();
        if let Some(b) = blk {
            updated.fork_block_number = Some(b);
        }
        let next_index = updated.snapshots.len() as u32;
        updated.snapshots.push(treb_core::types::fork::SnapshotEntry {
            index: next_index,
            snapshot_id: snapshot_id.clone(),
            command: "restart".into(),
            timestamp: Utc::now(),
        });
        store.update_active_fork(updated).context("failed to update fork entry")?;

        history_snapshots.push((instance_name.clone(), snapshot_id.clone()));
        if json {
            json_result = Some(ForkRestartJson {
                network: network.clone(),
                chain_id: entry.chain_id,
                port: entry.port,
                rpc_url: entry.rpc_url.clone(),
                snapshot_id: Some(snapshot_id),
            });
        }
    }

    // Restore registry from snapshot.
    let snapshot_dir = PathBuf::from(&session.snapshot_dir);
    restore_registry(&snapshot_dir, &treb_dir).context("failed to restore registry")?;

    if let Some(b) = blk {
        let session_has_runtime =
            runtime_entries.iter().any(|entry| entry.instance_name == session.instance_name);
        if !session_has_runtime {
            let mut updated = session.clone();
            updated.fork_block_number = Some(b);
            store.update_active_fork(updated).context("failed to update fork entry")?;
        }
    }

    // Add history entry.
    store
        .add_history(ForkHistoryEntry {
            action: "restart".into(),
            network: network.clone(),
            timestamp: Utc::now(),
            details: match history_snapshots.as_slice() {
                [] => None,
                [(_, snapshot_id)] => Some(format!("Anvil reset; snapshot: {snapshot_id}")),
                snapshots => Some(format!(
                    "Anvil reset; snapshots: {}",
                    snapshots
                        .iter()
                        .map(|(instance_name, snapshot_id)| format!(
                            "{instance_name}={snapshot_id}"
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            },
        })
        .context("failed to record restart history")?;

    if json {
        let result = json_result.ok_or_else(|| anyhow::anyhow!("missing fork restart result"))?;
        output::print_json(&result)?;
    } else {
        // Use the first runtime entry for the field display
        let display_entry = &runtime_entries[0];
        println!("Restarted fork for network '{network}'.");
        render_fork_fields(display_entry, setup_executed);
        println!();
        println!("Registry restored to initial fork state. All previous snapshots cleared.");
    }

    Ok(())
}

// ── Fork field rendering (Go-matching format) ────────────────────────────────

/// Render fork entry fields in Go-matching indented key-value format.
///
/// Prints 2-space indented fields with labels left-padded to 14 characters
/// (including colon), matching `render/fork.go`'s `RenderEnter` /
/// `RenderRestart` layout.  Fields whose values are empty or zero are
/// omitted.
fn render_fork_fields(entry: &ForkEntry, setup_executed: bool) {
    println!();
    for line in fork_field_lines(entry, setup_executed) {
        println!("{line}");
    }
}

fn fork_field_lines(entry: &ForkEntry, setup_executed: bool) -> Vec<String> {
    let mut lines = vec![
        format!("  {:<14}{}", "Network:", entry.network),
        format!("  {:<14}{}", "Chain ID:", entry.chain_id),
    ];

    // Go surfaces the local fork endpoint here. Rust only knows that value
    // once `dev anvil start` has filled in the runtime entry.
    if !entry.rpc_url.is_empty() {
        lines.push(format!("  {:<14}{}", "Fork URL:", entry.rpc_url));
    }
    if entry.anvil_pid != 0 {
        lines.push(format!("  {:<14}{}", "Anvil PID:", entry.anvil_pid));
    }
    if !entry.env_var_name.is_empty() && !entry.rpc_url.is_empty() {
        lines.push(format!("  {:<14}{}={}", "Env Override:", entry.env_var_name, entry.rpc_url));
    }
    if !entry.log_file.is_empty() {
        lines.push(format!("  {:<14}{}", "Logs:", entry.log_file));
    }
    if setup_executed {
        lines.push(format!("  {:<14}{}", "Setup:", "executed successfully"));
    }

    lines
}

async fn execute_fork_setup_if_configured(
    project_root: &Path,
    network: &str,
    rpc_url: &str,
    chain_id: u64,
) -> anyhow::Result<bool> {
    let resolved = resolve_config(ResolveOpts {
        project_root: project_root.to_path_buf(),
        namespace: None,
        network: Some(network.to_string()),
        profile: None,
        sender_overrides: HashMap::new(),
    })
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    let Some(script_path) = resolved.fork_setup.clone() else {
        return Ok(false);
    };

    let resolved_senders =
        resolve_all_senders(&resolved.senders).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut script_config =
        build_script_config_with_senders(&resolved, &script_path, &resolved_senders)
            .map_err(|e| anyhow::anyhow!("{e}"))?;

    script_config.rpc_url(rpc_url).chain_id(chain_id).broadcast(true).non_interactive(true);

    let script_args = script_config.into_script_args().map_err(|e| anyhow::anyhow!("{e}"))?;
    let _foundry_shell = FoundryShellGuard::suppress();
    let result = execute_script(script_args).await.map_err(|e| anyhow::anyhow!("{e}"))?;

    if !result.success {
        bail!("fork setup script '{}' did not complete successfully", script_path);
    }

    Ok(true)
}

// ── run_status ────────────────────────────────────────────────────────────────

/// Display active fork status with live port reachability checks.
pub async fn run_status(json: bool) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let treb_dir = cwd.join(TREB_DIR);

    ensure_treb_dir(&treb_dir)?;

    let mut store = ForkStateStore::new(&treb_dir);
    store.load().context("failed to load fork state")?;

    let networks = store.list_active_networks();

    let now = Utc::now();
    let deployments = load_json_map(&treb_dir.join(DEPLOYMENTS_FILE)).unwrap_or_default();

    if json {
        let mut statuses = Vec::new();
        for network in &networks {
            let session = resolve_fork_session_entry(&store, network)
                .ok_or_else(|| anyhow::anyhow!("network '{}' is not in fork mode", network))?;
            let runtime = primary_tracked_anvil_entry(&store, network);
            let rpc_url = runtime.map_or(session.rpc_url.as_str(), |entry| entry.rpc_url.as_str());
            let port = runtime.map_or(session.port, |entry| entry.port);
            let chain_id = runtime.map_or(session.chain_id, |entry| entry.chain_id);
            let started_at = runtime.map_or(session.started_at, |entry| entry.started_at);
            let snapshot_count =
                runtime.map_or(session.snapshots.len(), |entry| entry.snapshots.len());
            let running = network_is_running(runtime).await;
            let uptime = format_uptime(now - started_at);
            let deployment_count = count_fork_deployments_for_chain(&deployments, chain_id);
            statuses.push(serde_json::json!({
                "network":         network,
                "rpcUrl":          rpc_url,
                "port":            port,
                "chainId":         chain_id,
                "forkBlockNumber": session.fork_block_number,
                "startedAt":       started_at,
                "uptime":          uptime,
                "snapshotCount":   snapshot_count,
                "deploymentCount": deployment_count,
                "status":          if running { "running" } else { "stopped" },
            }));
        }
        output::print_json(&statuses)?;
        return Ok(());
    }

    if networks.is_empty() {
        println!("No active forks.");
        return Ok(());
    }

    println!("Active Forks");
    println!();

    let current_network = load_local_config(&cwd)
        .ok()
        .and_then(|config| (!config.network.is_empty()).then_some(config.network));

    for network in &networks {
        let session = resolve_fork_session_entry(&store, network)
            .ok_or_else(|| anyhow::anyhow!("network '{}' is not in fork mode", network))?;
        let runtime = primary_tracked_anvil_entry(&store, network);
        let rpc_url = status_fork_url(session, runtime);
        let chain_id = runtime.map_or(session.chain_id, |entry| entry.chain_id);
        let started_at = runtime.map_or(session.started_at, |entry| entry.started_at);
        let snapshot_count = runtime.map_or(session.snapshots.len(), |entry| entry.snapshots.len());
        let running = network_is_running(runtime).await;
        let health_detail = if running { "running" } else { "stopped" };
        let uptime = format_status_uptime(now - started_at);
        let deployment_count = count_fork_deployments_for_chain(&deployments, chain_id);
        let anvil_pid = runtime.map_or(session.anvil_pid, |entry| entry.anvil_pid);
        let log_file = runtime.map_or(session.log_file.as_str(), |entry| entry.log_file.as_str());

        println!("{}", status_entry_header(network, current_network.as_deref()));
        println!("    {:<14}{}", "Chain ID:", chain_id);
        if !rpc_url.is_empty() {
            println!("    {:<14}{}", "Fork URL:", rpc_url);
        }
        if anvil_pid != 0 {
            println!("    {:<14}{}", "Anvil PID:", anvil_pid);
        }
        println!("    {:<14}{}", "Status:", health_detail);
        println!("    {:<14}{}", "Uptime:", uptime);
        println!("    {:<14}{}", "Snapshots:", snapshot_count);
        println!("    {:<14}{}", "Fork Deploys:", deployment_count);
        if !log_file.is_empty() {
            println!("    {:<14}{}", "Logs:", log_file);
        }
        println!();
    }

    Ok(())
}

// ── run_history ───────────────────────────────────────────────────────────────

/// Display fork history, optionally filtered by network.
pub async fn run_history(network: Option<String>, json: bool) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let treb_dir = cwd.join(TREB_DIR);
    ensure_treb_dir(&treb_dir)?;

    let mut store = ForkStateStore::new(&treb_dir);
    store.load().context("failed to load fork state")?;

    let history: Vec<_> = store
        .data()
        .history
        .iter()
        .filter(|e| network.as_deref().is_none_or(|n| e.network == n))
        .collect();

    if json {
        output::print_json(&history)?;
        return Ok(());
    }

    if history.is_empty() {
        let filter_msg =
            network.as_deref().map_or_else(String::new, |n| format!(" for network '{n}'"));
        println!("No fork history{filter_msg}.");
        return Ok(());
    }

    // Collect unique networks in order of first appearance (most-recent-first).
    let mut networks: Vec<String> = Vec::new();
    for entry in &history {
        if !networks.contains(&entry.network) {
            networks.push(entry.network.clone());
        }
    }

    for net in &networks {
        // Filter entries for this network and reverse to chronological order.
        let net_entries: Vec<_> = history.iter().filter(|e| e.network == *net).rev().collect();
        let last_idx = net_entries.len() - 1;

        println!("Fork History: {net}");
        println!();

        for (i, entry) in net_entries.iter().enumerate() {
            let marker = if i == last_idx { "\u{2192} " } else { "  " };
            let label = if i == 0 {
                "initial".to_string()
            } else {
                match &entry.details {
                    Some(d) => format!("{} {d}", entry.action),
                    None => entry.action.clone(),
                }
            };
            let ts = entry.timestamp.format("%Y-%m-%d %H:%M:%S");
            println!("  {marker}[{i}] {label}  ({ts})");
        }

        println!();
    }

    Ok(())
}

// ── run_diff ──────────────────────────────────────────────────────────────────

/// Compare the current registry against the fork snapshot for a network.
///
/// Reports added, removed, and modified entries in `deployments.json` and
/// `transactions.json`. Supports `--json` output.
pub async fn run_diff(network: String, json: bool) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let treb_dir = cwd.join(TREB_DIR);

    let mut store = ForkStateStore::new(&treb_dir);
    store.load().context("failed to load fork state")?;

    let entry = resolve_fork_session_entry(&store, &network)
        .ok_or_else(|| anyhow::anyhow!("network '{}' is not in fork mode", network))?;

    let snapshot_dir = PathBuf::from(&entry.snapshot_dir);

    // Files to diff.
    let diff_files = [DEPLOYMENTS_FILE, TRANSACTIONS_FILE];
    let mut changes: Vec<serde_json::Value> = Vec::new();

    for &file_name in &diff_files {
        let current_path = treb_dir.join(file_name);
        let snapshot_path = snapshot_dir.join(file_name);

        let current_map = load_json_map(&current_path);
        let snapshot_map = load_json_map(&snapshot_path);

        // Collect all keys from both maps.
        let all_keys: BTreeSet<String> = current_map
            .iter()
            .flat_map(|m| m.keys().cloned())
            .chain(snapshot_map.iter().flat_map(|m| m.keys().cloned()))
            .collect();

        let file_label = file_name.trim_end_matches(".json");

        for key in &all_keys {
            let in_current = current_map.as_ref().and_then(|m| m.get(key));
            let in_snapshot = snapshot_map.as_ref().and_then(|m| m.get(key));

            let change_type = match (in_current, in_snapshot) {
                (Some(_), None) => "added",
                (None, Some(_)) => "removed",
                (Some(a), Some(b)) if a != b => "modified",
                _ => continue, // unchanged
            };

            changes.push(serde_json::json!({
                "change": change_type,
                "file":   file_label,
                "key":    key,
            }));
        }
    }

    if json {
        output::print_json(&serde_json::json!({
            "network": network,
            "changes": changes,
            "clean":   changes.is_empty(),
        }))?;
        return Ok(());
    }

    if changes.is_empty() {
        println!("No changes detected for network '{network}'.");
        return Ok(());
    }

    let mut table = crate::output::build_table(&["Change", "File", "Key"]);
    for change in &changes {
        let change_text = change["change"].as_str().unwrap_or("");
        let change_style = match change_text {
            "added" => Style::new().green(),
            "removed" => Style::new().red(),
            "modified" => Style::new().yellow(),
            _ => Style::new(),
        };
        table.add_row(vec![
            styled(change_text, change_style),
            change["file"].as_str().unwrap_or("").to_string(),
            styled(change["key"].as_str().unwrap_or(""), color::LABEL),
        ]);
    }
    crate::output::print_table(&table);
    Ok(())
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn is_tracked_anvil_instance(entry: &ForkEntry) -> bool {
    entry.port != 0 || !entry.pid_file.is_empty() || !entry.log_file.is_empty()
}

fn resolve_fork_session_entry<'a>(
    store: &'a ForkStateStore,
    network: &str,
) -> Option<&'a ForkEntry> {
    store.get_active_fork(network).or_else(|| {
        store.list_active_forks_for_network(network).into_iter().min_by(|left, right| {
            left.instance_name
                .is_some()
                .cmp(&right.instance_name.is_some())
                .then_with(|| left.resolved_instance_name().cmp(right.resolved_instance_name()))
        })
    })
}

fn exit_ordered_networks(store: &ForkStateStore) -> Vec<String> {
    let mut networks: Vec<(String, chrono::DateTime<Utc>)> = store
        .list_active_networks()
        .into_iter()
        .filter_map(|network| {
            resolve_fork_session_entry(store, &network).map(|entry| (network, entry.entered_at))
        })
        .collect();

    networks.sort_by(|(left_network, left_entered_at), (right_network, right_entered_at)| {
        right_entered_at.cmp(left_entered_at).then_with(|| left_network.cmp(right_network))
    });

    networks.into_iter().map(|(network, _)| network).collect()
}

fn tracked_anvil_entries_for_network<'a>(
    store: &'a ForkStateStore,
    network: &str,
) -> Vec<&'a ForkEntry> {
    let mut entries: Vec<&ForkEntry> = store
        .list_active_forks_for_network(network)
        .into_iter()
        .filter(|entry| is_tracked_anvil_instance(entry))
        .collect();
    entries.sort_by(|left, right| {
        left.instance_name
            .is_some()
            .cmp(&right.instance_name.is_some())
            .then_with(|| left.resolved_instance_name().cmp(right.resolved_instance_name()))
    });
    entries
}

fn primary_tracked_anvil_entry<'a>(
    store: &'a ForkStateStore,
    network: &str,
) -> Option<&'a ForkEntry> {
    tracked_anvil_entries_for_network(store, network).into_iter().next()
}

async fn network_is_running(entry: Option<&ForkEntry>) -> bool {
    match entry {
        Some(entry) => is_port_reachable(entry.port).await,
        None => false,
    }
}

fn status_fork_url<'a>(session: &'a ForkEntry, runtime: Option<&'a ForkEntry>) -> &'a str {
    runtime.filter(|entry| !entry.rpc_url.is_empty()).map_or_else(
        || {
            if !session.fork_url.is_empty() {
                session.fork_url.as_str()
            } else {
                session.rpc_url.as_str()
            }
        },
        |entry| entry.rpc_url.as_str(),
    )
}

fn format_status_uptime(duration: chrono::Duration) -> String {
    output::format_duration(duration.to_std().unwrap_or(Duration::ZERO))
}

fn status_entry_header(network: &str, current_network: Option<&str>) -> String {
    if current_network == Some(network) {
        format!("  {network} (current)")
    } else {
        format!("  {network}")
    }
}

fn revert_success_message(networks: &[String], all: bool) -> String {
    if all {
        "Reverted all active forks.".to_string()
    } else {
        format!("Reverted fork for network '{}'.", networks[0])
    }
}

fn styled(text: &str, style: Style) -> String {
    if color::is_color_enabled() { format!("{}", text.style(style)) } else { text.to_string() }
}

/// Format a duration as a human-readable uptime string.
///
/// Examples: `"2h 15m"`, `"3d 1h"`, `"< 1m"`, `"45m"`.
fn format_uptime(duration: chrono::Duration) -> String {
    let total_secs = duration.num_seconds().max(0);
    let days = total_secs / 86400;
    let hours = (total_secs % 86400) / 3600;
    let minutes = (total_secs % 3600) / 60;

    if days > 0 {
        if hours > 0 { format!("{days}d {hours}h") } else { format!("{days}d") }
    } else if hours > 0 {
        if minutes > 0 { format!("{hours}h {minutes}m") } else { format!("{hours}h") }
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        "< 1m".to_string()
    }
}

/// Count deployments in the registry for the given fork chain namespace.
fn count_fork_deployments_for_chain(
    deployments: &serde_json::Map<String, serde_json::Value>,
    chain_id: u64,
) -> usize {
    let prefix = format!("fork/{chain_id}/");
    deployments.keys().filter(|k| k.starts_with(&prefix)).count()
}

fn ensure_treb_dir(treb_dir: &Path) -> anyhow::Result<()> {
    if !treb_dir.exists() {
        bail!(
            "project not initialized — .treb/ directory not found\n\n\
             Run `treb init` first."
        );
    }
    Ok(())
}

fn resolve_fork_url(
    cwd: &Path,
    network: &str,
    rpc_url_override: Option<String>,
) -> anyhow::Result<String> {
    if let Some(url) = rpc_url_override {
        return Ok(url);
    }
    let foundry_config =
        treb_config::load_foundry_config(cwd).map_err(|e| anyhow::anyhow!("{e}"))?;
    let endpoints = treb_config::rpc_endpoints(&foundry_config);
    endpoints.get(network).cloned().ok_or_else(|| {
        anyhow::anyhow!(
            "no RPC URL configured for network '{}' in foundry.toml\n\n\
             Add it under [rpc_endpoints] or pass --rpc-url to specify directly.",
            network
        )
    })
}

async fn fetch_chain_id(rpc_url: &str) -> anyhow::Result<u64> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(10)).build()?;
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "eth_chainId",
        "params": [],
        "id": 1
    });

    let resp = client
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("failed to connect to {rpc_url}"))?;

    let json: serde_json::Value =
        resp.json().await.context("failed to parse eth_chainId response")?;

    let hex = json
        .get("result")
        .and_then(|r| r.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'result' in eth_chainId response"))?;

    let hex = hex.strip_prefix("0x").unwrap_or(hex);
    u64::from_str_radix(hex, 16).with_context(|| format!("invalid chain ID hex: '{hex}'"))
}

/// Generic JSON-RPC call over HTTP.  Returns the `result` field of the response.
async fn json_rpc_call(
    client: &reqwest::Client,
    url: &str,
    method: &str,
    params: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let params_arr = match params {
        serde_json::Value::Null => serde_json::json!([]),
        serde_json::Value::Array(a) => serde_json::Value::Array(a),
        other => serde_json::json!([other]),
    };
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method":  method,
        "params":  params_arr,
        "id":      1,
    });
    let resp = client
        .post(url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("failed to connect to {url}"))?;
    let json: serde_json::Value = resp.json().await.context("failed to parse JSON-RPC response")?;
    if let Some(err) = json.get("error") {
        bail!("JSON-RPC error from {url}: {err}");
    }
    json.get("result")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("missing 'result' in JSON-RPC response"))
}

/// Take an EVM snapshot and return the snapshot ID as a hex string.
pub(crate) async fn evm_snapshot_http(
    client: &reqwest::Client,
    rpc_url: &str,
) -> anyhow::Result<String> {
    let result = json_rpc_call(client, rpc_url, "evm_snapshot", serde_json::Value::Null).await?;
    result
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("unexpected evm_snapshot result type: {result}"))
}

/// Revert EVM state to a previously created snapshot.  Returns `true` on success.
async fn evm_revert_http(
    client: &reqwest::Client,
    rpc_url: &str,
    snapshot_id: &str,
) -> anyhow::Result<bool> {
    let result =
        json_rpc_call(client, rpc_url, "evm_revert", serde_json::json!([snapshot_id])).await?;
    result.as_bool().ok_or_else(|| anyhow::anyhow!("unexpected evm_revert result type: {result}"))
}

/// Reset Anvil to a fresh fork state using `anvil_reset`.
async fn anvil_reset_http(
    client: &reqwest::Client,
    rpc_url: &str,
    fork_url: &str,
    fork_block_number: Option<u64>,
) -> anyhow::Result<()> {
    let forking = if let Some(blk) = fork_block_number {
        serde_json::json!({ "jsonRpcUrl": fork_url, "blockNumber": blk })
    } else {
        serde_json::json!({ "jsonRpcUrl": fork_url })
    };
    json_rpc_call(client, rpc_url, "anvil_reset", serde_json::json!([{ "forking": forking }]))
        .await?;
    Ok(())
}

/// Deploy the CreateX factory bytecode at its canonical address via `anvil_setCode`.
async fn deploy_createx_http(client: &reqwest::Client, rpc_url: &str) -> anyhow::Result<()> {
    let bytecode_bytes = createx_deployed_bytecode();
    let hex: String = bytecode_bytes.iter().map(|b| format!("{b:02x}")).collect();
    let hex_str = format!("0x{hex}");
    json_rpc_call(client, rpc_url, "anvil_setCode", serde_json::json!([CREATEX_ADDRESS, hex_str]))
        .await?;
    Ok(())
}

/// Check whether a port on 127.0.0.1 is currently accepting TCP connections.
async fn is_port_reachable(port: u16) -> bool {
    if port == 0 {
        return false;
    }
    let addr = format!("127.0.0.1:{port}");
    TcpStream::connect(&addr).await.is_ok()
}

/// Load a JSON file as an object map.  Returns `None` if the file is missing or not an object.
fn load_json_map(path: &Path) -> Option<serde_json::Map<String, serde_json::Value>> {
    let content = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    value.as_object().cloned()
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::ForkSubcommand;
    use chrono::Utc;
    use clap::{Parser, Subcommand};
    use std::{fs, path::PathBuf};
    use tempfile::TempDir;
    use treb_core::types::fork::{ForkEntry, ForkHistoryEntry};
    use treb_registry::{DEPLOYMENTS_FILE, ForkStateStore, restore_registry, snapshot_registry};

    // ── Minimal test CLI for clap parsing ─────────────────────────────────

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: TestCommands,
    }

    #[derive(Subcommand)]
    enum TestCommands {
        Fork {
            #[command(subcommand)]
            subcommand: ForkSubcommand,
        },
    }

    fn parse_fork(args: &[&str]) -> anyhow::Result<ForkSubcommand> {
        let mut argv = vec!["treb", "fork"];
        argv.extend_from_slice(args);
        let cli = TestCli::try_parse_from(argv).map_err(|e| anyhow::anyhow!("{e}"))?;
        match cli.command {
            TestCommands::Fork { subcommand } => Ok(subcommand),
        }
    }

    // ── clap parsing tests ────────────────────────────────────────────────

    #[test]
    fn parse_enter_network_only() {
        let sub = parse_fork(&["enter", "--network", "mainnet"]).unwrap();
        match sub {
            ForkSubcommand::Enter { network, rpc_url, fork_block_number, json } => {
                assert_eq!(network, "mainnet");
                assert!(rpc_url.is_none());
                assert!(fork_block_number.is_none());
                assert!(!json);
            }
            _ => panic!("expected Enter"),
        }
    }

    #[test]
    fn parse_enter_with_all_flags() {
        let sub = parse_fork(&[
            "enter",
            "--network",
            "mainnet",
            "--rpc-url",
            "https://eth.example.com",
            "--fork-block-number",
            "19000000",
            "--json",
        ])
        .unwrap();
        match sub {
            ForkSubcommand::Enter { network, rpc_url, fork_block_number, json } => {
                assert_eq!(network, "mainnet");
                assert_eq!(rpc_url.as_deref(), Some("https://eth.example.com"));
                assert_eq!(fork_block_number, Some(19_000_000));
                assert!(json);
            }
            _ => panic!("expected Enter"),
        }
    }

    #[test]
    fn parse_exit() {
        let sub = parse_fork(&["exit", "--network", "sepolia"]).unwrap();
        match sub {
            ForkSubcommand::Exit { network, all, json } => {
                assert_eq!(network, "sepolia");
                assert!(!all);
                assert!(!json);
            }
            _ => panic!("expected Exit"),
        }
    }

    #[test]
    fn parse_status_json() {
        let sub = parse_fork(&["status", "--json"]).unwrap();
        match sub {
            ForkSubcommand::Status { json } => assert!(json),
            _ => panic!("expected Status"),
        }
    }

    #[test]
    fn parse_history_with_filter() {
        let sub = parse_fork(&["history", "--network", "mainnet"]).unwrap();
        match sub {
            ForkSubcommand::History { network, json } => {
                assert_eq!(network.as_deref(), Some("mainnet"));
                assert!(!json);
            }
            _ => panic!("expected History"),
        }
    }

    #[test]
    fn parse_diff_with_json() {
        let sub = parse_fork(&["diff", "--network", "mainnet", "--json"]).unwrap();
        match sub {
            ForkSubcommand::Diff { network, json } => {
                assert_eq!(network, "mainnet");
                assert!(json);
            }
            _ => panic!("expected Diff"),
        }
    }

    // ── helpers ───────────────────────────────────────────────────────────

    fn make_treb_dir() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let treb_dir = dir.path().join(".treb");
        fs::create_dir_all(&treb_dir).unwrap();
        (dir, treb_dir)
    }

    fn sample_entry(treb_dir: &std::path::Path, network: &str) -> ForkEntry {
        let now = Utc::now();
        ForkEntry {
            network: network.to_string(),
            instance_name: None,
            rpc_url: String::new(),
            port: 0,
            chain_id: 1,
            fork_url: "https://eth.example.com".into(),
            fork_block_number: None,
            snapshot_dir: treb_dir.join("snapshots").join(network).to_string_lossy().into_owned(),
            started_at: now,
            env_var_name: String::new(),
            original_rpc: String::new(),
            anvil_pid: 0,
            pid_file: String::new(),
            log_file: String::new(),
            entered_at: now,
            snapshots: vec![],
        }
    }

    fn sample_named_entry(
        treb_dir: &std::path::Path,
        network: &str,
        instance_name: &str,
    ) -> ForkEntry {
        let mut entry = sample_entry(treb_dir, network);
        entry.instance_name = Some(instance_name.to_string());
        entry
    }

    // ── fork state written on enter ───────────────────────────────────────

    #[test]
    fn fork_state_written_on_enter() {
        let (_root, treb_dir) = make_treb_dir();
        let mut store = ForkStateStore::new(&treb_dir);
        store.insert_active_fork(sample_entry(&treb_dir, "mainnet")).unwrap();
        store
            .add_history(ForkHistoryEntry {
                action: "enter".into(),
                network: "mainnet".into(),
                timestamp: Utc::now(),
                details: None,
            })
            .unwrap();

        // Read back in fresh store
        let mut store2 = ForkStateStore::new(&treb_dir);
        store2.load().unwrap();

        let fork = store2.get_active_fork("mainnet").unwrap();
        assert_eq!(fork.network, "mainnet");
        assert_eq!(fork.chain_id, 1);
        assert_eq!(store2.data().history.len(), 1);
        assert_eq!(store2.data().history[0].action, "enter");
    }

    // ── registry snapshot/restore round-trip ─────────────────────────────

    #[test]
    fn registry_snapshot_restore_round_trip() {
        let (_root, treb_dir) = make_treb_dir();
        let snapshot_dir = treb_dir.join("snapshots").join("mainnet");

        // Write initial registry state
        fs::write(treb_dir.join(DEPLOYMENTS_FILE), r#"{"original": true}"#).unwrap();

        // Snapshot
        snapshot_registry(&treb_dir, &snapshot_dir).unwrap();

        // Overwrite registry (simulate deployments during fork session)
        fs::write(treb_dir.join(DEPLOYMENTS_FILE), r#"{"modified": true}"#).unwrap();

        // Restore
        restore_registry(&snapshot_dir, &treb_dir).unwrap();

        let content = fs::read_to_string(treb_dir.join(DEPLOYMENTS_FILE)).unwrap();
        assert_eq!(content, r#"{"original": true}"#);
    }

    // ── error on duplicate enter ──────────────────────────────────────────

    #[test]
    fn error_on_duplicate_enter() {
        let (_root, treb_dir) = make_treb_dir();
        let mut store = ForkStateStore::new(&treb_dir);
        store.insert_active_fork(sample_entry(&treb_dir, "mainnet")).unwrap();

        let result = store.insert_active_fork(sample_entry(&treb_dir, "mainnet"));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("already forked"), "expected 'already forked' in: {msg}");
    }

    // ── error on non-active exit ──────────────────────────────────────────

    #[test]
    fn error_on_non_active_exit() {
        let (_root, treb_dir) = make_treb_dir();
        let mut store = ForkStateStore::new(&treb_dir);
        store.load().unwrap();

        let result = store.remove_active_fork("mainnet");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("not actively forked"), "expected 'not actively forked' in: {msg}");
    }

    // ── status with no forks ──────────────────────────────────────────────

    #[test]
    fn status_with_no_forks() {
        let (_root, treb_dir) = make_treb_dir();
        let mut store = ForkStateStore::new(&treb_dir);
        store.load().unwrap();

        let forks = store.list_active_forks();
        assert!(forks.is_empty(), "expected no active forks");
    }

    // ── status with active forks ──────────────────────────────────────────

    #[test]
    fn status_with_active_forks() {
        let (_root, treb_dir) = make_treb_dir();
        let mut store = ForkStateStore::new(&treb_dir);

        let mut entry = sample_entry(&treb_dir, "mainnet");
        entry.rpc_url = "http://127.0.0.1:8545".into();
        entry.port = 8545;
        entry.chain_id = 1;
        store.insert_active_fork(entry).unwrap();

        let mut store2 = ForkStateStore::new(&treb_dir);
        store2.load().unwrap();
        let forks = store2.list_active_forks();

        assert_eq!(forks.len(), 1);
        assert_eq!(forks[0].network, "mainnet");
        assert_eq!(forks[0].port, 8545);
        assert_eq!(forks[0].chain_id, 1);
    }

    #[test]
    fn resolve_fork_session_entry_prefers_default_session() {
        let (_root, treb_dir) = make_treb_dir();
        let mut store = ForkStateStore::new(&treb_dir);

        let default_entry = sample_entry(&treb_dir, "mainnet");
        let mut named_entry = sample_named_entry(&treb_dir, "mainnet", "alpha");
        named_entry.port = 18545;
        named_entry.rpc_url = "http://127.0.0.1:18545".into();

        store.insert_active_fork(named_entry).unwrap();
        store.insert_active_fork(default_entry.clone()).unwrap();

        let resolved = super::resolve_fork_session_entry(&store, "mainnet").unwrap();
        assert_eq!(resolved.instance_name, None);
        assert_eq!(resolved.snapshot_dir, default_entry.snapshot_dir);
    }

    #[test]
    fn tracked_anvil_entries_for_network_ignores_placeholder_session() {
        let (_root, treb_dir) = make_treb_dir();
        let mut store = ForkStateStore::new(&treb_dir);

        let placeholder = sample_entry(&treb_dir, "mainnet");
        let mut named_entry = sample_named_entry(&treb_dir, "mainnet", "alpha");
        named_entry.port = 18545;
        named_entry.rpc_url = "http://127.0.0.1:18545".into();
        named_entry.pid_file = ".treb/anvil-alpha.pid".into();
        named_entry.log_file = ".treb/anvil-alpha.log".into();

        store.insert_active_fork(placeholder).unwrap();
        store.insert_active_fork(named_entry).unwrap();

        let tracked = super::tracked_anvil_entries_for_network(&store, "mainnet");
        assert_eq!(tracked.len(), 1);
        assert_eq!(tracked[0].resolved_instance_name(), "alpha");
    }

    #[test]
    fn primary_tracked_anvil_entry_prefers_default_named_order() {
        let (_root, treb_dir) = make_treb_dir();
        let mut store = ForkStateStore::new(&treb_dir);

        let mut beta = sample_named_entry(&treb_dir, "mainnet", "beta");
        beta.port = 18546;
        beta.rpc_url = "http://127.0.0.1:18546".into();

        let mut alpha = sample_named_entry(&treb_dir, "mainnet", "alpha");
        alpha.port = 18545;
        alpha.rpc_url = "http://127.0.0.1:18545".into();

        store.insert_active_fork(beta).unwrap();
        store.insert_active_fork(alpha).unwrap();

        let primary = super::primary_tracked_anvil_entry(&store, "mainnet").unwrap();
        assert_eq!(primary.resolved_instance_name(), "alpha");
    }

    #[test]
    fn status_fork_url_uses_session_fork_url_without_runtime() {
        let (_root, treb_dir) = make_treb_dir();
        let session = sample_entry(&treb_dir, "mainnet");

        assert_eq!(super::status_fork_url(&session, None), "https://eth.example.com");
    }

    #[test]
    fn status_fork_url_prefers_runtime_rpc_url_when_available() {
        let (_root, treb_dir) = make_treb_dir();
        let session = sample_entry(&treb_dir, "mainnet");
        let mut runtime = sample_named_entry(&treb_dir, "mainnet", "alpha");
        runtime.rpc_url = "http://127.0.0.1:18545".into();
        runtime.port = 18545;
        runtime.log_file = "/tmp/anvil-alpha.log".into();

        assert_eq!(super::status_fork_url(&session, Some(&runtime)), "http://127.0.0.1:18545");
    }

    #[test]
    fn status_entry_header_marks_current_network() {
        assert_eq!(super::status_entry_header("mainnet", Some("mainnet")), "  mainnet (current)");
        assert_eq!(super::status_entry_header("sepolia", Some("mainnet")), "  sepolia");
    }

    #[test]
    fn format_status_uptime_matches_go_duration_format() {
        assert_eq!(super::format_status_uptime(chrono::Duration::seconds(45)), "45s");
        assert_eq!(super::format_status_uptime(chrono::Duration::minutes(135)), "2h15m");
    }

    #[test]
    fn format_status_uptime_handles_negative_duration() {
        assert_eq!(super::format_status_uptime(chrono::Duration::seconds(-10)), "0s");
    }

    #[test]
    fn revert_success_message_uses_all_flag_for_single_active_network() {
        let networks = vec!["mainnet".to_string()];

        assert_eq!(super::revert_success_message(&networks, true), "Reverted all active forks.");
        assert_eq!(
            super::revert_success_message(&networks, false),
            "Reverted fork for network 'mainnet'."
        );
    }

    #[test]
    fn revert_command_summary_renders_shared_command() {
        let mut summary = super::RevertCommandSummary::default();
        summary.record("deploy");
        summary.record("deploy");

        assert_eq!(summary.renderable_command(), Some("deploy"));
    }

    #[test]
    fn revert_command_summary_hides_mixed_commands() {
        let mut summary = super::RevertCommandSummary::default();
        summary.record("deploy");
        summary.record("upgrade");

        assert_eq!(summary.renderable_command(), None);
    }

    #[test]
    fn fork_field_lines_omit_runtime_only_fields_for_placeholder_entry() {
        let (_root, treb_dir) = make_treb_dir();
        let mut entry = sample_entry(&treb_dir, "mainnet");
        entry.env_var_name = "ETH_RPC_URL_MAINNET".into();

        let lines = super::fork_field_lines(&entry, false);
        let joined = lines.join("\n");

        assert!(joined.contains("Network:"));
        assert!(joined.contains("Chain ID:"));
        assert!(!joined.contains("Fork URL:"));
        assert!(!joined.contains("Env Override:"));
        assert!(!joined.contains("https://eth.example.com"));
    }

    #[test]
    fn fork_field_lines_use_local_runtime_fields_and_setup_marker() {
        let (_root, treb_dir) = make_treb_dir();
        let mut entry = sample_entry(&treb_dir, "mainnet");
        entry.rpc_url = "http://127.0.0.1:8545".into();
        entry.env_var_name = "ETH_RPC_URL_MAINNET".into();
        entry.anvil_pid = 4321;
        entry.log_file = "/tmp/anvil-mainnet.log".into();

        let lines = super::fork_field_lines(&entry, true);
        let joined = lines.join("\n");

        assert!(joined.contains("Fork URL:     http://127.0.0.1:8545"));
        assert!(joined.contains("Anvil PID:    4321"));
        assert!(joined.contains("Env Override: ETH_RPC_URL_MAINNET=http://127.0.0.1:8545"));
        assert!(joined.contains("Logs:         /tmp/anvil-mainnet.log"));
        assert!(joined.contains("Setup:        executed successfully"));
        assert!(!joined.contains("https://eth.example.com"));
    }

    // ── history filtering ─────────────────────────────────────────────────

    #[test]
    fn history_filtering() {
        let (_root, treb_dir) = make_treb_dir();
        let mut store = ForkStateStore::new(&treb_dir);

        store
            .add_history(ForkHistoryEntry {
                action: "enter".into(),
                network: "mainnet".into(),
                timestamp: Utc::now(),
                details: None,
            })
            .unwrap();
        store
            .add_history(ForkHistoryEntry {
                action: "enter".into(),
                network: "sepolia".into(),
                timestamp: Utc::now(),
                details: None,
            })
            .unwrap();

        let mut store2 = ForkStateStore::new(&treb_dir);
        store2.load().unwrap();

        let all_history: Vec<_> = store2.data().history.iter().collect();
        assert_eq!(all_history.len(), 2);

        let mainnet_only: Vec<_> =
            store2.data().history.iter().filter(|e| e.network == "mainnet").collect();
        assert_eq!(mainnet_only.len(), 1);
        assert_eq!(mainnet_only[0].action, "enter");
        assert_eq!(mainnet_only[0].network, "mainnet");
    }

    // ── history with empty history ────────────────────────────────────────

    #[test]
    fn history_with_empty_history() {
        let (_root, treb_dir) = make_treb_dir();
        let mut store = ForkStateStore::new(&treb_dir);
        store.load().unwrap();

        let history: Vec<_> = store.data().history.iter().collect();
        assert!(history.is_empty(), "expected empty history");
    }

    // ── diff detects changes ──────────────────────────────────────────────

    #[test]
    fn diff_detects_changes() {
        let (_root, treb_dir) = make_treb_dir();
        let snapshot_dir = treb_dir.join("snapshots").join("mainnet");
        fs::create_dir_all(&snapshot_dir).unwrap();

        // Write matching state to both locations first.
        let deployments_json = r#"{"Counter_1": {"address": "0xaaa"}}"#;
        fs::write(treb_dir.join(DEPLOYMENTS_FILE), deployments_json).unwrap();
        fs::write(snapshot_dir.join(DEPLOYMENTS_FILE), deployments_json).unwrap();

        // Simulate a new deployment added to the current registry.
        let current_json =
            r#"{"Counter_1": {"address": "0xaaa"}, "Token_2": {"address": "0xbbb"}}"#;
        fs::write(treb_dir.join(DEPLOYMENTS_FILE), current_json).unwrap();

        // Diff
        let current_map = super::load_json_map(&treb_dir.join(DEPLOYMENTS_FILE)).unwrap();
        let snapshot_map = super::load_json_map(&snapshot_dir.join(DEPLOYMENTS_FILE)).unwrap();

        let added: Vec<_> = current_map.keys().filter(|k| !snapshot_map.contains_key(*k)).collect();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0], "Token_2");

        let removed: Vec<_> =
            snapshot_map.keys().filter(|k| !current_map.contains_key(*k)).collect();
        assert!(removed.is_empty());
    }

    // ── diff shows clean when matching ────────────────────────────────────

    #[test]
    fn diff_shows_clean_when_matching() {
        let (_root, treb_dir) = make_treb_dir();
        let snapshot_dir = treb_dir.join("snapshots").join("mainnet");
        fs::create_dir_all(&snapshot_dir).unwrap();

        let deployments_json = r#"{"Counter_1": {"address": "0xaaa"}}"#;
        fs::write(treb_dir.join(DEPLOYMENTS_FILE), deployments_json).unwrap();
        fs::write(snapshot_dir.join(DEPLOYMENTS_FILE), deployments_json).unwrap();

        let current_map = super::load_json_map(&treb_dir.join(DEPLOYMENTS_FILE)).unwrap();
        let snapshot_map = super::load_json_map(&snapshot_dir.join(DEPLOYMENTS_FILE)).unwrap();

        let changes: Vec<_> = current_map
            .keys()
            .filter(|k| !snapshot_map.contains_key(*k))
            .chain(snapshot_map.keys().filter(|k| !current_map.contains_key(*k)))
            .collect();
        assert!(changes.is_empty(), "expected no changes: {changes:?}");
    }

    // ── format_uptime tests ──────────────────────────────────────────────

    #[test]
    fn format_uptime_less_than_one_minute() {
        let dur = chrono::Duration::seconds(30);
        assert_eq!(super::format_uptime(dur), "< 1m");
    }

    #[test]
    fn format_uptime_zero() {
        let dur = chrono::Duration::seconds(0);
        assert_eq!(super::format_uptime(dur), "< 1m");
    }

    #[test]
    fn format_uptime_negative() {
        let dur = chrono::Duration::seconds(-10);
        assert_eq!(super::format_uptime(dur), "< 1m");
    }

    #[test]
    fn format_uptime_minutes_only() {
        let dur = chrono::Duration::minutes(45);
        assert_eq!(super::format_uptime(dur), "45m");
    }

    #[test]
    fn format_uptime_hours_and_minutes() {
        let dur = chrono::Duration::minutes(135); // 2h 15m
        assert_eq!(super::format_uptime(dur), "2h 15m");
    }

    #[test]
    fn format_uptime_hours_exact() {
        let dur = chrono::Duration::hours(3);
        assert_eq!(super::format_uptime(dur), "3h");
    }

    #[test]
    fn format_uptime_days_and_hours() {
        let dur = chrono::Duration::hours(25); // 1d 1h
        assert_eq!(super::format_uptime(dur), "1d 1h");
    }

    #[test]
    fn format_uptime_days_exact() {
        let dur = chrono::Duration::days(3);
        assert_eq!(super::format_uptime(dur), "3d");
    }
}
