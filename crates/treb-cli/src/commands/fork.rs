//! `treb fork` subcommands — holistic fork mode with multi-network Anvil orchestration.
//!
//! Fork mode snapshots the registry once, then spawns background Anvil nodes for
//! ALL configured networks (or a subset via `--network`). Commands like `run` and
//! `compose` automatically detect active forks and route RPC traffic to the local
//! Anvil instances.

use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
    time::Duration,
};

use alloy_primitives::Address;
use alloy_provider::Provider;
use anyhow::{Context, bail};
use chrono::Utc;
use clap::Subcommand;
use tokio::net::TcpStream;
use treb_core::types::fork::{
    ForkEntry, ForkHistoryEntry, ForkRunSnapshot, ForkRunSource, SnapshotEntry,
};
use treb_forge::{
    anvil::{
        BackgroundAnvilConfig, deterministic_fork_port, find_available_port, is_port_available,
        poll_anvil_health, stop_background_anvil,
    },
    createx::createx_deployed_bytecode,
    provider::build_http_provider,
};
use treb_registry::{ForkStateStore, remove_snapshot, restore_registry, snapshot_registry};

use crate::commands::receipt;

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
    /// Enter fork mode: snapshot registry and start Anvil for all configured networks
    ///
    /// Snapshots the current registry once, then spawns a background Anvil
    /// subprocess for every network in `foundry.toml [rpc_endpoints]` (or a
    /// subset via `--network`). Each Anvil gets a deterministic port based on
    /// chain ID, deploys CreateX, and takes an initial EVM snapshot.
    Enter {
        /// Fork specific networks only (comma-separated or repeated)
        #[arg(long, value_delimiter = ',')]
        network: Vec<String>,
        /// Upstream RPC URL override (only valid with a single --network)
        #[arg(long)]
        rpc_url: Option<String>,
        /// Fork at a specific block number (applies to all networks)
        #[arg(long)]
        fork_block_number: Option<u64>,
    },
    /// Exit fork mode: stop all Anvils, restore registry from snapshot
    ///
    /// Stops every running Anvil process, restores the registry to the state
    /// it was in before `fork enter`, and clears all fork state.
    Exit,
    /// Revert all forks to their initial state
    ///
    /// Restores the registry from the holistic snapshot and reverts every
    /// running Anvil to its initial EVM snapshot, discarding any deployments
    /// made during the fork session.
    Revert,
    /// Restart the fork from a new block
    ///
    /// Resets the local Anvil node to a fresh fork at the given block number
    /// (or at the latest block if omitted) without exiting fork mode.
    Restart {
        /// Network name (required)
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
    /// Tail Anvil log files for active forks
    ///
    /// Shows logs from all active fork Anvil instances with colored network
    /// prefixes (foreman-style). Use `--follow` for continuous tailing.
    Logs {
        /// Continuously tail log files (Ctrl+C to exit)
        #[arg(long, short)]
        follow: bool,
        /// Filter to a specific network
        #[arg(long)]
        network: Option<String>,
    },
    /// Execute queued Safe/Governor items on the active fork
    ///
    /// Executes pending Safe transactions and governance proposals that were
    /// queued during a `treb run --broadcast` on the fork.
    Exec {
        /// Safe tx hash or proposal ID to execute (omit for --all)
        query: Option<String>,
        /// Execute all queued items
        #[arg(long)]
        all: bool,
        /// Network name or chain ID
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
            run_enter(network, rpc_url, fork_block_number).await
        }
        ForkSubcommand::Exit => run_exit().await,
        ForkSubcommand::Revert => run_revert().await,
        ForkSubcommand::Restart { network, fork_block_number } => {
            run_restart(network, fork_block_number).await
        }
        ForkSubcommand::Status { json } => run_status(json).await,
        ForkSubcommand::History { network, json } => run_history(network, json).await,
        ForkSubcommand::Logs { follow, network } => run_logs(follow, network).await,
        ForkSubcommand::Exec { query, all, network, json } => {
            run_exec(query, all, network, json).await
        }
    }
}

// ── run_enter ─────────────────────────────────────────────────────────────────

