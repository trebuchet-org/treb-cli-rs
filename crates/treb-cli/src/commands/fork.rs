//! `treb fork` subcommands — enter/exit fork mode, status, history, diff, revert, restart.

use std::{
    collections::BTreeSet,
    env,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, bail};
use chrono::Utc;
use clap::Subcommand;
use tokio::net::TcpStream;
use treb_core::types::fork::{ForkEntry, ForkHistoryEntry, SnapshotEntry};
use treb_forge::{
    anvil::{BackgroundAnvilConfig, find_available_port, poll_anvil_health, stop_background_anvil},
    createx::createx_deployed_bytecode,
};
use treb_registry::{
    DEPLOYMENTS_FILE, ForkStateStore, TRANSACTIONS_FILE, remove_snapshot, restore_registry,
    snapshot_registry,
};

const TREB_DIR: &str = ".treb";
const SNAPSHOT_BASE: &str = "priv/snapshots";

/// Format a chrono duration into a human-readable string matching the Go CLI.
fn format_duration(d: chrono::Duration) -> String {
    let secs = d.num_seconds().max(0);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}
const CREATEX_ADDRESS: &str = "0xba5Ed099633D3B313e4D5F7bdc1305d3c28ba5Ed";

// ── Subcommand enum ───────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum ForkSubcommand {
    /// Enter fork mode for a network: snapshot registry, start Anvil, record fork state
    ///
    /// Snapshots the current registry, spawns Anvil as a background subprocess
    /// forking from the upstream RPC, deploys CreateX, takes an initial EVM
    /// snapshot, and records a fully populated fork entry in `fork.json`.
    Enter {
        /// Network name (resolved from config if omitted)
        #[arg(long)]
        network: Option<String>,
        /// Upstream RPC URL to fork (overrides foundry.toml)
        #[arg(long)]
        rpc_url: Option<String>,
        /// Fork at a specific block number
        #[arg(long)]
        fork_block_number: Option<u64>,
    },
    /// Exit fork mode: restore registry from snapshot and remove fork state
    ///
    /// Restores the registry to the state it was in before `fork enter` and
    /// removes the fork entry and its snapshot directory.
    Exit {
        /// Network name (resolved from config if omitted)
        #[arg(long)]
        network: Option<String>,
    },
    /// Revert the fork to its last snapshot
    ///
    /// Restores the registry from the snapshot taken when fork mode was entered,
    /// discarding any deployments made during the fork session.
    Revert {
        /// Network name (resolved from config if omitted)
        #[arg(long)]
        network: Option<String>,
        /// Revert all active forks
        #[arg(long)]
        all: bool,
    },
    /// Restart the fork from a new block
    ///
    /// Resets the local Anvil node to a fresh fork at the given block number
    /// (or at the latest block if omitted) without exiting fork mode.
    Restart {
        /// Network name (resolved from config if omitted)
        #[arg(long)]
        network: Option<String>,
        /// Fork block number to reset to (uses latest if omitted)
        #[arg(long)]
        fork_block_number: Option<u64>,
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
        /// Filter by network name
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
        /// Network name (resolved from config if omitted)
        #[arg(long)]
        network: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

pub async fn run(subcommand: ForkSubcommand) -> anyhow::Result<()> {
    match subcommand {
        ForkSubcommand::Enter { network, rpc_url, fork_block_number } => {
            let network = require_network(network)?;
            run_enter(network, rpc_url, fork_block_number).await
        }
        ForkSubcommand::Exit { network } => {
            let network = require_network(network)?;
            run_exit(network).await
        }
        ForkSubcommand::Revert { network, all } => {
            let network = require_network(network)?;
            run_revert(network, all).await
        }
        ForkSubcommand::Restart { network, fork_block_number } => {
            let network = require_network(network)?;
            run_restart(network, fork_block_number).await
        }
        ForkSubcommand::Status { json } => run_status(json).await,
        ForkSubcommand::History { network, json } => run_history(network, json).await,
        ForkSubcommand::Diff { network, json } => {
            let network = require_network(network)?;
            run_diff(network, json).await
        }
    }
}

/// Resolve the network from the CLI flag or fall back to config/interactive picker.
fn require_network(cli_network: Option<String>) -> anyhow::Result<String> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let resolved = super::run::resolve_network(cli_network, &cwd, true, false)?;
    resolved.ok_or_else(|| anyhow::anyhow!("no network specified; pass --network or set one in config.local.json"))
}

