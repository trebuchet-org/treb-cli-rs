//! `treb prune` command implementation.
//!
//! Scans the deployment registry for broken cross-references, reports prune
//! candidates, and (in destructive mode) removes them with a timestamped backup.

use std::env;
use std::io::{self, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context};
use clap::Args;
use serde::{Deserialize, Serialize};
use treb_registry::{snapshot_registry, Registry, REGISTRY_DIR};

use crate::output;

// ── Args ─────────────────────────────────────────────────────────────────────

/// Arguments for the `treb prune` command.
#[derive(Args, Debug)]
pub struct PruneArgs {
    /// Report prune candidates without deleting anything
    #[arg(long)]
    pub dry_run: bool,

    /// Include pending transactions in the prune scan
    #[arg(long)]
    pub include_pending: bool,

    /// Filter candidates to a specific network (by chain ID)
    #[arg(long)]
    pub network: Option<String>,

    /// Skip confirmation prompt (destructive mode only)
    #[arg(long, short = 'y')]
    pub yes: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

// ── Types ─────────────────────────────────────────────────────────────────────

/// The reason a registry entry was flagged for pruning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PruneCandidateKind {
    /// A deployment references a transaction ID that does not exist.
    BrokenTransactionRef,
    /// A transaction references a deployment ID that does not exist.
    BrokenDeploymentRef,
    /// The deployment's contract bytecode is absent at its address on-chain.
    DestroyedOnChain,
    /// A pending transaction entry with no confirmed execution.
    OrphanedPendingEntry,
}

impl std::fmt::Display for PruneCandidateKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PruneCandidateKind::BrokenTransactionRef => write!(f, "BrokenTransactionRef"),
            PruneCandidateKind::BrokenDeploymentRef => write!(f, "BrokenDeploymentRef"),
            PruneCandidateKind::DestroyedOnChain => write!(f, "DestroyedOnChain"),
            PruneCandidateKind::OrphanedPendingEntry => write!(f, "OrphanedPendingEntry"),
        }
    }
}

/// A registry entry identified as a candidate for pruning.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PruneCandidate {
    /// The ID of the entry (deployment ID or transaction ID).
    pub id: String,
    /// Why this entry is flagged.
    pub kind: PruneCandidateKind,
    /// Human-readable description of the issue.
    pub reason: String,
    /// Chain ID of the entry, if available.
    pub chain_id: Option<u64>,
}

// ── find_prune_candidates ─────────────────────────────────────────────────────

/// Scan `registry` for broken cross-references and return all prune candidates.
///
/// If `chain_id_filter` is `Some(id)`, only entries matching that chain are
/// included in the results. If `include_pending` is true, pending transactions
/// with no on-chain confirmation are also flagged.
pub fn find_prune_candidates(
    registry: &Registry,
    chain_id_filter: Option<u64>,
    include_pending: bool,
) -> Vec<PruneCandidate> {
    let mut candidates = Vec::new();

    // ── Check deployments ────────────────────────────────────────────────
    for dep in registry.list_deployments() {
        // Apply chain filter.
        if let Some(filter_id) = chain_id_filter {
            if dep.chain_id != filter_id {
                continue;
            }
        }

        // Flag deployments that point to a missing transaction.
        if !dep.transaction_id.is_empty()
            && registry.get_transaction(&dep.transaction_id).is_none()
        {
            candidates.push(PruneCandidate {
                id: dep.id.clone(),
                kind: PruneCandidateKind::BrokenTransactionRef,
                reason: format!(
                    "deployment '{}' references missing transaction '{}'",
                    dep.id, dep.transaction_id
                ),
                chain_id: Some(dep.chain_id),
            });
        }
    }

    // ── Check transactions ───────────────────────────────────────────────
    for tx in registry.list_transactions() {
        // Apply chain filter.
        if let Some(filter_id) = chain_id_filter {
            if tx.chain_id != filter_id {
                continue;
            }
        }

        // Flag transactions that reference missing deployments.
        for dep_id in &tx.deployments {
            if registry.get_deployment(dep_id).is_none() {
                candidates.push(PruneCandidate {
                    id: tx.id.clone(),
                    kind: PruneCandidateKind::BrokenDeploymentRef,
                    reason: format!(
                        "transaction '{}' references missing deployment '{}'",
                        tx.id, dep_id
                    ),
                    chain_id: Some(tx.chain_id),
                });
                // Only flag once per transaction even if multiple refs are broken.
                break;
            }
        }

        // Flag orphaned pending entries if requested.
        if include_pending {
            use treb_core::types::TransactionStatus;
            if tx.status == TransactionStatus::Queued {
                candidates.push(PruneCandidate {
                    id: tx.id.clone(),
                    kind: PruneCandidateKind::OrphanedPendingEntry,
                    reason: format!(
                        "transaction '{}' has status Queued and may be orphaned",
                        tx.id
                    ),
                    chain_id: Some(tx.chain_id),
                });
            }
        }
    }

    candidates
}