/// Enter holistic fork mode.
///
/// 1. Check not already in fork mode.
/// 2. Resolve networks from foundry.toml (or filter by `--network`).
/// 3. Snapshot registry ONCE.
/// 4. For each network: resolve RPC, fetch chain ID, spawn Anvil, deploy CreateX, take EVM
///    snapshot, record ForkEntry.
/// 5. Record history entry.
pub async fn run_enter(
    network_filter: Vec<String>,
    rpc_url_override: Option<String>,
    fork_block_number: Option<u64>,
) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let treb_dir = cwd.join(TREB_DIR);

    ensure_treb_dir(&treb_dir)?;

    // Validate --rpc-url only with a single network
    if rpc_url_override.is_some() && network_filter.len() != 1 {
        bail!("--rpc-url can only be used with a single --network");
    }

    // Load fork state and check not already in fork mode
    let mut store = ForkStateStore::new(&treb_dir);
    store.load().context("failed to load fork state")?;

    if store.is_fork_mode_active() {
        bail!(
            "already in fork mode (entered at {}); run `treb fork exit` first",
            store
                .data()
                .entered_at
                .map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| "unknown".into())
        );
    }

    // Resolve networks to fork
    let networks = resolve_fork_networks(&cwd, &network_filter)?;
    if networks.is_empty() {
        bail!(
            "no networks found in foundry.toml [rpc_endpoints]\n\n\
             Add RPC endpoints to your foundry.toml or pass --network explicitly."
        );
    }

    // Holistic snapshot — one snapshot for all networks
    let snapshot_dir = treb_dir.join(SNAPSHOT_BASE).join("holistic");
    snapshot_registry(&treb_dir, &snapshot_dir).context("failed to snapshot registry")?;

    // Enter fork mode
    store.enter_fork_mode(&snapshot_dir.to_string_lossy()).context("failed to enter fork mode")?;

    // Set up priv dir
    let priv_dir = treb_dir.join("priv");
    std::fs::create_dir_all(&priv_dir).context("failed to create priv directory")?;

    // Resolve fork URLs synchronously (needs filesystem access for foundry.toml)
    let mut fork_configs: Vec<(String, String)> = Vec::new();
    let mut errors: Vec<(String, String)> = Vec::new();
    for (i, network_name) in networks.iter().enumerate() {
        let rpc_override = if i == 0 { rpc_url_override.clone() } else { None };
        match resolve_fork_url(&cwd, network_name, rpc_override) {
            Ok(url) => fork_configs.push((network_name.clone(), url)),
            Err(e) => errors.push((network_name.clone(), format!("{e}"))),
        }
    }

    // Fetch chain IDs in parallel
    let mut chain_id_set = tokio::task::JoinSet::new();
    for (name, url) in fork_configs {
        chain_id_set.spawn(async move {
            match fetch_chain_id(&url).await {
                Ok(id) => Ok((name, url, id)),
                Err(e) => Err((name, format!("failed to fetch chain ID: {e}"))),
            }
        });
    }
    let mut chain_id_results = Vec::new();
    while let Some(result) = chain_id_set.join_next().await {
        match result {
            Ok(r) => chain_id_results.push(r),
            Err(e) => errors.push(("unknown".into(), format!("task panicked: {e}"))),
        }
    }

    // Allocate ports sequentially (deterministic port per chain, fallback needs exclusion)
    struct ForkSetup {
        network: String,
        fork_url: String,
        chain_id: u64,
        port: u16,
        pid_file: PathBuf,
        log_file: PathBuf,
    }
    let mut setups: Vec<ForkSetup> = Vec::new();
    for result in chain_id_results {
        match result {
            Ok((name, url, chain_id)) => {
                let det_port = deterministic_fork_port(chain_id);
                let port = if is_port_available(det_port) {
                    det_port
                } else {
                    match find_available_port() {
                        Ok(p) => p,
                        Err(e) => {
                            errors.push((name, format!("no available port: {e}")));
                            continue;
                        }
                    }
                };
                let pid_file = priv_dir.join(format!("fork-{name}.pid"));
                let log_file = priv_dir.join(format!("fork-{name}.log"));
                setups.push(ForkSetup {
                    network: name,
                    fork_url: url,
                    chain_id,
                    port,
                    pid_file,
                    log_file,
                });
            }
            Err((name, msg)) => errors.push((name, msg)),
        }
    }

    // Spawn all Anvil nodes and run post-spawn setup in parallel
    let spinner = crate::ui::spinner::create_spinner("Starting forks...");
    let mut spawn_set = tokio::task::JoinSet::new();
    for setup in setups {
        spawn_set.spawn(async move {
            // Spawn Anvil (sync — runs quickly)
            let anvil_config = BackgroundAnvilConfig {
                port: setup.port,
                chain_id: Some(setup.chain_id),
                fork_url: Some(setup.fork_url.clone()),
                fork_block_number,
                pid_file: setup.pid_file.clone(),
                log_file: setup.log_file.clone(),
            };

            let bg_anvil = treb_forge::anvil::spawn_background_anvil(&anvil_config)
                .map_err(|e| (setup.network.clone(), format!("failed to spawn anvil: {e}")))?;

            let rpc_url = bg_anvil.rpc_url.clone();
            let pid_file = setup.pid_file.clone();

            // Poll until healthy (blocking — run in spawn_blocking)
            let health_url = rpc_url.clone();
            let health_pid = pid_file.clone();
            let is_forked = anvil_config.fork_url.is_some();
            let health_result =
                tokio::task::spawn_blocking(move || poll_anvil_health(&health_url, is_forked))
                    .await
                    .map_err(|e| {
                        let _ = stop_background_anvil(&health_pid);
                        (setup.network.clone(), format!("health poll task failed: {e}"))
                    })?;

            if let Err(e) = health_result {
                let _ = stop_background_anvil(&pid_file);
                return Err((setup.network.clone(), format!("anvil not ready: {e}")));
            }

            // Deploy CreateX
            if let Err(e) = deploy_createx_http(&rpc_url).await {
                eprintln!("Warning: failed to deploy CreateX on {}: {e}", setup.network);
            }

            // Take initial EVM snapshot
            let snapshot_id = evm_snapshot_http(&rpc_url).await.map_err(|e| {
                let _ = stop_background_anvil(&pid_file);
                (setup.network.clone(), format!("failed to take EVM snapshot: {e}"))
            })?;

            Ok((setup, bg_anvil, rpc_url, snapshot_id))
        });
    }

    let mut spawn_results = Vec::new();
    while let Some(result) = spawn_set.join_next().await {
        match result {
            Ok(r) => spawn_results.push(r),
            Err(e) => errors.push(("unknown".into(), format!("task panicked: {e}"))),
        }
    }

    spinner.finish_and_clear();

    // Write results to store sequentially
    let mut started = Vec::new();
    for result in spawn_results {
        match result {
            Ok((setup, bg_anvil, rpc_url, snapshot_id)) => {
                let now = Utc::now();
                let anvil_pid = bg_anvil.pid as i32;
                let entry = ForkEntry {
                    network: setup.network.clone(),
                    instance_name: None,
                    rpc_url: rpc_url.clone(),
                    port: setup.port,
                    chain_id: setup.chain_id,
                    fork_url: setup.fork_url.clone(),
                    fork_block_number,
                    snapshot_dir: snapshot_dir.to_string_lossy().into_owned(),
                    started_at: now,
                    env_var_name: format!("ETH_RPC_URL_{}", setup.network.to_uppercase()),
                    original_rpc: setup.fork_url,
                    anvil_pid,
                    pid_file: setup.pid_file.to_string_lossy().into_owned(),
                    log_file: setup.log_file.to_string_lossy().into_owned(),
                    entered_at: now,
                    snapshots: vec![SnapshotEntry {
                        index: 0,
                        snapshot_id,
                        command: "fork enter".into(),
                        timestamp: now,
                    }],
                };
                store.insert_active_fork(entry).context("failed to record fork entry")?;
                started.push((setup.network, setup.chain_id, setup.port, rpc_url, anvil_pid));
            }
            Err((name, msg)) => errors.push((name, msg)),
        }
    }

    // Record history
    let network_list: String =
        started.iter().map(|(n, ..)| n.as_str()).collect::<Vec<_>>().join(", ");
    store
        .add_history(ForkHistoryEntry {
            action: "enter".into(),
            network: network_list.clone(),
            timestamp: Utc::now(),
            details: Some(format!("{} networks", started.len())),
        })
        .context("failed to record fork history")?;

    // Ensure .treb/priv/ is in .gitignore
    ensure_gitignore_entry(&cwd, ".treb/priv/");

    // Print summary
    if started.is_empty() && !errors.is_empty() {
        // All networks failed — exit fork mode
        let _ = restore_registry(&snapshot_dir, &treb_dir);
        let _ = remove_snapshot(&snapshot_dir);
        store.exit_fork_mode().ok();
        bail!(
            "failed to start any forks:\n{}",
            errors.iter().map(|(n, e)| format!("  {n}: {e}")).collect::<Vec<_>>().join("\n")
        );
    }

    println!("Fork mode entered ({} networks)", started.len());
    println!();

    for (net, chain_id, port, rpc_url, pid) in &started {
        println!("  {net}");
        println!("    {:14}{}", "Chain ID:", chain_id);
        println!("    {:14}{}", "Port:", port);
        println!("    {:14}{}", "RPC URL:", rpc_url);
        println!("    {:14}{}", "Anvil PID:", pid);
    }

    if !errors.is_empty() {
        println!();
        eprintln!("Failed to start:");
        for (net, err) in &errors {
            eprintln!("  {net}: {err}");
        }
    }

    println!();
    println!("Run 'treb fork status' to check fork state");
    println!("Run 'treb fork logs -f' to tail all Anvil logs");
    println!("Run 'treb fork exit' to stop all forks and restore original state");

    Ok(())
}

// ── snapshot_fork_before_broadcast ─────────────────────────────────────────────

