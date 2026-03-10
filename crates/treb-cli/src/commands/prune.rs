//! `treb prune` command implementation.
//!
//! Scans the deployment registry for broken cross-references, reports prune
//! candidates, and (in destructive mode) removes them with a timestamped backup.

use std::{
    env, io,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, bail};
use clap::Args;
use serde::{Deserialize, Serialize};
use treb_registry::{REGISTRY_DIR, Registry, snapshot_registry};

use crate::{output, ui::emoji};

// ── Args ─────────────────────────────────────────────────────────────────────

/// Arguments for the `treb prune` command.
#[derive(Args, Debug)]
#[command(long_about = "Scan the deployment registry for broken cross-references \
(e.g., a deployment pointing to a missing transaction or vice versa) and remove \
them. A timestamped backup is created under `.treb/backups/` before any \
destructive operation. Use --dry-run to preview candidates without deleting.")]
pub struct PruneArgs {
    /// Simulate execution without making changes
    #[arg(long)]
    pub dry_run: bool,

    /// Include pending transactions in the prune scan
    #[arg(long)]
    pub include_pending: bool,

    /// Network name or chain ID
    #[arg(long)]
    pub network: Option<String>,

    /// Skip confirmation prompt (destructive mode only)
    #[arg(long, short = 'y')]
    pub yes: bool,

    /// Check on-chain bytecode for deployed contracts (requires --rpc-url)
    #[arg(long)]
    pub check_onchain: bool,

    /// RPC URL for on-chain bytecode checks
    #[arg(long)]
    pub rpc_url: Option<String>,

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    /// Contract address (for deployment candidates, used in human output).
    #[serde(skip)]
    pub address: Option<String>,
    /// Transaction status (for transaction candidates, used in human output).
    #[serde(skip)]
    pub status: Option<String>,
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
        if !dep.transaction_id.is_empty() && registry.get_transaction(&dep.transaction_id).is_none()
        {
            candidates.push(PruneCandidate {
                id: dep.id.clone(),
                kind: PruneCandidateKind::BrokenTransactionRef,
                reason: format!(
                    "deployment '{}' references missing transaction '{}'",
                    dep.id, dep.transaction_id
                ),
                chain_id: Some(dep.chain_id),
                address: Some(dep.address.clone()),
                status: None,
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
                    address: None,
                    status: Some(tx.status.to_string()),
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
                    address: None,
                    status: Some(tx.status.to_string()),
                });
            }
        }
    }

    candidates
}

// ── on-chain bytecode check ──────────────────────────────────────────────────

/// Check deployments on-chain via `eth_getCode` and return candidates for
/// contracts whose bytecode is empty (`0x` or `0x0`), indicating the contract
/// has been destroyed or was never deployed at that address.
///
/// RPC failures for individual addresses are reported as warnings on stderr
/// rather than fatal errors.
pub async fn find_onchain_prune_candidates(
    registry: &Registry,
    chain_id_filter: Option<u64>,
    rpc_url: &str,
) -> Vec<PruneCandidate> {
    let client = match reqwest::Client::builder().timeout(Duration::from_secs(30)).build() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Warning: failed to build HTTP client: {e}");
            return Vec::new();
        }
    };

    let mut candidates = Vec::new();

    for dep in registry.list_deployments() {
        // Apply chain filter.
        if let Some(filter_id) = chain_id_filter {
            if dep.chain_id != filter_id {
                continue;
            }
        }

        let address = &dep.address;
        if address.is_empty() {
            continue;
        }

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_getCode",
            "params": [address, "latest"],
            "id": 1
        });

        let resp = match client.post(rpc_url).json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Warning: eth_getCode failed for {} ({}): {}", dep.id, address, e);
                continue;
            }
        };

        let json: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "Warning: invalid response for eth_getCode on {} ({}): {}",
                    dep.id, address, e
                );
                continue;
            }
        };

        if let Some(error) = json.get("error") {
            eprintln!("Warning: RPC error for eth_getCode on {} ({}): {}", dep.id, address, error);
            continue;
        }

        let code = json.get("result").and_then(|v| v.as_str()).unwrap_or("0x");

        // Empty bytecode: "0x" or "0x0" means no contract at that address.
        if code == "0x" || code == "0x0" {
            candidates.push(PruneCandidate {
                id: dep.id.clone(),
                kind: PruneCandidateKind::DestroyedOnChain,
                reason: format!("deployment '{}' at {} has no on-chain bytecode", dep.id, address),
                chain_id: Some(dep.chain_id),
                address: Some(address.clone()),
                status: None,
            });
        }
    }

    candidates
}