// ── backup ────────────────────────────────────────────────────────────────────

/// Create a timestamped backup of the registry under `.treb/backups/prune-<ts>/`.
///
/// Returns the path to the backup directory.
pub fn backup_registry(project_root: &Path) -> anyhow::Result<std::path::PathBuf> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let registry_dir = project_root.join(REGISTRY_DIR);
    let backup_dir = registry_dir.join(format!("backups/prune-{ts}"));
    snapshot_registry(&registry_dir, &backup_dir)
        .with_context(|| format!("failed to create prune backup at {}", backup_dir.display()))?;
    Ok(backup_dir)
}

// ── run ───────────────────────────────────────────────────────────────────────

/// Entry point for `treb prune`.
pub async fn run(args: PruneArgs) -> anyhow::Result<()> {
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

    // Resolve chain ID filter from --network argument.
    let chain_id_filter: Option<u64> = match &args.network {
        Some(s) => match s.parse::<u64>() {
            Ok(id) => Some(id),
            Err(_) => bail!("--network must be a chain ID (e.g. 1, 31337); got '{}'", s),
        },
        None => None,
    };

    let registry = Registry::open(&cwd).context("failed to open registry")?;
    let candidates = find_prune_candidates(&registry, chain_id_filter, args.include_pending);

    if candidates.is_empty() {
        println!("Nothing to prune.");
        return Ok(());
    }

    // Dry-run: just display candidates.
    if args.dry_run {
        if args.json {
            output::print_json(&candidates)?;
        } else {
            let mut table = output::build_table(&["ID", "Kind", "Reason", "Chain ID"]);
            for c in &candidates {
                table.add_row(vec![
                    c.id.clone(),
                    c.kind.to_string(),
                    c.reason.clone(),
                    c.chain_id.map(|id| id.to_string()).unwrap_or_default(),
                ]);
            }
            output::print_table(&table);
            println!("\n{} prune candidate(s) found. Re-run without --dry-run to remove.", candidates.len());
        }
        return Ok(());
    }

    // Destructive mode: confirm, backup, then remove.
    if !args.yes {
        eprint!(
            "About to remove {} entry(s). A backup will be created first. Continue? [y/N] ",
            candidates.len()
        );
        io::stderr().flush()?;
        let mut response = String::new();
        io::stdin().read_line(&mut response)?;
        if !response.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    let backup_path = backup_registry(&cwd)?;

    // Re-open registry mutably for removals.
    let mut registry = Registry::open(&cwd).context("failed to open registry")?;

    let mut removed: Vec<PruneCandidate> = Vec::new();
    for c in candidates {
        match c.kind {
            PruneCandidateKind::BrokenTransactionRef | PruneCandidateKind::DestroyedOnChain => {
                if registry.remove_deployment(&c.id).is_ok() {
                    removed.push(c);
                }
            }
            PruneCandidateKind::BrokenDeploymentRef
            | PruneCandidateKind::OrphanedPendingEntry => {
                if registry.remove_transaction(&c.id).is_ok() {
                    removed.push(c);
                }
            }
        }
    }

    if args.json {
        #[derive(Serialize)]
        struct PruneResult<'a> {
            removed: &'a [PruneCandidate],
            backup_path: String,
        }
        output::print_json(&PruneResult {
            removed: &removed,
            backup_path: backup_path.display().to_string(),
        })?;
    } else {
        let mut table = output::build_table(&["ID", "Kind", "Reason", "Chain ID"]);
        for c in &removed {
            table.add_row(vec![
                c.id.clone(),
                c.kind.to_string(),
                c.reason.clone(),
                c.chain_id.map(|id| id.to_string()).unwrap_or_default(),
            ]);
        }
        output::print_table(&table);
        println!("\nRemoved {} entry(s).", removed.len());
        println!("Backup created at: {}", backup_path.display());
    }

    Ok(())
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use chrono::Utc;
    use tempfile::TempDir;
    use treb_core::types::{
        ArtifactInfo, DeploymentMethod, DeploymentStrategy, DeploymentType, TransactionStatus,
        VerificationInfo, VerificationStatus,
    };
    use treb_registry::Registry;

    fn make_deployment(id: &str, tx_id: &str, chain_id: u64) -> treb_core::types::Deployment {
        let ts = Utc::now();
        treb_core::types::Deployment {
            id: id.to_string(),
            namespace: "default".to_string(),
            chain_id,
            contract_name: "TestContract".to_string(),
            label: "v1".to_string(),
            address: format!("0x{:040x}", 1u64),
            deployment_type: DeploymentType::Singleton,
            transaction_id: tx_id.to_string(),
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

    fn make_transaction(
        id: &str,
        dep_ids: Vec<String>,
        chain_id: u64,
        status: TransactionStatus,
    ) -> treb_core::types::Transaction {
        let ts = Utc::now();
        treb_core::types::Transaction {
            id: id.to_string(),
            chain_id,
            hash: format!("0x{:064x}", 0u64),
            status,
            block_number: 1000,
            sender: "0x56fD3F2bEE130e9867942D0F463a16fBE49B8d81".to_string(),
            nonce: 0,
            deployments: dep_ids,
            operations: vec![],
            safe_context: None,
            environment: "testnet".to_string(),
            created_at: ts,
        }
    }

    #[test]
    fn clean_registry_returns_empty() {
        let dir = TempDir::new().unwrap();
        let mut registry = Registry::init(dir.path()).unwrap();

        // dep-1 references tx-1, both exist
        registry
            .insert_transaction(make_transaction(
                "tx-1",
                vec!["dep-1".to_string()],
                1,
                TransactionStatus::Executed,
            ))
            .unwrap();
        registry
            .insert_deployment(make_deployment("dep-1", "tx-1", 1))
            .unwrap();

        let candidates = find_prune_candidates(&registry, None, false);
        assert!(candidates.is_empty(), "expected no candidates in a clean registry");
    }

    #[test]
    fn detects_broken_transaction_ref() {
        let dir = TempDir::new().unwrap();
        let mut registry = Registry::init(dir.path()).unwrap();

        // dep-1 points to tx-MISSING which does not exist
        registry
            .insert_deployment(make_deployment("dep-1", "tx-MISSING", 1))
            .unwrap();

        let candidates = find_prune_candidates(&registry, None, false);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id, "dep-1");
        assert_eq!(candidates[0].kind, PruneCandidateKind::BrokenTransactionRef);
    }

    #[test]
    fn detects_broken_deployment_ref() {
        let dir = TempDir::new().unwrap();
        let mut registry = Registry::init(dir.path()).unwrap();

        // tx-1 lists dep-MISSING which doesn't exist
        registry
            .insert_transaction(make_transaction(
                "tx-1",
                vec!["dep-MISSING".to_string()],
                1,
                TransactionStatus::Executed,
            ))
            .unwrap();

        let candidates = find_prune_candidates(&registry, None, false);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id, "tx-1");
        assert_eq!(candidates[0].kind, PruneCandidateKind::BrokenDeploymentRef);
    }

    #[test]
    fn chain_id_filter_restricts_results() {
        let dir = TempDir::new().unwrap();
        let mut registry = Registry::init(dir.path()).unwrap();

        // Two broken deployments on different chains.
        registry
            .insert_deployment(make_deployment("dep-1", "tx-MISSING-1", 1))
            .unwrap();
        registry
            .insert_deployment(make_deployment("dep-2", "tx-MISSING-2", 42220))
            .unwrap();

        // Filter to chain 1 — only dep-1 should appear.
        let candidates = find_prune_candidates(&registry, Some(1), false);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id, "dep-1");
        assert_eq!(candidates[0].chain_id, Some(1));
    }

    #[test]
    fn prune_candidate_serializes_to_json() {
        let candidate = PruneCandidate {
            id: "dep-1".to_string(),
            kind: PruneCandidateKind::BrokenTransactionRef,
            reason: "test reason".to_string(),
            chain_id: Some(1),
        };
        let json = serde_json::to_string(&candidate).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["id"], "dep-1");
        assert_eq!(value["kind"], "BrokenTransactionRef");
        assert_eq!(value["chainId"], 1);
    }

    #[test]
    fn include_pending_flags_orphaned_transactions() {
        let dir = TempDir::new().unwrap();
        let mut registry = Registry::init(dir.path()).unwrap();

        // A pending transaction (no deployment refs)
        registry
            .insert_transaction(make_transaction("tx-pending", vec![], 1, TransactionStatus::Queued))
            .unwrap();

        let candidates_no_pending = find_prune_candidates(&registry, None, false);
        assert!(candidates_no_pending.is_empty());

        let candidates_with_pending = find_prune_candidates(&registry, None, true);
        assert_eq!(candidates_with_pending.len(), 1);
        assert_eq!(candidates_with_pending[0].kind, PruneCandidateKind::OrphanedPendingEntry);
    }
}
