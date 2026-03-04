//! `treb fork` subcommands — enter/exit fork mode, status, history, diff, revert, restart.

use std::collections::BTreeSet;
use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context};
use chrono::Utc;
use clap::Subcommand;
use tokio::net::TcpStream;
use treb_core::types::fork::{ForkEntry, ForkHistoryEntry};
use treb_forge::createx::createx_deployed_bytecode;
use treb_registry::{
    remove_snapshot, restore_registry, snapshot_registry, ForkStateStore, DEPLOYMENTS_FILE,
    TRANSACTIONS_FILE,
};

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
        /// Network name (must match an entry in foundry.toml [rpc_endpoints])
        #[arg(long)]
        network: String,
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
        /// Network name to exit
        #[arg(long)]
        network: String,
    },
    /// Revert the fork to its last snapshot
    ///
    /// Restores the registry from the snapshot taken when fork mode was entered,
    /// discarding any deployments made during the fork session.
    Revert {
        /// Network name to revert
        #[arg(long)]
        network: String,
        /// Revert all active forks
        #[arg(long)]
        all: bool,
    },
    /// Restart the fork from a new block
    ///
    /// Resets the local Anvil node to a fresh fork at the given block number
    /// (or at the latest block if omitted) without exiting fork mode.
    Restart {
        /// Network name to restart
        #[arg(long)]
        network: String,
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
        /// Network name to diff
        #[arg(long)]
        network: String,
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
        ForkSubcommand::Exit { network } => run_exit(network).await,
        ForkSubcommand::Revert { network, all } => run_revert(network, all).await,
        ForkSubcommand::Restart { network, fork_block_number } => {
            run_restart(network, fork_block_number).await
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
) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let treb_dir = cwd.join(TREB_DIR);

    ensure_treb_dir(&treb_dir)?;

    // Resolve upstream RPC URL
    let fork_url = resolve_fork_url(&cwd, &network, rpc_url_override)?;

    // Get chain_id from upstream
    let chain_id = fetch_chain_id(&fork_url)
        .await
        .with_context(|| format!("failed to get chain ID from RPC URL: {fork_url}"))?;

    // Load fork state and check not already forked
    let mut store = ForkStateStore::new(&treb_dir);
    store.load().context("failed to load fork state")?;

    if store.get_active_fork(&network).is_some() {
        bail!(
            "network '{}' is already forked; run `treb fork exit --network {}` first",
            network,
            network
        );
    }

    // Create snapshot dir and snapshot registry
    let snapshot_dir = treb_dir.join(SNAPSHOT_BASE).join(&network);
    snapshot_registry(&treb_dir, &snapshot_dir).context("failed to snapshot registry")?;

    // Build and insert fork entry (rpc_url/port are placeholders until dev anvil start)
    let now = Utc::now();
    let entry = ForkEntry {
        network: network.clone(),
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

    println!("Entered fork mode for network '{network}'.");
    println!("Registry snapshot saved to {}", snapshot_dir.display());
    println!("Run `treb dev anvil start --network {network}` to start a local Anvil node.");

    Ok(())
}

// ── run_exit ──────────────────────────────────────────────────────────────────

/// Exit fork mode for a network.
///
/// Restores the registry from its snapshot, removes the snapshot directory,
/// and removes the [`ForkEntry`] from `fork.json`.
pub async fn run_exit(network: String) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let treb_dir = cwd.join(TREB_DIR);

    let mut store = ForkStateStore::new(&treb_dir);
    store.load().context("failed to load fork state")?;

    // Remove active fork entry (errors if not active)
    let entry = store
        .remove_active_fork(&network)
        .with_context(|| format!("cannot exit: network '{network}' is not actively forked"))?;

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

    let client =
        reqwest::Client::builder().timeout(Duration::from_secs(10)).build()?;

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
                .with_context(|| {
                    format!("failed to revert EVM state for network '{net}'")
                })?;
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
            .with_context(|| {
                format!("failed to take new EVM snapshot for network '{net}'")
            })?;

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

/// Restart an Anvil fork from a fresh block.
///
/// Calls `anvil_reset` to reinitialize the Anvil instance with the same fork URL
/// (optionally at a different block number), re-deploys the CreateX factory,
/// takes a new EVM snapshot, restores the registry, and adds a restart history entry.
pub async fn run_restart(
    network: String,
    fork_block_number: Option<u64>,
) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let treb_dir = cwd.join(TREB_DIR);

    let mut store = ForkStateStore::new(&treb_dir);
    store.load().context("failed to load fork state")?;

    let entry = store
        .get_active_fork(&network)
        .ok_or_else(|| anyhow::anyhow!("network '{}' is not in fork mode", network))?
        .clone();

    if !is_port_reachable(entry.port).await {
        bail!(
            "Anvil node for network '{}' is not reachable at port {}; cannot restart",
            network,
            entry.port
        );
    }

    let client =
        reqwest::Client::builder().timeout(Duration::from_secs(10)).build()?;

    // Determine the block to reset to.
    let blk = fork_block_number.or(entry.fork_block_number);

    // Reset Anvil to a fresh fork state.
    anvil_reset_http(&client, &entry.rpc_url, &entry.fork_url, blk)
        .await
        .with_context(|| format!("failed to reset Anvil for network '{network}'"))?;

    println!("Anvil reset to {} (block: {}).", entry.fork_url, blk.map_or("latest".into(), |b| b.to_string()));

    // Re-deploy the CreateX factory.
    deploy_createx_http(&client, &entry.rpc_url)
        .await
        .with_context(|| format!("failed to re-deploy CreateX for network '{network}'"))?;

    println!("CreateX factory re-deployed at {CREATEX_ADDRESS}.");

    // Take a new EVM snapshot as the fresh baseline.
    let snapshot_id = evm_snapshot_http(&client, &entry.rpc_url)
        .await
        .with_context(|| format!("failed to take EVM snapshot for network '{network}'"))?;

    // Restore registry from snapshot.
    let snapshot_dir = PathBuf::from(&entry.snapshot_dir);
    restore_registry(&snapshot_dir, &treb_dir).context("failed to restore registry")?;

    // Update the fork entry.
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

    // Add history entry.
    store
        .add_history(ForkHistoryEntry {
            action: "restart".into(),
            network: network.clone(),
            timestamp: Utc::now(),
            details: Some(format!("Anvil reset; snapshot: {snapshot_id}")),
        })
        .context("failed to record restart history")?;

    println!("Restarted fork for network '{network}'.");
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
        println!("No active forks.");
        return Ok(());
    }

    let mut table = crate::output::build_table(&[
        "Network",
        "RPC URL",
        "Port",
        "Chain ID",
        "Fork Block",
        "Started At",
        "Status",
    ]);

    for entry in &forks {
        let running = is_port_reachable(entry.port).await;
        let status = if running { "running" } else { "stopped" };
        let fork_block = entry
            .fork_block_number
            .map(|b| b.to_string())
            .unwrap_or_else(|| "latest".into());

        table.add_row(vec![
            entry.network.as_str(),
            entry.rpc_url.as_str(),
            &entry.port.to_string(),
            &entry.chain_id.to_string(),
            fork_block.as_str(),
            &entry.started_at.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            status,
        ]);
    }

    crate::output::print_table(&table);
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
        let filter_msg = network
            .as_deref()
            .map_or_else(String::new, |n| format!(" for network '{n}'"));
        println!("No fork history{filter_msg}.");
        return Ok(());
    }

    let mut table =
        crate::output::build_table(&["Timestamp", "Action", "Network", "Details"]);

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
    let json: serde_json::Value =
        resp.json().await.context("failed to parse JSON-RPC response")?;
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
    let result =
        json_rpc_call(client, rpc_url, "evm_snapshot", serde_json::Value::Null).await?;
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
    let result = json_rpc_call(
        client,
        rpc_url,
        "evm_revert",
        serde_json::json!([snapshot_id]),
    )
    .await?;
    result
        .as_bool()
        .ok_or_else(|| anyhow::anyhow!("unexpected evm_revert result type: {result}"))
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
    json_rpc_call(
        client,
        rpc_url,
        "anvil_reset",
        serde_json::json!([{ "forking": forking }]),
    )
    .await?;
    Ok(())
}