// ── backup ────────────────────────────────────────────────────────────────────

/// Create a timestamped backup of the registry under `.treb/backups/prune-<ts>/`.
///
/// Returns the path to the backup directory.
pub fn backup_registry(project_root: &Path) -> anyhow::Result<std::path::PathBuf> {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
    let registry_dir = project_root.join(REGISTRY_DIR);
    let backup_dir = registry_dir.join(format!("backups/prune-{ts}"));
    snapshot_registry(&registry_dir, &backup_dir)
        .with_context(|| format!("failed to create prune backup at {}", backup_dir.display()))?;
    Ok(backup_dir)
}

fn targets_deployment(kind: &PruneCandidateKind) -> bool {
    matches!(kind, PruneCandidateKind::BrokenTransactionRef | PruneCandidateKind::DestroyedOnChain)
}

fn merge_onchain_candidates(
    candidates: &mut Vec<PruneCandidate>,
    onchain_candidates: Vec<PruneCandidate>,
) {
    for onchain in onchain_candidates {
        if let Some(existing) =
            candidates.iter_mut().find(|c| c.id == onchain.id && targets_deployment(&c.kind))
        {
            if onchain.kind == PruneCandidateKind::DestroyedOnChain {
                existing.kind = PruneCandidateKind::DestroyedOnChain;
                existing.reason = onchain.reason;
                existing.chain_id = onchain.chain_id;
            }
            continue;
        }
        candidates.push(onchain);
    }
}

fn validate_onchain_args(args: &PruneArgs) -> anyhow::Result<()> {
    if args.check_onchain && args.rpc_url.is_none() {
        bail!("--check-onchain requires --rpc-url <url>");
    }
    if args.check_onchain && args.network.is_none() {
        bail!("--check-onchain requires --network <chain-id>");
    }
    Ok(())
}

fn prune_confirmation_prompt() -> String {
    format!(
        "{}  Are you sure you want to prune these items? This cannot be undone. [y/N]: ",
        emoji::WARNING
    )
}

/// Print section listings for prune candidates grouped by type (deployment vs transaction).
fn print_candidate_sections(candidates: &[PruneCandidate]) {
    let dep_candidates: Vec<&PruneCandidate> =
        candidates.iter().filter(|c| targets_deployment(&c.kind)).collect();
    let tx_candidates: Vec<&PruneCandidate> =
        candidates.iter().filter(|c| !targets_deployment(&c.kind)).collect();

    if !dep_candidates.is_empty() {
        println!("Deployments ({}):", dep_candidates.len());
        for c in &dep_candidates {
            let addr = c.address.as_deref().unwrap_or("unknown");
            println!("  - {} at {} (reason: {})", c.id, addr, c.reason);
        }
        if !tx_candidates.is_empty() {
            println!();
        }
    }

    if !tx_candidates.is_empty() {
        println!("Transactions ({}):", tx_candidates.len());
        for c in &tx_candidates {
            let status = c.status.as_deref().unwrap_or("Unknown");
            // Convert status to title case (e.g., "EXECUTED" -> "Executed") to match Go output.
            let status_display =
                format!("{}{}", &status[..1].to_uppercase(), &status[1..].to_lowercase());
            println!("  - {} [{}] (reason: {})", c.id, status_display, c.reason);
        }
    }
}

// ── run ───────────────────────────────────────────────────────────────────────