// ── run_enter ─────────────────────────────────────────────────────────────────

/// Enter fork mode for a network.
///
/// 1. Validates the project is initialized and the network is not already forked.
/// 2. Resolves the upstream RPC URL and fetches the chain ID.
/// 3. Snapshots the registry.
/// 4. Finds an available port and spawns Anvil as a background subprocess.
/// 5. Polls until healthy, deploys CreateX, and takes an initial EVM snapshot.
/// 6. Records a fully populated [`ForkEntry`] in `fork.json`.
pub async fn run_enter(
    network: String,
    rpc_url_override: Option<String>,
    fork_block_number: Option<u64>,
) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let treb_dir = cwd.join(TREB_DIR);

    ensure_treb_dir(&treb_dir)?;

    // Load fork state and check not already forked (before any HTTP calls)
    let mut store = ForkStateStore::new(&treb_dir);
    store.load().context("failed to load fork state")?;

    if store.get_active_fork(&network).is_some() {
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

    // Find an available port
    let port = find_available_port().map_err(|e| anyhow::anyhow!("{e}"))?;

    // Set up .treb/priv/ directory for PID/log files (Go-compatible)
    let priv_dir = treb_dir.join("priv");
    std::fs::create_dir_all(&priv_dir).context("failed to create priv directory")?;

    let pid_file = priv_dir.join(format!("fork-{network}.pid"));
    let log_file = priv_dir.join(format!("fork-{network}.log"));

    // Spawn Anvil as a background subprocess
    let anvil_config = BackgroundAnvilConfig {
        port,
        chain_id: Some(chain_id),
        fork_url: Some(fork_url.clone()),
        fork_block_number,
        pid_file: pid_file.clone(),
        log_file: log_file.clone(),
    };

    let bg_anvil = treb_forge::anvil::spawn_background_anvil(&anvil_config)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let rpc_url = bg_anvil.rpc_url.clone();
    let anvil_pid = bg_anvil.pid as i32;

    // Poll until healthy
    let is_forked = anvil_config.fork_url.is_some();
    if let Err(e) = poll_anvil_health(&rpc_url, is_forked) {
        // Clean up on failure
        let _ = stop_background_anvil(&pid_file);
        bail!("failed to start fork anvil: {e}");
    }

    // Deploy CreateX factory via HTTP
    let client = reqwest::Client::builder().timeout(Duration::from_secs(10)).build()?;
    if let Err(e) = deploy_createx_http(&client, &rpc_url).await {
        eprintln!("Warning: failed to deploy CreateX: {e}");
    }

    // Take initial EVM snapshot
    let snapshot_id = match evm_snapshot_http(&client, &rpc_url).await {
        Ok(id) => id,
        Err(e) => {
            let _ = stop_background_anvil(&pid_file);
            bail!("failed to take initial EVM snapshot: {e}");
        }
    };

    // Build and insert fork entry with real values
    let now = Utc::now();
    let entry = ForkEntry {
        network: network.clone(),
        instance_name: None,
        rpc_url: rpc_url.clone(),
        port,
        chain_id,
        fork_url: fork_url.clone(),
        fork_block_number,
        snapshot_dir: snapshot_dir.to_string_lossy().into_owned(),
        started_at: now,
        env_var_name: format!("ETH_RPC_URL_{}", network.to_uppercase()),
        original_rpc: fork_url,
        anvil_pid,
        pid_file: pid_file.to_string_lossy().into_owned(),
        log_file: log_file.to_string_lossy().into_owned(),
        entered_at: now,
        snapshots: vec![SnapshotEntry {
            index: 0,
            snapshot_id,
            command: "fork enter".into(),
            timestamp: now,
        }],
    };
    store.insert_active_fork(entry).context("failed to record fork entry")?;

    // Add history entry
    store
        .add_history(ForkHistoryEntry {
            action: "enter".into(),
            network: network.clone(),
            timestamp: Utc::now(),
            details: None,
        })
        .context("failed to record fork history")?;

    // Ensure .treb/priv/ is in .gitignore
    ensure_gitignore_entry(&cwd, ".treb/priv/");

    println!("Fork mode entered for network '{network}'");
    println!();
    println!("  {:14}{}", "Network:", network);
    println!("  {:14}{}", "Chain ID:", chain_id);
    println!("  {:14}{}", "Fork URL:", rpc_url);
    println!("  {:14}{}", "Anvil PID:", anvil_pid);
    println!(
        "  {:14}{}={}",
        "Env Override:",
        format!("ETH_RPC_URL_{}", network.to_uppercase()),
        rpc_url,
    );
    println!("  {:14}{}", "Logs:", log_file.display());
    println!();
    println!("Run 'treb fork status' to check fork state");
    println!("Run 'treb fork exit' to stop fork and restore original state");

    Ok(())
}