/// Take a pre-broadcast snapshot of the registry and EVM state for each active fork.
///
/// Called from `run.rs` and `compose.rs` after user confirms broadcast but before
/// `broadcast_all()`. Returns the snapshot that was pushed onto the store stack.
pub async fn snapshot_fork_before_broadcast(
    treb_dir: &Path,
    store: &mut ForkStateStore,
    source: ForkRunSource,
) -> anyhow::Result<ForkRunSnapshot> {
    let index = store.next_run_snapshot_index();
    let snapshot_dir = treb_dir.join(SNAPSHOT_BASE).join(format!("run-{index}"));

    // 1. Copy current registry JSON
    snapshot_registry(treb_dir, &snapshot_dir)
        .context("failed to snapshot registry before broadcast")?;

    // 2. Take EVM snapshot for each active fork
    let mut evm_snapshots: HashMap<String, String> = HashMap::new();

    let forks: Vec<(String, ForkEntry)> = store
        .list_active_forks()
        .into_iter()
        .map(|e| {
            let key = match &e.instance_name {
                Some(name) if name != &e.network => format!("{}:{}", e.network, name),
                _ => e.network.clone(),
            };
            (key, e.clone())
        })
        .collect();

    for (key, entry) in &forks {
        if is_port_reachable(entry.port).await {
            match evm_snapshot_http(&entry.rpc_url).await {
                Ok(snap_id) => {
                    evm_snapshots.insert(key.clone(), snap_id);
                }
                Err(e) => {
                    eprintln!("Warning: failed to take EVM snapshot for '{}': {e}", entry.network);
                }
            }
        }
    }

    // 3. Build and push the snapshot (counts updated after broadcast)
    let snapshot = ForkRunSnapshot {
        index,
        source,
        registry_snapshot_dir: snapshot_dir.to_string_lossy().into_owned(),
        evm_snapshots,
        deployment_count: 0,
        transaction_count: 0,
        timestamp: Utc::now(),
    };

    store.push_run_snapshot(snapshot.clone())?;
    Ok(snapshot)
}

// ── run_exit ──────────────────────────────────────────────────────────────────

/// Exit holistic fork mode.
///
/// Stops all Anvil processes, restores the registry from the holistic snapshot,
/// removes the snapshot, and clears fork state.
pub async fn run_exit() -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let treb_dir = cwd.join(TREB_DIR);

    let mut store = ForkStateStore::new(&treb_dir);
    store.load().context("failed to load fork state")?;

    if !store.is_fork_mode_active() {
        bail!("not in fork mode");
    }

    // Stop all Anvil processes
    let forks: Vec<ForkEntry> = store.list_active_forks().into_iter().cloned().collect();
    for entry in &forks {
        if !entry.pid_file.is_empty() {
            let pid_path = PathBuf::from(&entry.pid_file);
            if let Err(e) = stop_background_anvil(&pid_path) {
                eprintln!("Warning: failed to stop anvil for '{}': {}", entry.network, e);
            }
        }
    }

    // Restore registry from holistic snapshot
    let snapshot_dir = store
        .data()
        .snapshot_dir
        .as_ref()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("fork state has no snapshot directory"))?;

    restore_registry(&snapshot_dir, &treb_dir)
        .context("failed to restore registry from snapshot")?;

    // Clean up per-run snapshot dirs
    let cleared = store.clear_run_snapshots().unwrap_or_default();
    for snap in &cleared {
        let snap_path = PathBuf::from(&snap.registry_snapshot_dir);
        let _ = remove_snapshot(&snap_path);
    }

    // Remove holistic snapshot dir
    remove_snapshot(&snapshot_dir).context("failed to remove snapshot directory")?;

    // Record history
    let network_list: String =
        forks.iter().map(|e| e.network.as_str()).collect::<Vec<_>>().join(", ");
    store
        .add_history(ForkHistoryEntry {
            action: "exit".into(),
            network: network_list,
            timestamp: Utc::now(),
            details: Some(format!("{} networks stopped", forks.len())),
        })
        .context("failed to record exit history")?;

    // Clear fork mode
    store.exit_fork_mode().context("failed to clear fork state")?;

    println!("Exited fork mode. {} Anvil instances stopped, registry restored.", forks.len());

    Ok(())
}

// ── run_revert ────────────────────────────────────────────────────────────────

/// Revert the last run broadcast in fork mode.
///
/// Pops the most recent `ForkRunSnapshot` from the stack, reverts each
/// per-network EVM snapshot, restores the registry from the snapshot dir,
/// and cleans up the snapshot directory.
pub async fn run_revert() -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let treb_dir = cwd.join(TREB_DIR);

    let mut store = ForkStateStore::new(&treb_dir);
    store.load().context("failed to load fork state")?;

    if !store.is_fork_mode_active() {
        bail!("not in fork mode");
    }

    let snapshot =
        store.pop_run_snapshot()?.ok_or_else(|| anyhow::anyhow!("no run snapshots to revert"))?;

    // Revert EVM state for each fork that had a snapshot
    for (fork_key, snap_id) in &snapshot.evm_snapshots {
        if let Some(entry) = store.get_active_fork(fork_key) {
            if !is_port_reachable(entry.port).await {
                eprintln!(
                    "Warning: Anvil for '{fork_key}' not reachable at port {}; skipping EVM revert",
                    entry.port
                );
                continue;
            }
            let reverted = evm_revert_http(&entry.rpc_url, snap_id)
                .await
                .with_context(|| format!("failed to revert EVM state for '{fork_key}'"))?;
            if !reverted {
                eprintln!("Warning: EVM revert failed for '{fork_key}' (snapshot: {snap_id})");
            }
            // Re-snapshot for idempotency (evm_revert consumes the snapshot)
            let new_snap_id = evm_snapshot_http(&entry.rpc_url)
                .await
                .with_context(|| format!("failed to re-snapshot EVM state for '{fork_key}'"))?;
            // Update the fork entry's snapshots
            let mut updated = entry.clone();
            let next_index = updated.snapshots.len() as u32;
            updated.snapshots.push(SnapshotEntry {
                index: next_index,
                snapshot_id: new_snap_id,
                command: "revert".into(),
                timestamp: Utc::now(),
            });
            store.update_active_fork(updated).context("failed to update fork entry")?;
        }
    }

    // Restore registry from the run snapshot dir
    let snapshot_dir = PathBuf::from(&snapshot.registry_snapshot_dir);
    restore_registry(&snapshot_dir, &treb_dir)
        .context("failed to restore registry from run snapshot")?;

    // Clean up the run snapshot dir
    let _ = remove_snapshot(&snapshot_dir);

    // Build a human-readable source label
    let source_label = format_run_source(&snapshot.source);
    let remaining = store.run_snapshots().len();

    // Record history
    store
        .add_history(ForkHistoryEntry {
            action: "revert".into(),
            network: "all".into(),
            timestamp: Utc::now(),
            details: Some(format!("reverted: {source_label}")),
        })
        .context("failed to record revert history")?;

    println!("Reverted: {source_label}");
    let dep_count = snapshot.deployment_count;
    let tx_count = snapshot.transaction_count;
    println!("  {} deployment(s), {} transaction(s) undone", dep_count, tx_count);
    println!("  {} snapshot(s) remaining", remaining);
    Ok(())
}

// ── run_restart ───────────────────────────────────────────────────────────────