/// Entry point for `treb prune`.
pub async fn run(args: PruneArgs) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;

    if !cwd.join("foundry.toml").exists() {
        bail!("no foundry.toml found in {}\n\nRun `forge init`, then `treb init`.", cwd.display());
    }
    if !cwd.join(".treb").exists() {
        bail!("project not initialized — .treb/ directory not found\n\nRun `treb init` first.");
    }

    // Resolve chain ID filter from --network argument.
    let chain_id_filter: Option<u64> = match &args.network {
        Some(s) => match s.parse::<u64>() {
            Ok(id) => Some(id),
            Err(_) => bail!("--network must be a chain ID (e.g. 1, 31337); got '{}'", s),
        },
        None => None,
    };

    validate_onchain_args(&args)?;

    let registry = Registry::open(&cwd).context("failed to open registry")?;
    let mut candidates = find_prune_candidates(&registry, chain_id_filter, args.include_pending);

    // On-chain bytecode verification (only when --check-onchain is set).
    if args.check_onchain {
        let rpc_url = args.rpc_url.as_deref().unwrap(); // safe: validated above
        let onchain_candidates =
            find_onchain_prune_candidates(&registry, chain_id_filter, rpc_url).await;

        // Merge on-chain candidates and preserve DestroyedOnChain classification
        // when cross-reference checks flagged the same deployment ID.
        merge_onchain_candidates(&mut candidates, onchain_candidates);
    }

    if candidates.is_empty() {
        if args.json {
            output::print_json(&serde_json::json!({ "candidates": [] }))?;
        } else {
            println!("{} All registry entries are valid. Nothing to prune.", emoji::CHECK);
        }
        return Ok(());
    }

    // Dry-run: just display candidates.
    if args.dry_run {
        if args.json {
            output::print_json(&candidates)?;
        } else {
            println!("{} Checking registry entries...", emoji::MAGNIFYING_GLASS);
            println!("\n{}  Found {} items to prune:\n", emoji::WASTEBASKET, candidates.len());
            print_candidate_sections(&candidates);
        }
        return Ok(());
    }

    // Destructive mode: confirm, backup, then remove.
    if !args.json {
        println!("{} Checking registry entries...", emoji::MAGNIFYING_GLASS);
        println!("\n{}  Found {} items to prune:\n", emoji::WASTEBASKET, candidates.len());
        print_candidate_sections(&candidates);
    }

    let skip_confirmation = args.yes || crate::ui::interactive::is_non_interactive(false);
    if skip_confirmation {
        if !args.json {
            println!("Running in non-interactive mode. Proceeding with prune...");
        }
    } else {
        if !args.json {
            println!();
        }

        let prompt = prune_confirmation_prompt();
        let confirmed = if args.json {
            let stdin = io::stdin();
            let mut stdin = stdin.lock();
            let stderr = io::stderr();
            let mut stderr = stderr.lock();
            crate::ui::prompt::confirm_raw(&mut stderr, &mut stdin, &prompt)
                .context("failed to read prune confirmation")?
        } else {
            let stdin = io::stdin();
            let mut stdin = stdin.lock();
            let stdout = io::stdout();
            let mut stdout = stdout.lock();
            crate::ui::prompt::confirm_raw(&mut stdout, &mut stdin, &prompt)
                .context("failed to read prune confirmation")?
        };

        if !confirmed {
            if !args.json {
                println!("{} Prune cancelled.", emoji::CROSS);
            }
            return Ok(());
        }
    }

    let _backup_path = backup_registry(&cwd)?;

    // Re-open registry mutably for removals.
    let mut registry = Registry::open(&cwd).context("failed to open registry")?;

    if !args.json {
        println!("\n{} Pruning registry entries...", emoji::WRENCH);
    }

    let mut removed: Vec<PruneCandidate> = Vec::new();
    for c in candidates {
        match c.kind {
            PruneCandidateKind::BrokenTransactionRef | PruneCandidateKind::DestroyedOnChain => {
                if registry.remove_deployment(&c.id).is_ok() {
                    removed.push(c);
                }
            }
            PruneCandidateKind::BrokenDeploymentRef | PruneCandidateKind::OrphanedPendingEntry => {
                if registry.remove_transaction(&c.id).is_ok() {
                    removed.push(c);
                }
            }
        }
    }

    if args.json {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct PruneResult<'a> {
            removed: &'a [PruneCandidate],
            backup_path: String,
        }
        output::print_json(&PruneResult {
            removed: &removed,
            backup_path: _backup_path.display().to_string(),
        })?;
    } else {
        println!("{} Successfully pruned {} items.", emoji::CHECK, removed.len());
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
        registry.insert_deployment(make_deployment("dep-1", "tx-1", 1)).unwrap();

        let candidates = find_prune_candidates(&registry, None, false);
        assert!(candidates.is_empty(), "expected no candidates in a clean registry");
    }

    #[test]
    fn detects_broken_transaction_ref() {
        let dir = TempDir::new().unwrap();
        let mut registry = Registry::init(dir.path()).unwrap();

        // dep-1 points to tx-MISSING which does not exist
        registry.insert_deployment(make_deployment("dep-1", "tx-MISSING", 1)).unwrap();

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
        registry.insert_deployment(make_deployment("dep-1", "tx-MISSING-1", 1)).unwrap();
        registry.insert_deployment(make_deployment("dep-2", "tx-MISSING-2", 42220)).unwrap();

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
            address: Some("0x1234".to_string()),
            status: Some("EXECUTED".to_string()),
        };
        let json = serde_json::to_string(&candidate).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["id"], "dep-1");
        assert_eq!(value["kind"], "BrokenTransactionRef");
        assert_eq!(value["chainId"], 1);
        assert!(value.get("address").is_none());
        assert!(value.get("status").is_none());
    }

    #[test]
    fn include_pending_flags_orphaned_transactions() {
        let dir = TempDir::new().unwrap();
        let mut registry = Registry::init(dir.path()).unwrap();

        // A pending transaction (no deployment refs)
        registry
            .insert_transaction(make_transaction(
                "tx-pending",
                vec![],
                1,
                TransactionStatus::Queued,
            ))
            .unwrap();

        let candidates_no_pending = find_prune_candidates(&registry, None, false);
        assert!(candidates_no_pending.is_empty());

        let candidates_with_pending = find_prune_candidates(&registry, None, true);
        assert_eq!(candidates_with_pending.len(), 1);
        assert_eq!(candidates_with_pending[0].kind, PruneCandidateKind::OrphanedPendingEntry);
    }

    #[test]
    fn prune_confirmation_prompt_matches_phase_11_contract() {
        assert_eq!(
            prune_confirmation_prompt(),
            "⚠️  Are you sure you want to prune these items? This cannot be undone. [y/N]: "
        );
    }

    #[test]
    fn destructive_prune_removes_candidates_and_leaves_clean_registry() {
        let dir = TempDir::new().unwrap();
        let mut registry = Registry::init(dir.path()).unwrap();

        // dep-1 points to tx-MISSING (broken transaction ref)
        registry.insert_deployment(make_deployment("dep-1", "tx-MISSING", 1)).unwrap();
        // tx-1 references dep-MISSING (broken deployment ref)
        registry
            .insert_transaction(make_transaction(
                "tx-1",
                vec!["dep-MISSING".to_string()],
                1,
                TransactionStatus::Executed,
            ))
            .unwrap();

        let candidates = find_prune_candidates(&registry, None, false);
        assert_eq!(candidates.len(), 2, "should have 2 prune candidates");

        // Simulate destructive prune: remove each candidate.
        for c in candidates {
            match c.kind {
                PruneCandidateKind::BrokenTransactionRef | PruneCandidateKind::DestroyedOnChain => {
                    registry.remove_deployment(&c.id).unwrap();
                }
                PruneCandidateKind::BrokenDeploymentRef
                | PruneCandidateKind::OrphanedPendingEntry => {
                    registry.remove_transaction(&c.id).unwrap();
                }
            }
        }

        // After pruning, no candidates should remain.
        let after_candidates = find_prune_candidates(&registry, None, false);
        assert!(after_candidates.is_empty(), "registry should be clean after destructive prune");
    }

    #[test]
    fn backup_registry_creates_backup_dir() {
        let dir = TempDir::new().unwrap();
        // Create a foundry.toml and .treb dir so backup_registry can run.
        std::fs::write(dir.path().join("foundry.toml"), "").unwrap();
        let mut registry = Registry::init(dir.path()).unwrap();
        // Insert something so registry files exist.
        registry.insert_deployment(make_deployment("dep-1", "tx-1", 1)).unwrap();

        let backup_path = backup_registry(dir.path()).unwrap();
        assert!(
            backup_path.exists(),
            "backup directory should be created: {}",
            backup_path.display()
        );
        // Backup dir should be inside .treb/backups/
        assert!(
            backup_path.starts_with(dir.path().join(".treb/backups")),
            "backup should be inside .treb/backups/"
        );
    }

    #[test]
    fn dry_run_does_not_modify_registry() {
        let dir = TempDir::new().unwrap();
        let mut registry = Registry::init(dir.path()).unwrap();

        registry.insert_deployment(make_deployment("dep-1", "tx-MISSING", 1)).unwrap();

        // dry-run: just find candidates, do not remove
        let candidates = find_prune_candidates(&registry, None, false);
        assert_eq!(candidates.len(), 1);

        // Registry should still have dep-1.
        assert!(
            registry.get_deployment("dep-1").is_some(),
            "dep-1 should still exist after dry-run (no removal performed)"
        );
    }

    #[test]
    fn merge_onchain_candidates_upgrades_existing_kind_for_same_id() {
        let mut candidates = vec![PruneCandidate {
            id: "dep-1".to_string(),
            kind: PruneCandidateKind::BrokenTransactionRef,
            reason: "broken tx ref".to_string(),
            chain_id: Some(1),
            address: Some("0x1234".to_string()),
            status: None,
        }];
        let onchain_candidates = vec![PruneCandidate {
            id: "dep-1".to_string(),
            kind: PruneCandidateKind::DestroyedOnChain,
            reason: "no bytecode at address".to_string(),
            chain_id: Some(1),
            address: Some("0x1234".to_string()),
            status: None,
        }];

        merge_onchain_candidates(&mut candidates, onchain_candidates);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id, "dep-1");
        assert_eq!(candidates[0].kind, PruneCandidateKind::DestroyedOnChain);
        assert_eq!(candidates[0].reason, "no bytecode at address");
    }

    #[test]
    fn merge_onchain_candidates_does_not_rewrite_transaction_candidate_with_same_id() {
        let mut candidates = vec![PruneCandidate {
            id: "shared-id".to_string(),
            kind: PruneCandidateKind::BrokenDeploymentRef,
            reason: "tx references missing deployment".to_string(),
            chain_id: Some(1),
            address: None,
            status: Some("EXECUTED".to_string()),
        }];
        let onchain_candidates = vec![PruneCandidate {
            id: "shared-id".to_string(),
            kind: PruneCandidateKind::DestroyedOnChain,
            reason: "deployment has no bytecode".to_string(),
            chain_id: Some(1),
            address: Some("0x5678".to_string()),
            status: None,
        }];

        merge_onchain_candidates(&mut candidates, onchain_candidates);

        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().any(|c| c.kind == PruneCandidateKind::BrokenDeploymentRef));
        assert!(candidates.iter().any(|c| c.kind == PruneCandidateKind::DestroyedOnChain));
    }

    #[test]
    fn validate_onchain_args_requires_network_when_checking_onchain() {
        let args = PruneArgs {
            dry_run: true,
            include_pending: false,
            network: None,
            yes: false,
            check_onchain: true,
            rpc_url: Some("http://localhost:8545".to_string()),
            json: false,
        };

        let err = validate_onchain_args(&args).unwrap_err().to_string();
        assert!(err.contains("--check-onchain requires --network <chain-id>"));
    }
}
