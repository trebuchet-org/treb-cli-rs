//! `treb fork` subcommands — enter/exit fork mode, status, history, diff.

use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context};
use chrono::Utc;
use clap::Subcommand;
use treb_core::types::fork::{ForkEntry, ForkHistoryEntry};
use treb_registry::{remove_snapshot, restore_registry, snapshot_registry, ForkStateStore};

const TREB_DIR: &str = ".treb";
const SNAPSHOT_BASE: &str = "snapshots";

// ── Subcommand enum ───────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum ForkSubcommand {
    /// Enter fork mode for a network: snapshot registry and record fork state
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
    Exit {
        /// Network name to exit
        #[arg(long)]
        network: String,
    },
    /// Revert the fork to its last snapshot
    Revert {
        /// Network name to revert
        #[arg(long)]
        network: String,
        /// Revert all active forks
        #[arg(long)]
        all: bool,
    },
    /// Restart the fork from a new block
    Restart {
        /// Network name to restart
        #[arg(long)]
        network: String,
        /// Fork block number to reset to (uses latest if omitted)
        #[arg(long)]
        fork_block_number: Option<u64>,
    },
    /// Show active fork status
    Status {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show fork history
    History {
        /// Filter by network name
        #[arg(long)]
        network: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Diff current registry vs snapshot
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
        ForkSubcommand::Revert { .. } => {
            println!("fork revert: not yet implemented");
            Ok(())
        }
        ForkSubcommand::Restart { .. } => {
            println!("fork restart: not yet implemented");
            Ok(())
        }
        ForkSubcommand::Status { .. } => {
            println!("fork status: not yet implemented");
            Ok(())
        }
        ForkSubcommand::History { .. } => {
            println!("fork history: not yet implemented");
            Ok(())
        }
        ForkSubcommand::Diff { .. } => {
            println!("fork diff: not yet implemented");
            Ok(())
        }
    }
}

// ── run_enter ─────────────────────────────────────────────────────────────────

/// Enter fork mode for a network.
///
/// Validates the project is initialized, resolves the upstream RPC URL,
/// snapshots the registry, and records a [`ForkEntry`] in `fork-state.json`.
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
    let entry = ForkEntry {
        network: network.clone(),
        rpc_url: String::new(),
        port: 0,
        chain_id,
        fork_url,
        fork_block_number,
        snapshot_dir: snapshot_dir.to_string_lossy().into_owned(),
        started_at: Utc::now(),
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
/// and removes the [`ForkEntry`] from `fork-state.json`.
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
            started_at: Utc::now(),
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
}
