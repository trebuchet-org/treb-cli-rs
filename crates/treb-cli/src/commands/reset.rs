//! `treb reset` command implementation.
//!
//! Wipes registry state with optional --network and --namespace scoping,
//! creating a timestamped backup before any mutation.

use std::env;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context};
use clap::Args;
use serde::Serialize;
use treb_registry::{snapshot_registry, Registry, REGISTRY_DIR};

use crate::output;

// ── Args ─────────────────────────────────────────────────────────────────────

/// Arguments for the `treb reset` command.
#[derive(Args, Debug)]
#[command(long_about = "Clear all deployments and transactions from the registry, \
optionally scoped to a specific network (by chain ID) or namespace. A timestamped \
backup is created under `.treb/backups/` before removing any data.")]
pub struct ResetArgs {
    /// Filter reset to a specific network (by chain ID)
    #[arg(long)]
    pub network: Option<String>,

    /// Filter reset to a specific namespace
    #[arg(long)]
    pub namespace: Option<String>,

    /// Skip confirmation prompt
    #[arg(long, short = 'y')]
    pub yes: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

// ── run ───────────────────────────────────────────────────────────────────────

/// Entry point for `treb reset`.
pub async fn run(args: ResetArgs) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;

    if !cwd.join("foundry.toml").exists() {
        bail!(
            "no foundry.toml found in {}\n\nRun `forge init`, then `treb init`.",
            cwd.display()
        );
    }
    if !cwd.join(".treb").exists() {
        bail!(
            "project not initialized — .treb/ directory not found\n\nRun `treb init` first."
        );
    }

    // Resolve chain ID filter.
    let chain_id_filter: Option<u64> = match &args.network {
        Some(s) => match s.parse::<u64>() {
            Ok(id) => Some(id),
            Err(_) => bail!("--network must be a chain ID (e.g. 1, 31337); got '{}'", s),
        },
        None => None,
    };

    let mut registry = Registry::open(&cwd).context("failed to open registry")?;

    // Collect entries to remove.
    let deployments_to_remove: Vec<String> = registry
        .list_deployments()
        .iter()
        .filter(|d| {
            let chain_ok = chain_id_filter.is_none_or(|id| d.chain_id == id);
            let ns_ok = args
                .namespace
                .as_deref()
                .is_none_or(|ns| d.namespace.eq_ignore_ascii_case(ns));
            chain_ok && ns_ok
        })
        .map(|d| d.id.clone())
        .collect();

    let transactions_to_remove: Vec<String> = registry
        .list_transactions()
        .iter()
        .filter(|t| chain_id_filter.is_none_or(|id| t.chain_id == id))
        .map(|t| t.id.clone())
        .collect();

    let safe_txs_to_remove: Vec<String> = registry
        .list_safe_transactions()
        .iter()
        .filter(|t| chain_id_filter.is_none_or(|id| t.chain_id == id))
        .map(|t| t.safe_tx_hash.clone())
        .collect();

    let governor_proposals_to_remove: Vec<String> = registry
        .list_governor_proposals()
        .iter()
        .filter(|p| chain_id_filter.is_none_or(|id| p.chain_id == id))
        .map(|p| p.proposal_id.clone())
        .collect();

    let total = deployments_to_remove.len()
        + transactions_to_remove.len()
        + safe_txs_to_remove.len()
        + governor_proposals_to_remove.len();

    if total == 0 {
        println!("Nothing to reset.");
        return Ok(());
    }

    // Confirm.
    if !args.yes {
        let message = format!(
            "About to remove {} deployment(s), {} transaction(s), {} safe transaction(s), \
             {} governor proposal(s). A backup will be created first. Continue?",
            deployments_to_remove.len(),
            transactions_to_remove.len(),
            safe_txs_to_remove.len(),
            governor_proposals_to_remove.len(),
        );
        if !crate::ui::prompt::confirm(&message, false) {
            println!("Cancelled.");
            return Ok(());
        }
    }

    // Backup.
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let registry_dir = cwd.join(REGISTRY_DIR);
    let backup_dir = registry_dir.join(format!("backups/reset-{ts}"));
    snapshot_registry(&registry_dir, &backup_dir)
        .with_context(|| format!("failed to create reset backup at {}", backup_dir.display()))?;

    // Remove entries.
    for id in &deployments_to_remove {
        let _ = registry.remove_deployment(id);
    }
    for id in &transactions_to_remove {
        let _ = registry.remove_transaction(id);
    }
    for hash in &safe_txs_to_remove {
        let _ = registry.remove_safe_transaction(hash);
    }
    for id in &governor_proposals_to_remove {
        let _ = registry.remove_governor_proposal(id);
    }

