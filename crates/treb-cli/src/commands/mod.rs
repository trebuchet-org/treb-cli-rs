use std::path::Path;

use alloy_chains::Chain;
use anyhow::Context;

pub mod addressbook;
pub mod compose;
pub mod config;
pub mod dev;
pub mod fork;

pub mod init;
pub mod list;
pub mod migrate;

pub mod networks;
pub mod prune;
pub mod queued;
pub mod receipt;
pub mod register;
pub mod reset;
pub mod resolve;
pub mod run;
pub mod show;
pub mod sync;
pub mod tag;
pub mod verify;
pub mod version;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommandScope {
    pub namespace: Option<String>,
    pub network: Option<String>,
}

pub(crate) fn resolve_command_scope(
    cwd: &Path,
    namespace: Option<String>,
    network: Option<String>,
) -> anyhow::Result<CommandScope> {
    let local = treb_config::load_local_config(cwd).context("failed to load local config")?;

    let namespace = namespace.or(Some(local.namespace));
    let network =
        network.or(if local.network.trim().is_empty() { None } else { Some(local.network) });

    Ok(CommandScope { namespace, network })
}

pub(crate) fn resolve_chain_id_arg(network: &str) -> anyhow::Result<u64> {
    if let Ok(id) = network.parse::<u64>() {
        return Ok(id);
    }

    let chain: Chain =
        network.parse().map_err(|_| anyhow::anyhow!("unknown network: {network}"))?;
    Ok(chain.id())
}

pub(crate) async fn resolve_chain_id_for_network(
    cwd: &Path,
    network: Option<&str>,
) -> anyhow::Result<Option<u64>> {
    let Some(network) = network else {
        return Ok(None);
    };

    if let Ok(id) = resolve_chain_id_arg(network) {
        return Ok(Some(id));
    }

    let treb_dir = cwd.join(".treb");
    let mut fork_store = treb_registry::ForkStateStore::new(&treb_dir);
    if fork_store.load().is_ok() {
        if let Some(fork) = fork_store.get_active_fork(network) {
            return Ok(Some(fork.chain_id));
        }
    }

    if let Some(url) = crate::commands::run::resolve_rpc_url_for_chain_id(network, cwd) {
        let chain_id = crate::commands::run::fetch_chain_id(&url)
            .await
            .with_context(|| format!("failed to resolve chain ID for network '{network}'"))?;
        return Ok(Some(chain_id));
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    #[test]
    fn resolve_command_scope_uses_local_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        treb_config::save_local_config(
            tmp.path(),
            &treb_config::LocalConfig {
                namespace: "staging".to_string(),
                network: "celo".to_string(),
            },
        )
        .unwrap();

        let scope = resolve_command_scope(tmp.path(), None, None).unwrap();

        assert_eq!(scope.namespace.as_deref(), Some("staging"));
        assert_eq!(scope.network.as_deref(), Some("celo"));
    }

    #[test]
    fn resolve_command_scope_prefers_explicit_flags() {
        let tmp = tempfile::tempdir().unwrap();
        treb_config::save_local_config(
            tmp.path(),
            &treb_config::LocalConfig {
                namespace: "staging".to_string(),
                network: "celo".to_string(),
            },
        )
        .unwrap();

        let scope = resolve_command_scope(
            tmp.path(),
            Some("prod".to_string()),
            Some("sepolia".to_string()),
        )
        .unwrap();

        assert_eq!(scope.namespace.as_deref(), Some("prod"));
        assert_eq!(scope.network.as_deref(), Some("sepolia"));
    }

    #[tokio::test]
    async fn resolve_chain_id_for_network_uses_active_fork_alias() {
        let tmp = tempfile::tempdir().unwrap();
        let treb_dir = tmp.path().join(".treb");
        std::fs::create_dir_all(&treb_dir).unwrap();

        let mut store = treb_registry::ForkStateStore::new(&treb_dir);
        store.enter_fork_mode("/tmp/snapshot").unwrap();
        store
            .insert_active_fork(treb_core::types::fork::ForkEntry {
                network: "celo_sepolia".to_string(),
                instance_name: None,
                rpc_url: "http://127.0.0.1:9760".to_string(),
                port: 9760,
                chain_id: 11142220,
                fork_url: "https://example.invalid".to_string(),
                fork_block_number: None,
                snapshot_dir: "/tmp/snapshot".to_string(),
                started_at: Utc.with_ymd_and_hms(2026, 3, 25, 0, 0, 0).unwrap(),
                env_var_name: "CELO_SEPOLIA_RPC_URL".to_string(),
                original_rpc: "https://example.invalid".to_string(),
                anvil_pid: 1,
                pid_file: String::new(),
                log_file: String::new(),
                entered_at: Utc.with_ymd_and_hms(2026, 3, 25, 0, 0, 0).unwrap(),
                snapshots: Vec::new(),
            })
            .unwrap();

        let resolved =
            resolve_chain_id_for_network(tmp.path(), Some("celo_sepolia")).await.unwrap();

        assert_eq!(resolved, Some(11142220));
    }
}