/// Deploy the CreateX factory bytecode at its canonical address via `anvil_setCode`.
async fn deploy_createx_http(
    client: &reqwest::Client,
    rpc_url: &str,
) -> anyhow::Result<()> {
    let bytecode_bytes = createx_deployed_bytecode();
    let hex: String = bytecode_bytes.iter().map(|b| format!("{b:02x}")).collect();
    let hex_str = format!("0x{hex}");
    json_rpc_call(
        client,
        rpc_url,
        "anvil_setCode",
        serde_json::json!([CREATEX_ADDRESS, hex_str]),
    )
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
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;
    use treb_core::types::fork::{ForkEntry, ForkHistoryEntry};
    use treb_registry::{restore_registry, snapshot_registry, ForkStateStore, DEPLOYMENTS_FILE};

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
            rpc_url: String::new(),
            port: 0,
            chain_id: 1,
            fork_url: "https://eth.example.com".into(),
            fork_block_number: None,
            snapshot_dir: treb_dir
                .join("snapshots")
                .join(network)
                .to_string_lossy()
                .into_owned(),
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
        let current_json = r#"{"Counter_1": {"address": "0xaaa"}, "Token_2": {"address": "0xbbb"}}"#;
        fs::write(treb_dir.join(DEPLOYMENTS_FILE), current_json).unwrap();

        // Diff
        let current_map = super::load_json_map(&treb_dir.join(DEPLOYMENTS_FILE)).unwrap();
        let snapshot_map = super::load_json_map(&snapshot_dir.join(DEPLOYMENTS_FILE)).unwrap();

        let added: Vec<_> = current_map
            .keys()
            .filter(|k| !snapshot_map.contains_key(*k))
            .collect();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0], "Token_2");

        let removed: Vec<_> = snapshot_map
            .keys()
            .filter(|k| !current_map.contains_key(*k))
            .collect();
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
}