/// Restart forks: kill old Anvil(s), restore registry, spawn fresh background Anvil(s).
///
/// If `network` is `None`, restarts all active forks in parallel.
/// If `Some(network)`, restarts just that single network.
pub async fn run_restart(
    network: Option<String>,
    fork_block_number: Option<u64>,
) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let treb_dir = cwd.join(TREB_DIR);

    let mut store = ForkStateStore::new(&treb_dir);
    store.load().context("failed to load fork state")?;

    if !store.is_fork_mode_active() {
        bail!("not in fork mode");
    }

    // Determine which forks to restart
    let entries: Vec<ForkEntry> = if let Some(ref net) = network {
        let entry = store
            .get_active_fork(net)
            .ok_or_else(|| anyhow::anyhow!("network '{}' is not in fork mode", net))?
            .clone();
        vec![entry]
    } else {
        store.list_active_forks().into_iter().cloned().collect()
    };

    if entries.is_empty() {
        bail!("no active forks to restart");
    }

    // Stop all targeted Anvil processes
    for entry in &entries {
        if !entry.pid_file.is_empty() {
            let _ = stop_background_anvil(&PathBuf::from(&entry.pid_file));
        }
    }

    // Restore registry from holistic snapshot
    if let Some(ref snap_dir) = store.data().snapshot_dir {
        let snapshot_dir = PathBuf::from(snap_dir);
        restore_registry(&snapshot_dir, &treb_dir).context("failed to restore registry")?;
    }

    // Clear run snapshots (restart = clean slate)
    let cleared = store.clear_run_snapshots().unwrap_or_default();
    for snap in &cleared {
        let _ = remove_snapshot(&PathBuf::from(&snap.registry_snapshot_dir));
    }

    let priv_dir = treb_dir.join("priv");
    std::fs::create_dir_all(&priv_dir).context("failed to create priv directory")?;

    let spinner_msg = if entries.len() == 1 { "Restarting fork..." } else { "Restarting forks..." };
    let spinner = crate::ui::spinner::create_spinner(spinner_msg.to_string());

    // Spawn all Anvil nodes in parallel
    let mut spawn_set = tokio::task::JoinSet::new();
    for entry in entries {
        let priv_dir = priv_dir.clone();
        let blk = fork_block_number.or(entry.fork_block_number);
        spawn_set.spawn(async move {
            let network = entry.network.clone();
            let det_port = deterministic_fork_port(entry.chain_id);
            let port = if is_port_available(det_port) {
                det_port
            } else {
                find_available_port().map_err(|e| (network.clone(), format!("{e}")))?
            };

            let pid_file = priv_dir.join(format!("fork-{network}.pid"));
            let log_file = priv_dir.join(format!("fork-{network}.log"));

            let anvil_config = BackgroundAnvilConfig {
                port,
                chain_id: Some(entry.chain_id),
                fork_url: Some(entry.original_rpc.clone()),
                fork_block_number: blk,
                pid_file: pid_file.clone(),
                log_file: log_file.clone(),
            };

            let bg_anvil = treb_forge::anvil::spawn_background_anvil(&anvil_config)
                .map_err(|e| (network.clone(), format!("failed to spawn anvil: {e}")))?;

            let rpc_url = bg_anvil.rpc_url.clone();
            let anvil_pid = bg_anvil.pid as i32;

            // Poll until healthy
            let health_url = rpc_url.clone();
            let health_pid = pid_file.clone();
            let health_result =
                tokio::task::spawn_blocking(move || poll_anvil_health(&health_url, true))
                    .await
                    .map_err(|e| {
                        let _ = stop_background_anvil(&health_pid);
                        (network.clone(), format!("health poll task failed: {e}"))
                    })?;

            if let Err(e) = health_result {
                let _ = stop_background_anvil(&pid_file);
                return Err((network.clone(), format!("anvil not ready: {e}")));
            }

            if let Err(e) = deploy_createx_http(&rpc_url).await {
                eprintln!("Warning: failed to deploy CreateX on {network}: {e}");
            }

            let snapshot_id = evm_snapshot_http(&rpc_url)
                .await
                .map_err(|e| (network.clone(), format!("EVM snapshot failed: {e}")))?;

            Ok((entry, port, rpc_url, anvil_pid, pid_file, log_file, snapshot_id, blk))
        });
    }

    let mut results = Vec::new();
    let mut errors = Vec::new();
    while let Some(result) = spawn_set.join_next().await {
        match result {
            Ok(Ok(r)) => results.push(r),
            Ok(Err((net, msg))) => errors.push((net, msg)),
            Err(e) => errors.push(("unknown".into(), format!("task panicked: {e}"))),
        }
    }

    spinner.finish_and_clear();

    for (entry, port, rpc_url, anvil_pid, pid_file, log_file, snapshot_id, blk) in &results {
        let mut updated = entry.clone();
        updated.rpc_url = rpc_url.clone();
        updated.port = *port;
        updated.anvil_pid = *anvil_pid;
        updated.pid_file = pid_file.to_string_lossy().into_owned();
        updated.log_file = log_file.to_string_lossy().into_owned();
        updated.entered_at = Utc::now();
        if let Some(b) = blk {
            updated.fork_block_number = Some(*b);
        }
        updated.snapshots = vec![SnapshotEntry {
            index: 0,
            snapshot_id: snapshot_id.clone(),
            command: "fork restart".into(),
            timestamp: Utc::now(),
        }];
        store.update_active_fork(updated).context("failed to update fork entry")?;
    }

    // History
    let network_list: String =
        results.iter().map(|(e, ..)| e.network.as_str()).collect::<Vec<_>>().join(", ");
    store
        .add_history(ForkHistoryEntry {
            action: "restart".into(),
            network: network_list.clone(),
            timestamp: Utc::now(),
            details: Some(format!("{} networks restarted", results.len())),
        })
        .context("failed to record restart history")?;

    // Print summary
    for (entry, port, _rpc_url, anvil_pid, ..) in &results {
        println!("Restarted fork for '{}' (port {port}, PID {anvil_pid}).", entry.network);
    }
    if !errors.is_empty() {
        eprintln!();
        for (net, err) in &errors {
            eprintln!("Failed to restart '{net}': {err}");
        }
    }

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
        let output = serde_json::json!({
            "active": store.is_fork_mode_active(),
            "enteredAt": store.data().entered_at,
            "forks": statuses,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    if !store.is_fork_mode_active() {
        println!("Not in fork mode.");
        return Ok(());
    }

    // Show holistic header
    if let Some(entered_at) = store.data().entered_at {
        let uptime = format_duration(Utc::now() - entered_at);
        println!(
            "Fork mode active since {} ({} ago)",
            entered_at.format("%Y-%m-%d %H:%M:%S UTC"),
            uptime
        );
    } else {
        println!("Fork mode active");
    }

    if forks.is_empty() {
        println!("\nNo active forks.");
        return Ok(());
    }

    for entry in &forks {
        let running = is_port_reachable(entry.port).await;
        let status = if running { "healthy" } else { "dead" };
        let uptime = format_duration(Utc::now() - entry.started_at);
        let snapshot_count = entry.snapshots.len();

        println!();
        println!("  {}", entry.network);
        println!("    {:14}{}", "Chain ID:", entry.chain_id);
        println!("    {:14}{}", "RPC URL:", entry.rpc_url);
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

/// Display fork history as a snapshot stack.
///
/// Shows the run snapshot stack with the current state marked by `→`.
/// Index 0 is always "initial" (the fork enter point). Each entry above 0
/// is a run snapshot.
pub async fn run_history(network: Option<String>, json: bool) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let treb_dir = cwd.join(TREB_DIR);
    ensure_treb_dir(&treb_dir)?;

    let mut store = ForkStateStore::new(&treb_dir);
    store.load().context("failed to load fork state")?;

    if json {
        let data = store.data();
        let history: Vec<_> = if let Some(ref net) = network {
            data.history
                .iter()
                .filter(|e| e.network.split(", ").any(|n| n == net.as_str()))
                .cloned()
                .collect()
        } else {
            data.history.clone()
        };
        let output = serde_json::json!({
            "active": data.active,
            "enteredAt": data.entered_at,
            "runSnapshots": data.run_snapshots,
            "history": history,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    if !store.is_fork_mode_active() {
        println!("Not in fork mode.");
        return Ok(());
    }

    let data = store.data();
    let snapshots = &data.run_snapshots;

    // Determine the network list for the header
    let networks: Vec<String> = store.list_active_networks();
    let header = if let Some(ref net) = network {
        format!("Fork History: {net}")
    } else {
        format!("Fork History: {}", networks.join(", "))
    };
    println!("{header}");

    // Current state is the last snapshot (or initial if empty)
    let total = snapshots.len();

    // Print snapshots in reverse order (most recent first)
    for (i, snap) in snapshots.iter().enumerate().rev() {
        let marker = if i == total - 1 { "→" } else { " " };
        let source_label = format_run_source(&snap.source);
        let ts = snap.timestamp.format("%Y-%m-%d %H:%M:%S");
        println!("  {marker} [{}] {source_label}  ({ts})", snap.index);
    }

    // Always show index 0 as "initial"
    let marker = if total == 0 { "→" } else { " " };
    let initial_ts = data
        .entered_at
        .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "unknown".into());
    println!("  {marker} [0] initial  ({initial_ts})");

    Ok(())
}

// ── run_diff ──────────────────────────────────────────────────────────────────

// ── run_logs ──────────────────────────────────────────────────────────────────

/// Color palette for foreman-style log output (rotating by sorted network index).
const LOG_COLORS: &[&str] = &[
    "\x1b[36m", // cyan
    "\x1b[32m", // green
    "\x1b[33m", // yellow
    "\x1b[35m", // magenta
    "\x1b[34m", // blue
    "\x1b[31m", // red
];
const RESET: &str = "\x1b[0m";

/// Tail Anvil log files for active forks with colored network prefixes.
pub async fn run_logs(follow: bool, network_filter: Option<String>) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let treb_dir = cwd.join(TREB_DIR);
    ensure_treb_dir(&treb_dir)?;

    let mut store = ForkStateStore::new(&treb_dir);
    store.load().context("failed to load fork state")?;

    if !store.is_fork_mode_active() {
        bail!("not in fork mode");
    }

    // Collect forks, optionally filtered
    let mut forks: Vec<ForkEntry> = store
        .list_active_forks()
        .into_iter()
        .filter(|e| network_filter.as_deref().is_none_or(|n| e.network == n))
        .cloned()
        .collect();

    if forks.is_empty() {
        if let Some(ref net) = network_filter {
            bail!("no active fork for network '{net}'");
        }
        bail!("no active forks");
    }

    // Sort by network name for deterministic color assignment
    forks.sort_by(|a, b| a.network.cmp(&b.network));

    // Compute max network name length for aligned padding
    let max_name_len = forks.iter().map(|e| e.network.len()).max().unwrap_or(0);

    // Check NO_COLOR
    let no_color =
        std::env::var("NO_COLOR").is_ok() || std::env::var("TERM").ok().as_deref() == Some("dumb");

    if follow {
        run_logs_follow(&forks, max_name_len, no_color).await
    } else {
        run_logs_static(&forks, max_name_len, no_color)
    }
}

/// Print all existing log lines from each fork's log file (non-follow mode).
fn run_logs_static(forks: &[ForkEntry], max_name_len: usize, no_color: bool) -> anyhow::Result<()> {
    for (i, entry) in forks.iter().enumerate() {
        if entry.log_file.is_empty() {
            continue;
        }
        let path = PathBuf::from(&entry.log_file);
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                eprintln!("Warning: failed to read log for '{}': {e}", entry.network);
                continue;
            }
        };

        let color = if no_color { "" } else { LOG_COLORS[i % LOG_COLORS.len()] };
        let reset = if no_color { "" } else { RESET };

        for line in content.lines() {
            println!("{color}{:>width$}{reset} | {line}", entry.network, width = max_name_len);
        }
    }
    Ok(())
}

/// Continuously tail all log files with colored prefixes (follow mode).
///
/// Detects file rotation (e.g. after `fork exit` + `fork enter`) by
/// tracking how many bytes we have read and comparing against the current
/// file length on disk.  `spawn_background_anvil` truncates the log file
/// in-place (`File::create`), so the inode stays the same but the file
/// becomes shorter than our read cursor — we detect that and re-open.
async fn run_logs_follow(
    forks: &[ForkEntry],
    max_name_len: usize,
    no_color: bool,
) -> anyhow::Result<()> {
    use tokio::{
        io::{AsyncBufReadExt, BufReader},
        sync::mpsc,
    };

    let (tx, mut rx) = mpsc::channel::<(usize, String)>(256);

    // Spawn a tail task per log file
    for (i, entry) in forks.iter().enumerate() {
        if entry.log_file.is_empty() {
            continue;
        }
        let log_path = PathBuf::from(&entry.log_file);
        let tx = tx.clone();

        tokio::spawn(async move {
            let mut eof_count: u32 = 0;

            loop {
                // Wait for the file to exist
                loop {
                    if log_path.exists() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }

                let file = match tokio::fs::File::open(&log_path).await {
                    Ok(f) => f,
                    Err(_) => {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        continue;
                    }
                };
                let mut reader = BufReader::new(file);
                let mut line = String::new();
                let mut bytes_read: u64 = 0;

                loop {
                    line.clear();
                    match reader.read_line(&mut line).await {
                        Ok(0) => {
                            eof_count += 1;
                            tokio::time::sleep(Duration::from_millis(100)).await;

                            // Every ~1s of sustained EOF, check whether the
                            // file was truncated/replaced or deleted.
                            if eof_count >= 10 {
                                eof_count = 0;

                                match std::fs::metadata(&log_path) {
                                    Ok(meta) => {
                                        // File was truncated (same inode, shorter
                                        // than our cursor) — re-open from start.
                                        if meta.len() < bytes_read {
                                            break;
                                        }
                                    }
                                    Err(_) => {
                                        // File deleted — wait for recreation
                                        break;
                                    }
                                }
                            }
                        }
                        Ok(n) => {
                            eof_count = 0;
                            bytes_read += n as u64;
                            let trimmed = line.trim_end().to_string();
                            if tx.send((i, trimmed)).await.is_err() {
                                return;
                            }
                        }
                        Err(_) => {
                            // Read error — try to re-open
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            break;
                        }
                    }
                }
            }
        });
    }

    // Drop the sender so rx closes when all tasks are done
    drop(tx);

    // Print incoming lines with colored prefixes
    let network_names: Vec<&str> = forks.iter().map(|e| e.network.as_str()).collect();

    while let Some((idx, line)) = rx.recv().await {
        let name = network_names.get(idx).unwrap_or(&"unknown");
        let color = if no_color { "" } else { LOG_COLORS[idx % LOG_COLORS.len()] };
        let reset = if no_color { "" } else { RESET };
        println!("{color}{:>width$}{reset} | {line}", name, width = max_name_len);
    }

    Ok(())
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Format a `ForkRunSource` into a human-readable label.
fn format_run_source(source: &ForkRunSource) -> String {
    match source {
        ForkRunSource::Run { script } => format!("run {script}"),
        ForkRunSource::Compose { file, group, .. } => format!("compose {file} ({group})"),
    }
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

/// Resolve which networks to fork.
///
/// If `filter` is non-empty, uses those network names directly.
/// Otherwise, resolves ALL networks from `foundry.toml [rpc_endpoints]`.
fn resolve_fork_networks(cwd: &Path, filter: &[String]) -> anyhow::Result<Vec<String>> {
    if !filter.is_empty() {
        return Ok(filter.to_vec());
    }

    let endpoints = treb_config::resolve_rpc_endpoints(cwd).map_err(|e| anyhow::anyhow!("{e}"))?;

    // Filter out unresolved endpoints (missing env vars)
    let mut networks: Vec<String> =
        endpoints.into_iter().filter(|(_, ep)| !ep.unresolved).map(|(name, _)| name).collect();

    networks.sort();
    Ok(networks)
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
    let endpoints = treb_config::resolve_rpc_endpoints(cwd).map_err(|e| anyhow::anyhow!("{e}"))?;
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
    let provider = build_http_provider(rpc_url)
        .map_err(|e| anyhow::anyhow!("failed to build provider: {e}"))?;

    let chain_id = tokio::time::timeout(Duration::from_secs(10), provider.get_chain_id())
        .await
        .with_context(|| format!("timed out connecting to {rpc_url}"))?
        .with_context(|| format!("failed to fetch chain ID from {rpc_url}"))?;

    Ok(chain_id)
}

/// Take an EVM snapshot and return the snapshot ID as a hex string.
pub(crate) async fn evm_snapshot_http(rpc_url: &str) -> anyhow::Result<String> {
    let provider = build_http_provider(rpc_url)
        .map_err(|e| anyhow::anyhow!("failed to build provider: {e}"))?;
    let result: serde_json::Value = provider
        .raw_request("evm_snapshot".into(), ())
        .await
        .context("evm_snapshot RPC call failed")?;
    result
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("unexpected evm_snapshot result type: {result}"))
}

/// Revert EVM state to a previously created snapshot.  Returns `true` on success.
async fn evm_revert_http(rpc_url: &str, snapshot_id: &str) -> anyhow::Result<bool> {
    let provider = build_http_provider(rpc_url)
        .map_err(|e| anyhow::anyhow!("failed to build provider: {e}"))?;
    let result: serde_json::Value = provider
        .raw_request("evm_revert".into(), (snapshot_id,))
        .await
        .context("evm_revert RPC call failed")?;
    result.as_bool().ok_or_else(|| anyhow::anyhow!("unexpected evm_revert result type: {result}"))
}

/// Deploy the CreateX factory bytecode at its canonical address via `anvil_setCode`.
async fn deploy_createx_http(rpc_url: &str) -> anyhow::Result<()> {
    let provider = build_http_provider(rpc_url)
        .map_err(|e| anyhow::anyhow!("failed to build provider: {e}"))?;

    let createx_addr: Address = CREATEX_ADDRESS.parse().expect("valid CreateX address");

    // Check if CreateX already exists (e.g., on a forked chain where it's
    // natively deployed). Skip deployment if code is already present.
    let existing_code =
        provider.get_code_at(createx_addr).await.context("failed to check CreateX code")?;
    if !existing_code.is_empty() {
        return Ok(());
    }

    let bytecode_bytes = createx_deployed_bytecode();
    let hex: String = bytecode_bytes.iter().map(|b| format!("{b:02x}")).collect();
    let hex_str = format!("0x{hex}");
    let _: serde_json::Value = provider
        .raw_request("anvil_setCode".into(), (CREATEX_ADDRESS, &hex_str))
        .await
        .context("anvil_setCode RPC call failed")?;
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

// ── run_exec ──────────────────────────────────────────────────────────────────

async fn run_exec(
    query: Option<String>,
    all: bool,
    network: Option<String>,
    json: bool,
) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let treb_dir = cwd.join(TREB_DIR);

    ensure_treb_dir(&treb_dir)?;

    // Verify fork mode is active
    let mut fork_store = ForkStateStore::new(&treb_dir);
    fork_store.load()?;
    let state = fork_store.data();
    if state.forks.is_empty() {
        bail!("No active fork — run `treb fork enter` first");
    }

    if !all && query.is_none() {
        bail!("specify a safe tx hash or proposal ID, or use --all");
    }

    let mut registry = treb_registry::Registry::open(&cwd)?;

    // Collect queued Safe transactions
    let all_safe_txs = registry.list_safe_transactions();
    let mut queued_safe: Vec<_> = all_safe_txs
        .into_iter()
        .filter(|stx| stx.status == treb_core::types::TransactionStatus::Queued)
        .filter(|stx| stx.fork_executed_at.is_none())
        .filter(|stx| {
            network.as_ref().is_none_or(|n| {
                n.parse::<u64>().is_ok_and(|id| id == stx.chain_id)
                    || *n == stx.chain_id.to_string()
            })
        })
        .cloned()
        .collect();

    // Collect pending governor proposals
    let all_proposals = registry.list_governor_proposals();
    let mut queued_proposals: Vec<_> = all_proposals
        .into_iter()
        .filter(|p| {
            !matches!(
                p.status,
                treb_core::types::ProposalStatus::Executed
                    | treb_core::types::ProposalStatus::Canceled
                    | treb_core::types::ProposalStatus::Defeated
            )
        })
        .filter(|p| p.fork_executed_at.is_none())
        .filter(|p| {
            network.as_ref().is_none_or(|n| {
                n.parse::<u64>().is_ok_and(|id| id == p.chain_id) || *n == p.chain_id.to_string()
            })
        })
        .cloned()
        .collect();

    // Filter by query if provided
    if let Some(ref q) = query {
        queued_safe.retain(|stx| stx.safe_tx_hash.contains(q));
        queued_proposals.retain(|p| p.proposal_id.contains(q));
    }

    if queued_safe.is_empty() && queued_proposals.is_empty() {
        if json {
            println!("{{\"executed\":0}}");
        } else {
            println!("No queued items to execute.");
        }
        return Ok(());
    }

    let now = chrono::Utc::now();
    let mut executed_count = 0usize;

    // Execute queued Safe transactions
    for stx in &queued_safe {
        let fork_entry = state.forks.values().find(|f| f.chain_id == stx.chain_id);
        let Some(fork) = fork_entry else {
            if !json {
                eprintln!(
                    "  skipping safe tx {} — no active fork for chain {}",
                    &stx.safe_tx_hash[..10.min(stx.safe_tx_hash.len())],
                    stx.chain_id,
                );
            }
            continue;
        };
        let rpc_url = format!("http://127.0.0.1:{}", fork.port);

        let result = treb_forge::pipeline::fork_routing::exec_safe_from_registry(
            &rpc_url,
            &stx.safe_address,
            stx.chain_id,
            &stx.transactions,
        )
        .await;

        match result {
            Ok(receipts) => {
                // Update fork_executed_at in registry
                let mut updated = stx.clone();
                updated.fork_executed_at = Some(now);
                registry.update_safe_transaction(updated)?;
                executed_count += 1;
                if !json {
                    eprintln!(
                        "  executed safe tx {}",
                        &stx.safe_tx_hash[..10.min(stx.safe_tx_hash.len())],
                    );
                }

                // Process receipts for proxy upgrades and new deployments
                for r in &receipts {
                    let tx_hash_str = format!("{:#x}", r.hash);
                    match receipt::process_tx_receipt(&rpc_url, &tx_hash_str).await {
                        Ok(processed) => {
                            match receipt::apply_receipt_to_registry(
                                &processed,
                                &mut registry,
                                &tx_hash_str,
                            ) {
                                Ok(result) => {
                                    if !json {
                                        print_receipt_results(&result, &tx_hash_str);
                                    }
                                }
                                Err(e) => {
                                    if !json {
                                        eprintln!("    warning: failed to apply receipt: {e}");
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            if !json {
                                eprintln!("    warning: failed to process receipt: {e}");
                            }
                        }
                    }
                }
            }
            Err(e) => {
                if !json {
                    eprintln!(
                        "  failed safe tx {}: {e}",
                        &stx.safe_tx_hash[..10.min(stx.safe_tx_hash.len())],
                    );
                }
            }
        }
    }

    // Execute queued governor proposals
    for p in &queued_proposals {
        let fork_entry = state.forks.values().find(|f| f.chain_id == p.chain_id);
        let Some(fork) = fork_entry else {
            if !json {
                eprintln!(
                    "  skipping proposal {} — no active fork for chain {}",
                    &p.proposal_id[..10.min(p.proposal_id.len())],
                    p.chain_id,
                );
            }
            continue;
        };
        let rpc_url = format!("http://127.0.0.1:{}", fork.port);

        if p.actions.is_empty() {
            if !json {
                eprintln!(
                    "  skipping proposal {} — no stored actions",
                    &p.proposal_id[..10.min(p.proposal_id.len())],
                );
            }
            continue;
        }

        let result = treb_forge::pipeline::fork_routing::exec_governance_from_registry(
            &rpc_url,
            &p.governor_address,
            &p.timelock_address,
            &p.actions,
        )
        .await;

        match result {
            Ok(receipts) => {
                let mut updated = p.clone();
                updated.fork_executed_at = Some(now);
                registry.update_governor_proposal(updated)?;
                executed_count += 1;
                if !json {
                    eprintln!(
                        "  simulated proposal {}",
                        &p.proposal_id[..10.min(p.proposal_id.len())],
                    );
                }

                // Process receipts for proxy upgrades and new deployments
                for r in &receipts {
                    let tx_hash_str = format!("{:#x}", r.hash);
                    match receipt::process_tx_receipt(&rpc_url, &tx_hash_str).await {
                        Ok(processed) => {
                            match receipt::apply_receipt_to_registry(
                                &processed,
                                &mut registry,
                                &tx_hash_str,
                            ) {
                                Ok(result) => {
                                    if !json {
                                        print_receipt_results(&result, &tx_hash_str);
                                    }
                                }
                                Err(e) => {
                                    if !json {
                                        eprintln!("    warning: failed to apply receipt: {e}");
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            if !json {
                                eprintln!("    warning: failed to process receipt: {e}");
                            }
                        }
                    }
                }
            }
            Err(e) => {
                if !json {
                    eprintln!(
                        "  failed proposal {}: {e}",
                        &p.proposal_id[..10.min(p.proposal_id.len())],
                    );
                }
            }
        }
    }

    if json {
        println!("{{\"executed\":{executed_count}}}");
    } else {
        println!(
            "\nExecuted {executed_count} queued item{}.",
            if executed_count == 1 { "" } else { "s" }
        );
    }

    Ok(())
}

/// Print receipt processing results (proxy upgrades and new creations) under the exec output.
fn print_receipt_results(result: &receipt::ReceiptApplicationResult, _tx_hash: &str) {
    use crate::output;

    for up in &result.upgraded_deployments {
        eprintln!(
            "    upgraded  {}  {}  →  impl={}",
            up.contract_name,
            output::truncate_address(&up.proxy_address),
            output::truncate_address(&up.new_implementation),
        );
    }
    for creation in &result.new_creations {
        eprintln!(
            "    deployed  {}  ({})",
            output::truncate_address(&creation.address),
            creation.create_type,
        );
    }
    for dep_id in &result.verified_deployments {
        eprintln!("    verified  {dep_id}");
    }
}

/// Load a JSON file as an object map.  Returns `None` if the file is missing or not an object.
#[cfg(test)]
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

    if let Err(e) =
        std::fs::OpenOptions::new().create(true).append(true).open(&gitignore_path).and_then(
            |mut f| {
                use std::io::Write;
                f.write_all(to_append.as_bytes())
            },
        )
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
    use std::{
        fs,
        path::{Path, PathBuf},
    };
    use tempfile::TempDir;
    use treb_core::types::fork::{ForkEntry, ForkHistoryEntry};
    use treb_registry::{ForkStateStore, deployments_dir, restore_registry, snapshot_registry};

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

    fn canonical_deployments_file(project_root: &Path) -> PathBuf {
        deployments_dir(project_root).join("default").join("1.json")
    }

    // ── clap parsing tests ────────────────────────────────────────────────

    #[test]
    fn parse_enter_no_network() {
        let sub = parse_fork(&["enter"]).unwrap();
        match sub {
            ForkSubcommand::Enter { network, rpc_url, fork_block_number } => {
                assert!(network.is_empty());
                assert!(rpc_url.is_none());
                assert!(fork_block_number.is_none());
            }
            _ => panic!("expected Enter"),
        }
    }

    #[test]
    fn parse_enter_single_network() {
        let sub = parse_fork(&["enter", "--network", "mainnet"]).unwrap();
        match sub {
            ForkSubcommand::Enter { network, .. } => {
                assert_eq!(network, vec!["mainnet"]);
            }
            _ => panic!("expected Enter"),
        }
    }

    #[test]
    fn parse_enter_comma_separated_networks() {
        let sub = parse_fork(&["enter", "--network", "mainnet,sepolia"]).unwrap();
        match sub {
            ForkSubcommand::Enter { network, .. } => {
                assert_eq!(network, vec!["mainnet", "sepolia"]);
            }
            _ => panic!("expected Enter"),
        }
    }

    #[test]
    fn parse_enter_with_rpc_url_and_block() {
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
                assert_eq!(network, vec!["mainnet"]);
                assert_eq!(rpc_url.as_deref(), Some("https://eth.example.com"));
                assert_eq!(fork_block_number, Some(19_000_000));
            }
            _ => panic!("expected Enter"),
        }
    }

    #[test]
    fn parse_exit_no_args() {
        let sub = parse_fork(&["exit"]).unwrap();
        assert!(matches!(sub, ForkSubcommand::Exit));
    }

    #[test]
    fn parse_revert_no_args() {
        let sub = parse_fork(&["revert"]).unwrap();
        assert!(matches!(sub, ForkSubcommand::Revert));
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
    fn parse_logs_follow() {
        let sub = parse_fork(&["logs", "-f"]).unwrap();
        match sub {
            ForkSubcommand::Logs { follow, network } => {
                assert!(follow);
                assert!(network.is_none());
            }
            _ => panic!("expected Logs"),
        }
    }

    #[test]
    fn parse_logs_with_network() {
        let sub = parse_fork(&["logs", "--follow", "--network", "celo"]).unwrap();
        match sub {
            ForkSubcommand::Logs { follow, network } => {
                assert!(follow);
                assert_eq!(network.as_deref(), Some("celo"));
            }
            _ => panic!("expected Logs"),
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
            snapshot_dir: treb_dir.join("priv/snapshots/holistic").to_string_lossy().into_owned(),
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
    fn holistic_fork_state_written_on_enter() {
        let (_root, treb_dir) = make_treb_dir();
        let mut store = ForkStateStore::new(&treb_dir);

        // Enter holistic mode
        store.enter_fork_mode(".treb/priv/snapshots/holistic").unwrap();
        assert!(store.is_fork_mode_active());

        store.insert_active_fork(sample_entry(&treb_dir, "mainnet")).unwrap();
        store.insert_active_fork(sample_entry(&treb_dir, "sepolia")).unwrap();

        store
            .add_history(ForkHistoryEntry {
                action: "enter".into(),
                network: "mainnet, sepolia".into(),
                timestamp: Utc::now(),
                details: Some("2 networks".into()),
            })
            .unwrap();

        // Read back in fresh store
        let mut store2 = ForkStateStore::new(&treb_dir);
        store2.load().unwrap();

        assert!(store2.is_fork_mode_active());
        assert!(store2.get_active_fork("mainnet").is_some());
        assert!(store2.get_active_fork("sepolia").is_some());
        assert_eq!(store2.list_active_forks().len(), 2);
        assert_eq!(store2.data().history.len(), 1);
    }

    // ── exit clears all state ─────────────────────────────────────────────

    #[test]
    fn holistic_exit_clears_all_state() {
        let (_root, treb_dir) = make_treb_dir();
        let mut store = ForkStateStore::new(&treb_dir);

        store.enter_fork_mode(".treb/priv/snapshots/holistic").unwrap();
        store.insert_active_fork(sample_entry(&treb_dir, "mainnet")).unwrap();
        store.insert_active_fork(sample_entry(&treb_dir, "sepolia")).unwrap();

        // Exit fork mode
        store.exit_fork_mode().unwrap();

        assert!(!store.is_fork_mode_active());
        assert!(store.list_active_forks().is_empty());
        assert!(store.data().snapshot_dir.is_none());
        assert!(store.data().entered_at.is_none());
    }

    // ── backward compat: old fork.json without active field ───────────────

    #[test]
    fn backward_compat_old_fork_json_without_active() {
        let (_root, treb_dir) = make_treb_dir();
        let fork_json = r#"{"forks": {}, "history": []}"#;
        fs::write(treb_dir.join("fork.json"), fork_json).unwrap();

        let mut store = ForkStateStore::new(&treb_dir);
        store.load().unwrap();

        // active defaults to false
        assert!(!store.is_fork_mode_active());
        assert!(store.data().snapshot_dir.is_none());
        assert!(store.data().entered_at.is_none());
    }

    // ── registry snapshot/restore round-trip ─────────────────────────────

    #[test]
    fn registry_snapshot_restore_round_trip() {
        let (_root, treb_dir) = make_treb_dir();
        let snapshot_dir = treb_dir.join("priv/snapshots/holistic");
        let deployments_file = canonical_deployments_file(_root.path());
        fs::create_dir_all(deployments_file.parent().unwrap()).unwrap();

        // Write initial registry state
        fs::write(&deployments_file, r#"{"original": true}"#).unwrap();

        // Snapshot
        snapshot_registry(&treb_dir, &snapshot_dir).unwrap();

        // Overwrite registry (simulate deployments during fork session)
        fs::write(&deployments_file, r#"{"modified": true}"#).unwrap();

        // Restore
        restore_registry(&snapshot_dir, &treb_dir).unwrap();

        let content = fs::read_to_string(deployments_file).unwrap();
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
        let (root, treb_dir) = make_treb_dir();
        let snapshot_dir = treb_dir.join("priv/snapshots/holistic");
        let current_file = canonical_deployments_file(root.path());
        let snapshot_file = snapshot_dir.join("deployments").join("default").join("1.json");
        fs::create_dir_all(current_file.parent().unwrap()).unwrap();
        fs::create_dir_all(snapshot_file.parent().unwrap()).unwrap();

        // Write matching state to both locations first.
        let deployments_json = r#"{"Counter_1": {"address": "0xaaa"}}"#;
        fs::write(&current_file, deployments_json).unwrap();
        fs::write(&snapshot_file, deployments_json).unwrap();

        // Simulate a new deployment added to the current registry.
        let current_json =
            r#"{"Counter_1": {"address": "0xaaa"}, "Token_2": {"address": "0xbbb"}}"#;
        fs::write(&current_file, current_json).unwrap();

        // Diff
        let current_map = super::load_json_map(&current_file).unwrap();
        let snapshot_map = super::load_json_map(&snapshot_file).unwrap();

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
        let (root, treb_dir) = make_treb_dir();
        let snapshot_dir = treb_dir.join("priv/snapshots/holistic");
        let current_file = canonical_deployments_file(root.path());
        let snapshot_file = snapshot_dir.join("deployments").join("default").join("1.json");
        fs::create_dir_all(current_file.parent().unwrap()).unwrap();
        fs::create_dir_all(snapshot_file.parent().unwrap()).unwrap();

        let deployments_json = r#"{"Counter_1": {"address": "0xaaa"}}"#;
        fs::write(&current_file, deployments_json).unwrap();
        fs::write(&snapshot_file, deployments_json).unwrap();

        let current_map = super::load_json_map(&current_file).unwrap();
        let snapshot_map = super::load_json_map(&snapshot_file).unwrap();

        let changes: Vec<_> = current_map
            .keys()
            .filter(|k| !snapshot_map.contains_key(*k))
            .chain(snapshot_map.keys().filter(|k| !current_map.contains_key(*k)))
            .collect();
        assert!(changes.is_empty(), "expected no changes: {changes:?}");
    }

    // ── deterministic fork port ───────────────────────────────────────────

    #[test]
    fn deterministic_port_examples() {
        use treb_forge::anvil::deterministic_fork_port;
        assert_eq!(deterministic_fork_port(42220), 9734); // celo
        assert_eq!(deterministic_fork_port(1), 9701); // ethereum
        assert_eq!(deterministic_fork_port(42161), 9754); // arbitrum
        assert_eq!(deterministic_fork_port(11155111), 9774); // sepolia
        assert_eq!(deterministic_fork_port(44787), 9773); // alfajores
    }
}