// ── run_exit ──────────────────────────────────────────────────────────────────

/// Exit fork mode for a network.
///
/// Stops the background Anvil process, restores the registry from its snapshot,
/// removes the snapshot directory, and removes the [`ForkEntry`] from `fork.json`.
pub async fn run_exit(network: String) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let treb_dir = cwd.join(TREB_DIR);

    let mut store = ForkStateStore::new(&treb_dir);
    store.load().context("failed to load fork state")?;

    // Check if actively forked first (consistent error message with revert/restart/diff)
    if store.get_active_fork(&network).is_none() {
        bail!("network '{}' is not in fork mode", network);
    }

    // Remove active fork entry
    let entry = store.remove_active_fork(&network).context("failed to remove fork entry")?;

    // Stop the Anvil background process (safe to call if already dead)
    if !entry.pid_file.is_empty() {
        let pid_path = PathBuf::from(&entry.pid_file);
        if let Err(e) = stop_background_anvil(&pid_path) {
            eprintln!("Warning: failed to stop anvil for '{}': {}", network, e);
        }
    }

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
            network: network.clone(),
            timestamp: Utc::now(),
            details: None,
        })
        .context("failed to record exit history")?;

    println!("Exited fork mode for network '{network}'.");
    println!("Registry restored from snapshot.");

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
pub async fn run_revert(network: String, all: bool) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let treb_dir = cwd.join(TREB_DIR);

    let mut store = ForkStateStore::new(&treb_dir);
    store.load().context("failed to load fork state")?;

    let networks: Vec<String> = if all {
        store.list_active_forks().into_iter().map(|e| e.network.clone()).collect()
    } else {
        vec![network]
    };

    if networks.is_empty() {
        println!("No active forks to revert.");
        return Ok(());
    }

    let client = reqwest::Client::builder().timeout(Duration::from_secs(10)).build()?;

    for net in &networks {
        let entry = store
            .get_active_fork(net)
            .ok_or_else(|| anyhow::anyhow!("network '{}' is not in fork mode", net))?
            .clone();

        if !is_port_reachable(entry.port).await {
            bail!(
                "Anvil node for network '{}' is not reachable at port {}; cannot revert",
                net,
                entry.port
            );
        }

        // Revert EVM state to the last snapshot (if we have one).
        if let Some(last_snapshot) = entry.snapshots.last() {
            let reverted = evm_revert_http(&client, &entry.rpc_url, &last_snapshot.snapshot_id)
                .await
                .with_context(|| format!("failed to revert EVM state for network '{net}'"))?;
            if !reverted {
                bail!(
                    "EVM revert failed for network '{}' (snapshot ID: {})",
                    net,
                    last_snapshot.snapshot_id
                );
            }
            println!("EVM state reverted for network '{net}'.");
        } else {
            println!(
                "No EVM snapshots stored for network '{net}'; skipping EVM revert (registry will still be restored)."
            );
        }

        // Take a new EVM snapshot for the next revert.
        let new_snapshot_id = evm_snapshot_http(&client, &entry.rpc_url)
            .await
            .with_context(|| format!("failed to take new EVM snapshot for network '{net}'"))?;

        // Restore registry files from the snapshot directory.
        let snapshot_dir = PathBuf::from(&entry.snapshot_dir);
        restore_registry(&snapshot_dir, &treb_dir)
            .context("failed to restore registry from snapshot")?;

        // Update the fork entry with the new snapshot.
        let mut updated = entry.clone();
        let next_index = updated.snapshots.len() as u32;
        updated.snapshots.push(treb_core::types::fork::SnapshotEntry {
            index: next_index,
            snapshot_id: new_snapshot_id.clone(),
            command: "revert".into(),
            timestamp: Utc::now(),
        });
        store.update_active_fork(updated).context("failed to update fork entry")?;

        // Add history entry.
        store
            .add_history(ForkHistoryEntry {
                action: "revert".into(),
                network: net.clone(),
                timestamp: Utc::now(),
                details: Some(format!("new EVM snapshot: {new_snapshot_id}")),
            })
            .context("failed to record revert history")?;

        println!("Reverted fork for network '{net}' to initial state.");
    }

    Ok(())
}