    if args.json {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct ResetResult {
            removed_deployments: usize,
            removed_transactions: usize,
            removed_safe_transactions: usize,
            removed_governor_proposals: usize,
            backup_path: String,
        }
        output::print_json(&ResetResult {
            removed_deployments: deployments_to_remove.len(),
            removed_transactions: transactions_to_remove.len(),
            removed_safe_transactions: safe_txs_to_remove.len(),
            removed_governor_proposals: governor_proposals_to_remove.len(),
            backup_path: backup_dir.display().to_string(),
        })?;
    } else {
        println!("Reset complete.");
        println!("  Deployments removed:         {}", deployments_to_remove.len());
        println!("  Transactions removed:        {}", transactions_to_remove.len());
        println!("  Safe transactions removed:   {}", safe_txs_to_remove.len());
        println!("  Governor proposals removed:  {}", governor_proposals_to_remove.len());
        println!("Backup created at: {}", backup_dir.display());
    }

    Ok(())
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::Utc;
    use tempfile::TempDir;
    use treb_core::types::{
        ArtifactInfo, DeploymentMethod, DeploymentStrategy, DeploymentType, VerificationInfo,
        VerificationStatus,
    };
    use treb_registry::Registry;

    fn make_deployment(id: &str, chain_id: u64, namespace: &str) -> treb_core::types::Deployment {
        let ts = Utc::now();
        treb_core::types::Deployment {
            id: id.to_string(),
            namespace: namespace.to_string(),
            chain_id,
            contract_name: "TestContract".to_string(),
            label: "v1".to_string(),
            address: format!("0x{:040x}", 1u64),
            deployment_type: DeploymentType::Singleton,
            transaction_id: String::new(),
            deployment_strategy: DeploymentStrategy {
                method: DeploymentMethod::Create,
                salt: String::new(),
                init_code_hash: String::new(),
                factory: String::new(),
                constructor_args: String::new(),
                entropy: String::new(),
            },
            proxy_info: None,
            artifact: ArtifactInfo {
                path: "contracts/Test.sol".to_string(),
                compiler_version: "0.8.24".to_string(),
                bytecode_hash: "0xabc".to_string(),
                script_path: "script/Deploy.s.sol".to_string(),
                git_commit: "abc123".to_string(),
            },
            verification: VerificationInfo {
                status: VerificationStatus::Unverified,
                etherscan_url: String::new(),
                verified_at: None,
                reason: String::new(),
                verifiers: HashMap::new(),
            },
            tags: None,
            created_at: ts,
            updated_at: ts,
        }
    }

    #[test]
    fn collect_deployments_by_chain_id() {
        let dir = TempDir::new().unwrap();
        let mut registry = Registry::init(dir.path()).unwrap();

        registry
            .insert_deployment(make_deployment("dep-1", 1, "default"))
            .unwrap();
        registry
            .insert_deployment(make_deployment("dep-2", 42220, "default"))
            .unwrap();

        // Simulate what run() does when chain_id_filter = Some(1)
        let to_remove: Vec<String> = registry
            .list_deployments()
            .iter()
            .filter(|d| d.chain_id == 1)
            .map(|d| d.id.clone())
            .collect();

        assert_eq!(to_remove, vec!["dep-1"]);
    }

    #[test]
    fn collect_deployments_by_namespace() {
        let dir = TempDir::new().unwrap();
        let mut registry = Registry::init(dir.path()).unwrap();

        registry
            .insert_deployment(make_deployment("dep-1", 1, "default"))
            .unwrap();
        registry
            .insert_deployment(make_deployment("dep-2", 1, "staging"))
            .unwrap();

        let to_remove: Vec<String> = registry
            .list_deployments()
            .iter()
            .filter(|d| d.namespace.eq_ignore_ascii_case("default"))
            .map(|d| d.id.clone())
            .collect();

        assert_eq!(to_remove, vec!["dep-1"]);
    }

    #[test]
    fn empty_registry_returns_nothing_to_reset() {
        let dir = TempDir::new().unwrap();
        let registry = Registry::init(dir.path()).unwrap();

        let total: usize = registry.deployment_count()
            + registry.transaction_count()
            + registry.safe_transaction_count()
            + registry.governor_proposal_count();
        assert_eq!(total, 0);
    }

    #[test]
    fn full_reset_empties_lookup_index() {
        let dir = TempDir::new().unwrap();
        let mut registry = Registry::init(dir.path()).unwrap();

        registry
            .insert_deployment(make_deployment("dep-1", 1, "default"))
            .unwrap();
        registry
            .insert_deployment(make_deployment("dep-2", 1, "staging"))
            .unwrap();

        // Verify lookup index is non-empty before reset.
        let index_before = registry.load_lookup_index().unwrap();
        assert!(!index_before.by_name.is_empty(), "lookup should be populated");

        // Simulate a full reset: remove all deployments.
        let ids: Vec<String> = registry
            .list_deployments()
            .iter()
            .map(|d| d.id.clone())
            .collect();
        for id in ids {
            registry.remove_deployment(&id).unwrap();
        }

        // Lookup index should now be empty.
        let index_after = registry.load_lookup_index().unwrap();
        assert!(
            index_after.by_name.is_empty(),
            "lookup.by_name should be empty after full reset"
        );
        assert!(
            index_after.by_address.is_empty(),
            "lookup.by_address should be empty after full reset"
        );
    }
}