// ── run_restart ───────────────────────────────────────────────────────────────

/// Restart a fork: kill old Anvil, restore registry, spawn fresh background Anvil.
///
/// Stops the existing Anvil process, restores registry files from the initial
/// snapshot, finds a new port, spawns a fresh background Anvil subprocess,
/// deploys CreateX, takes a new EVM snapshot, and updates the fork state.
pub async fn run_restart(network: String, fork_block_number: Option<u64>) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let treb_dir = cwd.join(TREB_DIR);

    let mut store = ForkStateStore::new(&treb_dir);
    store.load().context("failed to load fork state")?;

    let entry = store
        .get_active_fork(&network)
        .ok_or_else(|| anyhow::anyhow!("network '{}' is not in fork mode", network))?
        .clone();

    // Stop existing Anvil process (may already be dead)
    if !entry.pid_file.is_empty() {
        let pid_path = PathBuf::from(&entry.pid_file);
        let _ = stop_background_anvil(&pid_path);
    }

    // Restore registry from snapshot
    let snapshot_dir = PathBuf::from(&entry.snapshot_dir);
    restore_registry(&snapshot_dir, &treb_dir).context("failed to restore registry")?;

    // Find a new available port
    let port = find_available_port().map_err(|e| anyhow::anyhow!("{e}"))?;

    // Determine the block to fork from
    let blk = fork_block_number.or(entry.fork_block_number);

    // Set up PID/log file paths (reuse priv dir)
    let priv_dir = treb_dir.join("priv");
    std::fs::create_dir_all(&priv_dir).context("failed to create priv directory")?;
    let pid_file = priv_dir.join(format!("fork-{network}.pid"));
    let log_file = priv_dir.join(format!("fork-{network}.log"));

    // Spawn fresh Anvil as a background subprocess
    let anvil_config = BackgroundAnvilConfig {
        port,
        chain_id: Some(entry.chain_id),
        fork_url: Some(entry.original_rpc.clone()),
        fork_block_number: blk,
        pid_file: pid_file.clone(),
        log_file: log_file.clone(),
    };

    let bg_anvil = treb_forge::anvil::spawn_background_anvil(&anvil_config)
        .map_err(|e| anyhow::anyhow!("failed to start fresh fork anvil: {e}"))?;

    let rpc_url = bg_anvil.rpc_url.clone();
    let anvil_pid = bg_anvil.pid as i32;

    // Poll until healthy
    if let Err(e) = poll_anvil_health(&rpc_url, true) {
        let _ = stop_background_anvil(&pid_file);
        bail!("fresh fork anvil failed to become ready: {e}");
    }

    let client = reqwest::Client::builder().timeout(Duration::from_secs(10)).build()?;

    // Ensure CreateX factory exists (skips if already present on-chain)
    if let Err(e) = deploy_createx_http(&client, &rpc_url).await {
        eprintln!("Warning: failed to deploy CreateX: {e}");
    }

    println!(
        "Anvil reset to {} (block: {}).",
        entry.original_rpc,
        blk.map_or("latest".into(), |b| b.to_string())
    );

    // Take a new EVM snapshot as the fresh baseline
    let snapshot_id = evm_snapshot_http(&client, &rpc_url)
        .await
        .with_context(|| format!("failed to take EVM snapshot for network '{network}'"))?;

    // Update the fork entry
    let mut updated = entry.clone();
    updated.rpc_url = rpc_url;
    updated.port = port;
    updated.anvil_pid = anvil_pid;
    updated.pid_file = pid_file.to_string_lossy().into_owned();
    updated.log_file = log_file.to_string_lossy().into_owned();
    updated.entered_at = Utc::now();
    if let Some(b) = blk {
        updated.fork_block_number = Some(b);
    }
    updated.snapshots = vec![SnapshotEntry {
        index: 0,
        snapshot_id: snapshot_id.clone(),
        command: "fork restart".into(),
        timestamp: Utc::now(),
    }];
    store.update_active_fork(updated).context("failed to update fork entry")?;

    // Add history entry
    store
        .add_history(ForkHistoryEntry {
            action: "restart".into(),
            network: network.clone(),
            timestamp: Utc::now(),
            details: Some(format!("Anvil reset; snapshot: {snapshot_id}")),
        })
        .context("failed to record restart history")?;

    println!("Restarted fork for network '{network}' (port {port}, PID {anvil_pid}).");
    Ok(())
}

// ── run_status ────────────────────────────────────────────────────────────────

/// Display active fork status with live port reachability checks.
pub async fn run_status(json: bool) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let treb_dir = cwd.join(TREB_DIR);

    ensure_treb_dir(&treb_dir)?;

    let mut store = ForkStateStore::new(&treb_dir);
    store.load().context("failed to load fork state")?;

    let forks = store.list_active_forks();

    if json {
        let mut statuses = Vec::new();
        for entry in &forks {
            let running = is_port_reachable(entry.port).await;
            statuses.push(serde_json::json!({
                "network":         entry.network,
                "rpcUrl":          entry.rpc_url,
                "port":            entry.port,
                "chainId":         entry.chain_id,
                "forkBlockNumber": entry.fork_block_number,
                "startedAt":       entry.started_at,
                "status":          if running { "running" } else { "stopped" },
            }));
        }
        println!("{}", serde_json::to_string_pretty(&statuses)?);
        return Ok(());
    }

    if forks.is_empty() {
        println!("No active forks");
        return Ok(());
    }

    println!("Active Forks");

    for entry in &forks {
        let running = is_port_reachable(entry.port).await;
        let status = if running { "healthy" } else { "dead" };
        let uptime = format_duration(Utc::now() - entry.started_at);
        let snapshot_count = entry.snapshots.len();

        println!();
        println!("  {}", entry.network);
        println!("    {:14}{}", "Chain ID:", entry.chain_id);
        println!("    {:14}{}", "Fork URL:", entry.rpc_url);
        println!("    {:14}{}", "Anvil PID:", entry.anvil_pid);
        println!("    {:14}{}", "Status:", status);
        println!("    {:14}{}", "Uptime:", uptime);
        println!("    {:14}{}", "Snapshots:", snapshot_count);
        if !entry.log_file.is_empty() {
            println!("    {:14}{}", "Logs:", entry.log_file);
        }
    }

    println!();
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
        println!("{}", serde_json::to_string_pretty(&history)?);
        return Ok(());
    }

    if history.is_empty() {
        let filter_msg =
            network.as_deref().map_or_else(String::new, |n| format!(" for network '{n}'"));
        println!("No fork history{filter_msg}.");
        return Ok(());
    }

    let mut table = crate::output::build_table(&["Timestamp", "Action", "Network", "Details"]);

    for entry in &history {
        table.add_row(vec![
            &entry.timestamp.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            entry.action.as_str(),
            entry.network.as_str(),
            entry.details.as_deref().unwrap_or("-"),
        ]);
    }

    crate::output::print_table(&table);
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

    let entry = store
        .get_active_fork(&network)
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
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "network": network,
                "changes": changes,
                "clean":   changes.is_empty(),
            }))?
        );
        return Ok(());
    }

    if changes.is_empty() {
        println!("No changes detected for network '{network}'.");
        return Ok(());
    }

    let mut table = crate::output::build_table(&["Change", "File", "Key"]);
    for change in &changes {
        table.add_row(vec![
            change["change"].as_str().unwrap_or(""),
            change["file"].as_str().unwrap_or(""),
            change["key"].as_str().unwrap_or(""),
        ]);
    }
    crate::output::print_table(&table);
    Ok(())
}

// ── helpers ───────────────────────────────────────────────────────────────────

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
    // Use resolve_rpc_endpoints which loads .env and expands ${VAR} placeholders.
    let endpoints =
        treb_config::resolve_rpc_endpoints(cwd).map_err(|e| anyhow::anyhow!("{e}"))?;
    let ep = endpoints.get(network).ok_or_else(|| {
        anyhow::anyhow!(
            "no RPC URL configured for network '{}' in foundry.toml\n\n\
             Add it under [rpc_endpoints] or pass --rpc-url to specify directly.",
            network
        )
    })?;
    if ep.unresolved {
        bail!(
            "RPC URL for network '{}' has unresolved environment variables: {}\n\n\
             Check your .env file or set the variables in your environment.",
            network,
            ep.missing_vars.join(", ")
        );
    }
    Ok(ep.expanded_url.clone())
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

/// Deploy the CreateX factory bytecode at its canonical address via `anvil_setCode`.
async fn deploy_createx_http(client: &reqwest::Client, rpc_url: &str) -> anyhow::Result<()> {
    // Check if CreateX already exists (e.g., on a forked chain where it's
    // natively deployed). Skip deployment if code is already present.
    let code_resp = json_rpc_call(
        client,
        rpc_url,
        "eth_getCode",
        serde_json::json!([CREATEX_ADDRESS, "latest"]),
    )
    .await?;
    let existing_code = code_resp.as_str().unwrap_or("0x");
    if existing_code.len() > 2 {
        // CreateX already has code — don't overwrite it
        return Ok(());
    }

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

/// Add an entry to `.gitignore` if it is not already present (best-effort).
fn ensure_gitignore_entry(project_root: &Path, entry: &str) {
    let gitignore_path = project_root.join(".gitignore");

    let content = std::fs::read_to_string(&gitignore_path).unwrap_or_default();

    // Check if the entry is already present
    for line in content.lines() {
        if line.trim() == entry {
            return;
        }
    }

    // Append the entry
    let prefix = if content.is_empty() || content.ends_with('\n') { "" } else { "\n" };
    let to_append = format!("{prefix}{entry}\n");

    if let Err(e) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&gitignore_path)
        .and_then(|mut f| {
            use std::io::Write;
            f.write_all(to_append.as_bytes())
        })
    {
        eprintln!("Warning: failed to update .gitignore: {e}");
    }
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
            ForkSubcommand::Enter { network, rpc_url, fork_block_number } => {
                assert_eq!(network, "mainnet");
                assert!(rpc_url.is_none());
                assert!(fork_block_number.is_none());
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
        ])
        .unwrap();
        match sub {
            ForkSubcommand::Enter { network, rpc_url, fork_block_number } => {
                assert_eq!(network, "mainnet");
                assert_eq!(rpc_url.as_deref(), Some("https://eth.example.com"));
                assert_eq!(fork_block_number, Some(19_000_000));
            }
            _ => panic!("expected Enter"),
        }
    }

    #[test]
    fn parse_exit() {
        let sub = parse_fork(&["exit", "--network", "sepolia"]).unwrap();
        match sub {
            ForkSubcommand::Exit { network } => assert_eq!(network, "sepolia"),
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
            snapshot_dir: treb_dir.join("priv/snapshots").join(network).to_string_lossy().into_owned(),
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
        let snapshot_dir = treb_dir.join("priv/snapshots").join("mainnet");

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
        let snapshot_dir = treb_dir.join("priv/snapshots").join("mainnet");
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
        let snapshot_dir = treb_dir.join("priv/snapshots").join("mainnet");
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
}
