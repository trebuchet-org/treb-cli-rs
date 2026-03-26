//! `treb registry migrate` command implementation.
//!
//! Migrates legacy `.treb` registry state into canonical `deployments/`
//! storage, linking deployments directly to immutable broadcast or queued
//! artifacts.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, bail};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use treb_core::types::{
    Deployment, ExecutionKind, ExecutionRef, ExecutionStatus, GovernorProposal, ProposalStatus,
    ProxyUpgrade, SafeTransaction, Transaction, TransactionStatus,
};
use treb_forge::pipeline::broadcast_writer::{
    QueuedGovernorProposal, QueuedOperations, QueuedSafeProposal, queued_path_from,
    timestamped_path_from_latest,
};
use treb_registry::{
    AddressbookStore, DeploymentStore, QueuedIndex, QueuedIndexEntry, QueuedIndexStore,
    SAFE_TXS_FILE, SolidityRegistryStore, TRANSACTIONS_FILE, deployments_dir, read_versioned_file,
    registry_dir,
};

const LEGACY_SAFE_TXS_FILE: &str = "safe_txs.json";
const LEGACY_GOVERNOR_FILE: &str = "governor_proposals.json";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MigrateOutput {
    dry_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    backup_path: Option<String>,
    deployments: usize,
    direct_refs: usize,
    queued_refs: usize,
    external_refs: usize,
    orphaned_refs: usize,
    queued_entries: usize,
    queued_artifacts: usize,
    promoted_broadcast_artifacts: usize,
    promoted_queued_artifacts: usize,
    orphaned_deployments: Vec<String>,
}

#[derive(Debug, Default)]
struct MigrationPlan {
    deployments: HashMap<String, Deployment>,
    addressbook: HashMap<String, HashMap<String, String>>,
    queued_index: QueuedIndex,
    queued_artifacts: BTreeMap<PathBuf, QueuedOperations>,
    copied_artifacts: BTreeMap<PathBuf, PathBuf>,
    report: MigrationReport,
}

#[derive(Debug, Default)]
struct MigrationReport {
    direct_refs: usize,
    queued_refs: usize,
    external_refs: usize,
    orphaned_refs: usize,
    orphaned_deployments: Vec<String>,
    queued_entries: usize,
}

#[derive(Clone, Debug)]
struct DirectArtifactMatch {
    canonical_path: PathBuf,
    copy_from: Option<PathBuf>,
    script_tx_index: usize,
}

#[derive(Clone, Debug)]
struct ExistingQueuedArtifact {
    canonical_path: PathBuf,
    copy_from: Option<PathBuf>,
    operations: QueuedOperations,
}

#[derive(Default)]
struct ArtifactScan {
    direct_by_tx_hash: HashMap<String, DirectArtifactMatch>,
    queued_by_safe_hash: HashMap<String, ExistingQueuedArtifact>,
    queued_by_proposal_id: HashMap<String, ExistingQueuedArtifact>,
    queued_by_propose_safe_hash: HashMap<String, ExistingQueuedArtifact>,
    queued_by_canonical_path: HashMap<PathBuf, ExistingQueuedArtifact>,
}

#[derive(Clone, Debug)]
struct QueuedExecutionCandidate {
    artifact_file: String,
    kind: ExecutionKind,
    status: ExecutionStatus,
    tx_hash: Option<String>,
    safe_tx_hash: Option<String>,
    proposal_id: Option<String>,
    propose_safe_tx_hash: Option<String>,
    script_tx_index: Option<usize>,
}

#[derive(Clone, Debug)]
struct QueuedWorkItem {
    candidate: QueuedExecutionCandidate,
}

#[derive(Default)]
struct LegacyState {
    deployments: HashMap<String, Deployment>,
    transactions: HashMap<String, Transaction>,
    safe_transactions: HashMap<String, SafeTransaction>,
    governor_proposals: HashMap<String, GovernorProposal>,
    addressbook: HashMap<String, HashMap<String, String>>,
}

impl LegacyState {
    fn is_empty(&self) -> bool {
        self.deployments.is_empty()
            && self.transactions.is_empty()
            && self.safe_transactions.is_empty()
            && self.governor_proposals.is_empty()
            && self.addressbook.is_empty()
    }
}

pub fn run(write: bool, force: bool, json: bool) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    if !cwd.join("foundry.toml").exists() {
        bail!("no foundry.toml found in {}\n\nRun `forge init`, then `treb init`.", cwd.display());
    }

    let legacy = load_legacy_state(&cwd)?;
    if legacy.is_empty() {
        bail!("no legacy registry state found under .treb/");
    }

    if canonical_registry_exists(&cwd)? && !force {
        bail!(
            "canonical deployment files already exist under deployments/\n\n\
             Re-run with --force to overwrite them."
        );
    }

    let scan = scan_broadcast_artifacts(&cwd)?;
    let plan = build_migration_plan(&cwd, legacy, scan)?;

    if !write {
        render_output(plan.report_output(true, None), json)?;
        return Ok(());
    }

    let backup_dir = backup_existing_state(&cwd)?;
    write_migration_plan(&cwd, &plan)?;
    render_output(plan.report_output(false, Some(backup_dir)), json)?;
    Ok(())
}

fn load_legacy_state(project_root: &Path) -> anyhow::Result<LegacyState> {
    let registry = registry_dir(project_root);
    Ok(LegacyState {
        deployments: read_optional_store(&[
            registry.join(treb_registry::DEPLOYMENTS_FILE),
            project_root.join(treb_registry::DEPLOYMENTS_FILE),
        ])?,
        transactions: read_optional_store(&[registry.join(TRANSACTIONS_FILE)])?,
        safe_transactions: read_optional_store(&[
            registry.join(SAFE_TXS_FILE),
            registry.join(LEGACY_SAFE_TXS_FILE),
        ])?,
        governor_proposals: read_optional_store(&[
            registry.join(treb_registry::GOVERNOR_PROPOSALS_FILE),
            registry.join(LEGACY_GOVERNOR_FILE),
        ])?,
        addressbook: read_optional_store(&[
            registry.join(treb_registry::ADDRESSBOOK_FILE),
            project_root.join(treb_registry::ADDRESSBOOK_FILE),
        ])?,
    })
}

fn read_optional_store<T>(paths: &[PathBuf]) -> anyhow::Result<T>
where
    T: Default + DeserializeOwned,
{
    for path in paths {
        if path.exists() {
            return read_versioned_file(path).map_err(|err| anyhow::anyhow!("{err}"));
        }
    }
    Ok(T::default())
}

fn canonical_registry_exists(project_root: &Path) -> anyhow::Result<bool> {
    let deployments_root = deployments_dir(project_root);
    if !deployments_root.exists() {
        return Ok(false);
    }

    let mut stack = vec![deployments_root];
    while let Some(dir) = stack.pop() {
        for entry in
            fs::read_dir(&dir).with_context(|| format!("failed to read {}", dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

fn scan_broadcast_artifacts(project_root: &Path) -> anyhow::Result<ArtifactScan> {
    let broadcast_root = project_root.join("broadcast");
    if !broadcast_root.exists() {
        return Ok(ArtifactScan::default());
    }

    let mut scan = ArtifactScan::default();
    let mut stack = vec![broadcast_root];
    while let Some(dir) = stack.pop() {
        for entry in
            fs::read_dir(&dir).with_context(|| format!("failed to read {}", dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                stack.push(path);
                continue;
            }

            let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
            if !file_name.ends_with(".json") {
                continue;
            }

            if file_name.ends_with(".queued.json") || file_name.ends_with(".deferred.json") {
                scan_queued_artifact(&mut scan, project_root, &path)?;
                continue;
            }

            if file_name.starts_with("run-") {
                scan_direct_artifact(&mut scan, project_root, &path)?;
            }
        }
    }

    Ok(scan)
}

fn scan_direct_artifact(
    scan: &mut ArtifactScan,
    project_root: &Path,
    path: &Path,
) -> anyhow::Result<()> {
    let value: Value = treb_registry::io::read_json_file(path)
        .with_context(|| format!("failed to read broadcast artifact {}", path.display()))?;
    let canonical_path = canonical_broadcast_path(path, artifact_timestamp(&value));
    let copy_from = (canonical_path != path).then(|| path.to_path_buf());
    let relative = PathBuf::from(relative_path(project_root, &canonical_path));

    let Some(transactions) = value.get("transactions").and_then(Value::as_array) else {
        return Ok(());
    };

    for (index, tx) in transactions.iter().enumerate() {
        let Some(hash) = tx.get("hash").and_then(Value::as_str).filter(|hash| !hash.is_empty())
        else {
            continue;
        };

        let candidate = DirectArtifactMatch {
            canonical_path: relative.clone(),
            copy_from: copy_from.clone(),
            script_tx_index: index,
        };
        upsert_direct_match(&mut scan.direct_by_tx_hash, hash.to_string(), candidate);
    }

    Ok(())
}

fn scan_queued_artifact(
    scan: &mut ArtifactScan,
    project_root: &Path,
    path: &Path,
) -> anyhow::Result<()> {
    let value: Value = treb_registry::io::read_json_file(path)
        .with_context(|| format!("failed to read queued artifact {}", path.display()))?;
    let operations: QueuedOperations = serde_json::from_value(value.clone())
        .with_context(|| format!("failed to parse queued artifact {}", path.display()))?;
    let canonical_path = canonical_queued_path(path, artifact_timestamp(&value));
    let copy_from = (canonical_path != path).then(|| path.to_path_buf());

    let artifact = ExistingQueuedArtifact {
        canonical_path: PathBuf::from(relative_path(project_root, &canonical_path)),
        copy_from,
        operations: operations.clone(),
    };
    upsert_queued_by_path(
        &mut scan.queued_by_canonical_path,
        canonical_path.clone(),
        artifact.clone(),
    );

    for proposal in &operations.safe_proposals {
        upsert_queued_match(
            &mut scan.queued_by_safe_hash,
            proposal.safe_tx_hash.clone(),
            artifact.clone(),
        );
    }
    for proposal in &operations.governor_proposals {
        if !proposal.proposal_id.is_empty() {
            upsert_queued_match(
                &mut scan.queued_by_proposal_id,
                proposal.proposal_id.clone(),
                artifact.clone(),
            );
        }
        if let Some(hash) = proposal.propose_safe_tx_hash.as_ref().filter(|hash| !hash.is_empty()) {
            upsert_queued_match(
                &mut scan.queued_by_propose_safe_hash,
                hash.clone(),
                artifact.clone(),
            );
        }
    }

    Ok(())
}

fn upsert_direct_match(
    map: &mut HashMap<String, DirectArtifactMatch>,
    tx_hash: String,
    candidate: DirectArtifactMatch,
) {
    match map.get(&tx_hash) {
        Some(existing) if existing.copy_from.is_none() && candidate.copy_from.is_some() => {}
        _ => {
            map.insert(tx_hash, candidate);
        }
    }
}

fn upsert_queued_match(
    map: &mut HashMap<String, ExistingQueuedArtifact>,
    key: String,
    candidate: ExistingQueuedArtifact,
) {
    match map.get(&key) {
        Some(existing) if existing.copy_from.is_none() && candidate.copy_from.is_some() => {}
        _ => {
            map.insert(key, candidate);
        }
    }
}

fn upsert_queued_by_path(
    map: &mut HashMap<PathBuf, ExistingQueuedArtifact>,
    key: PathBuf,
    candidate: ExistingQueuedArtifact,
) {
    match map.get(&key) {
        Some(existing) if existing.copy_from.is_none() && candidate.copy_from.is_some() => {}
        _ => {
            map.insert(key, candidate);
        }
    }
}

fn artifact_timestamp(value: &Value) -> Option<u128> {
    value.get("timestamp").and_then(Value::as_u64).map(u128::from)
}

fn canonical_broadcast_path(path: &Path, timestamp: Option<u128>) -> PathBuf {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return path.to_path_buf();
    };
    if file_name.contains("-latest.")
        && let Some(timestamp) = timestamp
    {
        return timestamped_path_from_latest(path, timestamp);
    }
    path.to_path_buf()
}

fn canonical_queued_path(path: &Path, timestamp: Option<u128>) -> PathBuf {
    let mut canonical = canonical_broadcast_path(path, timestamp);
    if canonical
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".deferred.json"))
    {
        let file_name = canonical.file_name().and_then(|name| name.to_str()).unwrap_or_default();
        canonical = canonical.with_file_name(file_name.replace(".deferred.json", ".queued.json"));
    }
    canonical
}

fn relative_path(project_root: &Path, path: &Path) -> String {
    path.strip_prefix(project_root).unwrap_or(path).to_string_lossy().into_owned()
}

fn build_migration_plan(
    project_root: &Path,
    legacy: LegacyState,
    scan: ArtifactScan,
) -> anyhow::Result<MigrationPlan> {
    let mut plan = MigrationPlan { addressbook: legacy.addressbook.clone(), ..Default::default() };
    let tx_by_deployment = index_transactions_by_deployment(&legacy.transactions);
    let mut queued_by_tx_id = HashMap::new();
    let mut queued_items = Vec::new();

    synthesize_queued_from_safe_transactions(
        project_root,
        &legacy.transactions,
        &legacy.safe_transactions,
        &scan,
        &mut plan,
        &mut queued_by_tx_id,
        &mut queued_items,
    )?;

    synthesize_queued_from_governor_proposals(
        project_root,
        &legacy.transactions,
        &legacy.safe_transactions,
        &legacy.governor_proposals,
        &scan,
        &mut plan,
        &mut queued_by_tx_id,
        &mut queued_items,
    )?;

    for mut deployment in legacy.deployments.into_values() {
        let direct_tx = legacy.transactions.get(&deployment.transaction_id).or_else(|| {
            tx_by_deployment.get(&deployment.id).and_then(|tx_id| legacy.transactions.get(tx_id))
        });

        let queued_candidate = direct_tx
            .and_then(|tx| queued_by_tx_id.get(&tx.id))
            .cloned()
            .or_else(|| queued_by_tx_id.get(&deployment.transaction_id).cloned());

        let execution = if let Some(candidate) = queued_candidate {
            plan.report.queued_refs += 1;
            Some(execution_ref_from_candidate(candidate))
        } else {
            resolve_direct_execution(
                project_root,
                direct_tx,
                &scan,
                &mut plan.copied_artifacts,
                &mut plan.report,
            )
        };

        if execution.is_none() {
            plan.report.orphaned_deployments.push(deployment.id.clone());
        }

        deployment.execution = execution;
        deployment.transaction_id.clear();
        migrate_proxy_history(
            project_root,
            &mut deployment,
            &legacy.transactions,
            &scan,
            &mut plan.copied_artifacts,
        );
        plan.deployments.insert(deployment.id.clone(), deployment);
    }

    plan.queued_index = build_queued_index(&plan.deployments, &queued_items);
    plan.report.queued_entries = plan.queued_index.entries.len();
    Ok(plan)
}

fn index_transactions_by_deployment(
    transactions: &HashMap<String, Transaction>,
) -> HashMap<String, String> {
    let mut index = HashMap::new();
    for transaction in transactions.values() {
        for deployment_id in &transaction.deployments {
            index.entry(deployment_id.clone()).or_insert_with(|| transaction.id.clone());
        }
    }
    index
}

fn synthesize_queued_from_safe_transactions(
    project_root: &Path,
    transactions: &HashMap<String, Transaction>,
    safe_transactions: &HashMap<String, SafeTransaction>,
    scan: &ArtifactScan,
    plan: &mut MigrationPlan,
    queued_by_tx_id: &mut HashMap<String, QueuedExecutionCandidate>,
    queued_items: &mut Vec<QueuedWorkItem>,
) -> anyhow::Result<()> {
    for safe_tx in safe_transactions.values() {
        let target = resolve_queued_artifact_for_safe(project_root, transactions, safe_tx, scan);
        if let Some(source) = target.copy_from.as_ref() {
            plan.copied_artifacts
                .entry(source.clone())
                .or_insert_with(|| target.canonical_path.clone());
        }
        let operations = plan
            .queued_artifacts
            .entry(target.canonical_path.clone())
            .or_insert_with(|| queued_seed_for_path(safe_tx.chain_id, &target, scan));

        if !operations
            .safe_proposals
            .iter()
            .any(|proposal| proposal.safe_tx_hash == safe_tx.safe_tx_hash)
        {
            operations.safe_proposals.push(QueuedSafeProposal {
                safe_tx_hash: safe_tx.safe_tx_hash.clone(),
                safe_address: safe_tx.safe_address.clone(),
                nonce: safe_tx.nonce,
                chain_id: safe_tx.chain_id,
                sender_role: String::new(),
                transaction_ids: safe_tx.transaction_ids.clone(),
                status: safe_status_string(&safe_tx.status),
                execution_tx_hash: non_empty(&safe_tx.execution_tx_hash),
            });
        }

        let candidate = QueuedExecutionCandidate {
            artifact_file: relative_path(project_root, &target.canonical_path),
            kind: ExecutionKind::SafeProposal,
            status: queued_execution_status_from_safe(&safe_tx.status),
            tx_hash: None,
            safe_tx_hash: Some(safe_tx.safe_tx_hash.clone()),
            proposal_id: None,
            propose_safe_tx_hash: None,
            script_tx_index: first_script_index(&safe_tx.transaction_ids, transactions, scan),
        };

        for tx_id in &safe_tx.transaction_ids {
            apply_queued_candidate(queued_by_tx_id, tx_id.clone(), candidate.clone());
        }
        queued_items.push(QueuedWorkItem { candidate });
    }

    Ok(())
}

fn synthesize_queued_from_governor_proposals(
    project_root: &Path,
    transactions: &HashMap<String, Transaction>,
    safe_transactions: &HashMap<String, SafeTransaction>,
    governor_proposals: &HashMap<String, GovernorProposal>,
    scan: &ArtifactScan,
    plan: &mut MigrationPlan,
    queued_by_tx_id: &mut HashMap<String, QueuedExecutionCandidate>,
    queued_items: &mut Vec<QueuedWorkItem>,
) -> anyhow::Result<()> {
    let safe_by_tx_ids = build_safe_lookup_by_tx_ids(safe_transactions);

    for proposal in governor_proposals.values() {
        let linked_safe = safe_by_tx_ids.get(&tx_id_key(&proposal.transaction_ids)).copied();
        let target = resolve_queued_artifact_for_governor(
            project_root,
            transactions,
            proposal,
            linked_safe,
            scan,
        );
        if let Some(source) = target.copy_from.as_ref() {
            plan.copied_artifacts
                .entry(source.clone())
                .or_insert_with(|| target.canonical_path.clone());
        }
        let operations = plan
            .queued_artifacts
            .entry(target.canonical_path.clone())
            .or_insert_with(|| queued_seed_for_path(proposal.chain_id, &target, scan));

        let propose_safe_tx_hash =
            linked_safe.map(|safe_tx| safe_tx.safe_tx_hash.clone()).filter(|hash| !hash.is_empty());

        if !operations.governor_proposals.iter().any(|existing| {
            same_governor_identity(existing, &proposal.proposal_id, propose_safe_tx_hash.as_deref())
        }) {
            operations.governor_proposals.push(QueuedGovernorProposal {
                proposal_id: proposal.proposal_id.clone(),
                governor_address: proposal.governor_address.clone(),
                sender_role: String::new(),
                transaction_ids: proposal.transaction_ids.clone(),
                status: governor_status_string(&proposal.status),
                propose_tx_hash: non_empty(&proposal.execution_tx_hash),
                propose_safe_tx_hash: propose_safe_tx_hash.clone(),
            });
        }

        let candidate = QueuedExecutionCandidate {
            artifact_file: relative_path(project_root, &target.canonical_path),
            kind: if propose_safe_tx_hash.is_some() {
                ExecutionKind::GovernorProposalViaSafe
            } else {
                ExecutionKind::GovernorProposal
            },
            status: queued_execution_status_from_governor(&proposal.status),
            tx_hash: None,
            safe_tx_hash: None,
            proposal_id: non_empty(&proposal.proposal_id),
            propose_safe_tx_hash,
            script_tx_index: first_script_index(&proposal.transaction_ids, transactions, scan),
        };

        for tx_id in &proposal.transaction_ids {
            apply_queued_candidate(queued_by_tx_id, tx_id.clone(), candidate.clone());
        }
        queued_items.push(QueuedWorkItem { candidate });
    }

    Ok(())
}

fn build_safe_lookup_by_tx_ids<'a>(
    safe_transactions: &'a HashMap<String, SafeTransaction>,
) -> HashMap<String, &'a SafeTransaction> {
    let mut lookup = HashMap::new();
    for safe_tx in safe_transactions.values() {
        lookup.insert(tx_id_key(&safe_tx.transaction_ids), safe_tx);
    }
    lookup
}

fn tx_id_key(tx_ids: &[String]) -> String {
    tx_ids.join("\u{1f}")
}

fn same_governor_identity(
    existing: &QueuedGovernorProposal,
    proposal_id: &str,
    propose_safe_tx_hash: Option<&str>,
) -> bool {
    (!proposal_id.is_empty() && existing.proposal_id == proposal_id)
        || propose_safe_tx_hash
            .zip(existing.propose_safe_tx_hash.as_deref())
            .is_some_and(|(left, right)| left == right)
}

#[derive(Clone)]
struct QueuedArtifactTarget {
    canonical_path: PathBuf,
    copy_from: Option<PathBuf>,
}

fn resolve_queued_artifact_for_safe(
    project_root: &Path,
    transactions: &HashMap<String, Transaction>,
    safe_tx: &SafeTransaction,
    scan: &ArtifactScan,
) -> QueuedArtifactTarget {
    if let Some(existing) = scan.queued_by_safe_hash.get(&safe_tx.safe_tx_hash) {
        return QueuedArtifactTarget {
            canonical_path: project_root.join(&existing.canonical_path),
            copy_from: existing.copy_from.clone(),
        };
    }

    if let Some(broadcast_path) =
        resolve_canonical_broadcast_path(project_root, transactions, &safe_tx.transaction_ids, scan)
    {
        return QueuedArtifactTarget {
            canonical_path: queued_path_from(&broadcast_path),
            copy_from: None,
        };
    }

    QueuedArtifactTarget {
        canonical_path: synthetic_queued_path(
            project_root,
            safe_tx.chain_id,
            &safe_tx.safe_tx_hash,
        ),
        copy_from: None,
    }
}

fn resolve_queued_artifact_for_governor(
    project_root: &Path,
    transactions: &HashMap<String, Transaction>,
    proposal: &GovernorProposal,
    linked_safe: Option<&SafeTransaction>,
    scan: &ArtifactScan,
) -> QueuedArtifactTarget {
    if !proposal.proposal_id.is_empty()
        && let Some(existing) = scan.queued_by_proposal_id.get(&proposal.proposal_id)
    {
        return QueuedArtifactTarget {
            canonical_path: project_root.join(&existing.canonical_path),
            copy_from: existing.copy_from.clone(),
        };
    }

    if let Some(safe_tx) = linked_safe
        && let Some(existing) = scan.queued_by_propose_safe_hash.get(&safe_tx.safe_tx_hash)
    {
        return QueuedArtifactTarget {
            canonical_path: project_root.join(&existing.canonical_path),
            copy_from: existing.copy_from.clone(),
        };
    }

    if let Some(broadcast_path) = resolve_canonical_broadcast_path(
        project_root,
        transactions,
        &proposal.transaction_ids,
        scan,
    ) {
        return QueuedArtifactTarget {
            canonical_path: queued_path_from(&broadcast_path),
            copy_from: None,
        };
    }

    let seed =
        if !proposal.proposal_id.is_empty() { proposal.proposal_id.as_str() } else { "governor" };
    QueuedArtifactTarget {
        canonical_path: synthetic_queued_path(project_root, proposal.chain_id, seed),
        copy_from: None,
    }
}

fn resolve_canonical_broadcast_path(
    project_root: &Path,
    transactions: &HashMap<String, Transaction>,
    tx_ids: &[String],
    scan: &ArtifactScan,
) -> Option<PathBuf> {
    for tx_id in tx_ids {
        let tx = transactions.get(tx_id)?;
        if let Some(direct) = scan.direct_by_tx_hash.get(&tx.hash) {
            return Some(project_root.join(&direct.canonical_path));
        }
        if let Some(path) = tx.broadcast_file.as_ref().filter(|path| !path.is_empty()) {
            return Some(project_root.join(path));
        }
    }
    None
}

fn synthetic_queued_path(project_root: &Path, chain_id: u64, seed: &str) -> PathBuf {
    let timestamp =
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let stable = seed.trim_start_matches("0x");
    project_root
        .join("broadcast")
        .join("_migrated")
        .join(chain_id.to_string())
        .join(format!("run-{timestamp}-{stable}.queued.json"))
}

fn queued_seed_for_path(
    chain_id: u64,
    target: &QueuedArtifactTarget,
    scan: &ArtifactScan,
) -> QueuedOperations {
    scan.queued_by_canonical_path
        .get(&target.canonical_path)
        .map(|existing| existing.operations.clone())
        .unwrap_or(QueuedOperations {
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
            chain: chain_id,
            commit: None,
            safe_proposals: Vec::new(),
            governor_proposals: Vec::new(),
        })
}

fn first_script_index(
    tx_ids: &[String],
    transactions: &HashMap<String, Transaction>,
    scan: &ArtifactScan,
) -> Option<usize> {
    tx_ids.iter().find_map(|tx_id| {
        let tx = transactions.get(tx_id)?;
        let direct = scan.direct_by_tx_hash.get(&tx.hash)?;
        Some(direct.script_tx_index)
    })
}

fn safe_status_string(status: &TransactionStatus) -> String {
    status.to_string().to_lowercase()
}

fn governor_status_string(status: &ProposalStatus) -> String {
    status.to_string()
}

fn queued_execution_status_from_safe(status: &TransactionStatus) -> ExecutionStatus {
    match status {
        TransactionStatus::Executed => ExecutionStatus::Executed,
        TransactionStatus::Failed => ExecutionStatus::Orphaned,
        TransactionStatus::Queued | TransactionStatus::Simulated => ExecutionStatus::Queued,
    }
}

fn queued_execution_status_from_governor(status: &ProposalStatus) -> ExecutionStatus {
    match status {
        ProposalStatus::Executed => ExecutionStatus::Executed,
        ProposalStatus::Canceled | ProposalStatus::Defeated => ExecutionStatus::Orphaned,
        ProposalStatus::Pending
        | ProposalStatus::Active
        | ProposalStatus::Succeeded
        | ProposalStatus::Queued => ExecutionStatus::Queued,
    }
}

fn apply_queued_candidate(
    queued_by_tx_id: &mut HashMap<String, QueuedExecutionCandidate>,
    tx_id: String,
    candidate: QueuedExecutionCandidate,
) {
    match queued_by_tx_id.get(&tx_id) {
        Some(existing)
            if queued_candidate_priority(existing) >= queued_candidate_priority(&candidate) => {}
        _ => {
            queued_by_tx_id.insert(tx_id, candidate);
        }
    }
}

fn queued_candidate_priority(candidate: &QueuedExecutionCandidate) -> u8 {
    match candidate.kind {
        ExecutionKind::GovernorProposalViaSafe => 3,
        ExecutionKind::GovernorProposal => 2,
        ExecutionKind::SafeProposal => 1,
        ExecutionKind::Tx | ExecutionKind::ExternalTx => 0,
    }
}

fn execution_ref_from_candidate(candidate: QueuedExecutionCandidate) -> ExecutionRef {
    ExecutionRef {
        status: candidate.status,
        kind: candidate.kind,
        artifact_file: candidate.artifact_file,
        tx_hash: candidate.tx_hash,
        safe_tx_hash: candidate.safe_tx_hash,
        proposal_id: candidate.proposal_id,
        propose_safe_tx_hash: candidate.propose_safe_tx_hash,
        script_tx_index: candidate.script_tx_index,
    }
}

fn resolve_direct_execution(
    project_root: &Path,
    transaction: Option<&Transaction>,
    scan: &ArtifactScan,
    copied_artifacts: &mut BTreeMap<PathBuf, PathBuf>,
    report: &mut MigrationReport,
) -> Option<ExecutionRef> {
    let Some(transaction) = transaction else {
        report.orphaned_refs += 1;
        return None;
    };

    if let Some(direct) = scan.direct_by_tx_hash.get(&transaction.hash) {
        let canonical_path = project_root.join(&direct.canonical_path);
        if let Some(source) = direct.copy_from.as_ref() {
            copied_artifacts.entry(source.clone()).or_insert_with(|| canonical_path.clone());
        }
        let status = match transaction.status {
            TransactionStatus::Executed => ExecutionStatus::Executed,
            TransactionStatus::Queued | TransactionStatus::Simulated => ExecutionStatus::Broadcast,
            TransactionStatus::Failed => ExecutionStatus::Orphaned,
        };
        match status {
            ExecutionStatus::Executed | ExecutionStatus::Broadcast => report.direct_refs += 1,
            ExecutionStatus::Orphaned => report.orphaned_refs += 1,
            ExecutionStatus::Queued | ExecutionStatus::External => {}
        }
        return Some(ExecutionRef {
            status,
            kind: ExecutionKind::Tx,
            artifact_file: relative_path(project_root, &canonical_path),
            tx_hash: non_empty(&transaction.hash),
            safe_tx_hash: None,
            proposal_id: None,
            propose_safe_tx_hash: None,
            script_tx_index: Some(direct.script_tx_index),
        });
    }

    if !transaction.hash.is_empty() {
        report.external_refs += 1;
        return Some(ExecutionRef {
            status: ExecutionStatus::External,
            kind: ExecutionKind::ExternalTx,
            artifact_file: String::new(),
            tx_hash: Some(transaction.hash.clone()),
            safe_tx_hash: None,
            proposal_id: None,
            propose_safe_tx_hash: None,
            script_tx_index: None,
        });
    }

    report.orphaned_refs += 1;
    None
}

fn migrate_proxy_history(
    project_root: &Path,
    deployment: &mut Deployment,
    transactions: &HashMap<String, Transaction>,
    scan: &ArtifactScan,
    copied_artifacts: &mut BTreeMap<PathBuf, PathBuf>,
) {
    let Some(proxy_info) = deployment.proxy_info.as_mut() else { return };
    for upgrade in &mut proxy_info.history {
        migrate_proxy_upgrade(project_root, upgrade, transactions, scan, copied_artifacts);
    }
}

fn migrate_proxy_upgrade(
    project_root: &Path,
    upgrade: &mut ProxyUpgrade,
    transactions: &HashMap<String, Transaction>,
    scan: &ArtifactScan,
    copied_artifacts: &mut BTreeMap<PathBuf, PathBuf>,
) {
    if upgrade.upgrade_tx_id.is_empty() {
        return;
    }

    let Some(transaction) = transactions.get(&upgrade.upgrade_tx_id) else {
        upgrade.upgrade_tx_id.clear();
        return;
    };

    if let Some(direct) = scan.direct_by_tx_hash.get(&transaction.hash) {
        let canonical_path = project_root.join(&direct.canonical_path);
        if let Some(source) = direct.copy_from.as_ref() {
            copied_artifacts.entry(source.clone()).or_insert_with(|| canonical_path.clone());
        }
        upgrade.execution = Some(ExecutionRef {
            status: if transaction.status == TransactionStatus::Executed {
                ExecutionStatus::Executed
            } else {
                ExecutionStatus::Broadcast
            },
            kind: ExecutionKind::Tx,
            artifact_file: relative_path(project_root, &canonical_path),
            tx_hash: non_empty(&transaction.hash),
            safe_tx_hash: None,
            proposal_id: None,
            propose_safe_tx_hash: None,
            script_tx_index: Some(direct.script_tx_index),
        });
    } else if !transaction.hash.is_empty() {
        upgrade.execution = Some(ExecutionRef {
            status: ExecutionStatus::External,
            kind: ExecutionKind::ExternalTx,
            artifact_file: String::new(),
            tx_hash: Some(transaction.hash.clone()),
            safe_tx_hash: None,
            proposal_id: None,
            propose_safe_tx_hash: None,
            script_tx_index: None,
        });
    }

    upgrade.upgrade_tx_id.clear();
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

fn build_queued_index(
    deployments: &HashMap<String, Deployment>,
    queued_items: &[QueuedWorkItem],
) -> QueuedIndex {
    let mut deployment_ids_by_key: HashMap<String, Vec<String>> = HashMap::new();
    for deployment in deployments.values() {
        let Some(execution) = deployment.execution.as_ref() else { continue };
        if execution.status != ExecutionStatus::Queued {
            continue;
        }
        deployment_ids_by_key
            .entry(queued_item_key(
                &execution.artifact_file,
                &execution.kind,
                execution.safe_tx_hash.as_deref(),
                execution.proposal_id.as_deref(),
                execution.propose_safe_tx_hash.as_deref(),
            ))
            .or_default()
            .push(deployment.id.clone());
    }

    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for item in queued_items {
        if item.candidate.status != ExecutionStatus::Queued {
            continue;
        }
        let key = queued_item_key(
            &item.candidate.artifact_file,
            &item.candidate.kind,
            item.candidate.safe_tx_hash.as_deref(),
            item.candidate.proposal_id.as_deref(),
            item.candidate.propose_safe_tx_hash.as_deref(),
        );
        if !seen.insert(key.clone()) {
            continue;
        }
        let mut deployment_ids = deployment_ids_by_key.remove(&key).unwrap_or_default();
        deployment_ids.sort();
        entries.push(QueuedIndexEntry {
            deployment_ids,
            artifact_file: item.candidate.artifact_file.clone(),
            kind: item.candidate.kind.clone(),
            status: item.candidate.status.clone(),
            tx_hash: item.candidate.tx_hash.clone(),
            safe_tx_hash: item.candidate.safe_tx_hash.clone(),
            proposal_id: item.candidate.proposal_id.clone(),
            propose_safe_tx_hash: item.candidate.propose_safe_tx_hash.clone(),
        });
    }

    entries.sort_by(|left, right| left.artifact_file.cmp(&right.artifact_file));
    QueuedIndex { entries }
}

fn queued_item_key(
    artifact_file: &str,
    kind: &ExecutionKind,
    safe_tx_hash: Option<&str>,
    proposal_id: Option<&str>,
    propose_safe_tx_hash: Option<&str>,
) -> String {
    format!(
        "{artifact_file}|{:?}|{}|{}|{}",
        kind,
        safe_tx_hash.unwrap_or_default(),
        proposal_id.unwrap_or_default(),
        propose_safe_tx_hash.unwrap_or_default()
    )
}

impl MigrationPlan {
    fn report_output(&self, dry_run: bool, backup_path: Option<PathBuf>) -> MigrateOutput {
        let promoted_broadcast_artifacts =
            self.copied_artifacts.keys().filter(|path| !is_queued_artifact(path)).count();
        let promoted_queued_artifacts =
            self.copied_artifacts.keys().filter(|path| is_queued_artifact(path)).count();
        MigrateOutput {
            dry_run,
            backup_path: backup_path.map(|path| path.display().to_string()),
            deployments: self.deployments.len(),
            direct_refs: self.report.direct_refs,
            queued_refs: self.report.queued_refs,
            external_refs: self.report.external_refs,
            orphaned_refs: self.report.orphaned_refs,
            queued_entries: self.report.queued_entries,
            queued_artifacts: self.queued_artifacts.len(),
            promoted_broadcast_artifacts,
            promoted_queued_artifacts,
            orphaned_deployments: self.report.orphaned_deployments.clone(),
        }
    }
}

fn is_queued_artifact(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".queued.json"))
}

fn backup_existing_state(project_root: &Path) -> anyhow::Result<PathBuf> {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let backup_dir = registry_dir(project_root).join(format!("backups/migrate-{ts}"));
    fs::create_dir_all(&backup_dir)
        .with_context(|| format!("failed to create backup dir {}", backup_dir.display()))?;

    backup_legacy_registry(project_root, &backup_dir.join(".treb"))?;
    copy_if_exists(&deployments_dir(project_root), &backup_dir.join("deployments"))?;
    Ok(backup_dir)
}

fn backup_legacy_registry(project_root: &Path, target: &Path) -> anyhow::Result<()> {
    let source = registry_dir(project_root);
    if !source.exists() {
        return Ok(());
    }

    fs::create_dir_all(target)?;
    for entry in fs::read_dir(&source)? {
        let entry = entry?;
        if entry.file_name() == "backups" {
            continue;
        }
        copy_tree(&entry.path(), &target.join(entry.file_name()))?;
    }
    Ok(())
}

fn copy_if_exists(source: &Path, target: &Path) -> anyhow::Result<()> {
    if !source.exists() {
        return Ok(());
    }
    copy_tree(source, target)
}

fn copy_tree(source: &Path, target: &Path) -> anyhow::Result<()> {
    if source.is_dir() {
        fs::create_dir_all(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(&entry.path(), &target.join(entry.file_name()))?;
        }
        return Ok(());
    }

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(target, fs::read(source)?)?;
    Ok(())
}

fn write_migration_plan(project_root: &Path, plan: &MigrationPlan) -> anyhow::Result<()> {
    let deployments_root = deployments_dir(project_root);
    fs::create_dir_all(&deployments_root)?;

    let mut deployment_store = DeploymentStore::new(&deployments_root);
    deployment_store
        .replace_all(plan.deployments.clone())
        .map_err(|err| anyhow::anyhow!("{err}"))?;

    let mut addressbook_store = AddressbookStore::new(&deployments_root);
    addressbook_store
        .replace_all(plan.addressbook.clone())
        .map_err(|err| anyhow::anyhow!("{err}"))?;

    let mut queued_store = QueuedIndexStore::new(&deployments_root);
    queued_store.replace_all(plan.queued_index.clone()).map_err(|err| anyhow::anyhow!("{err}"))?;

    let solidity_store = SolidityRegistryStore::new(&deployments_root);
    solidity_store.rebuild(&plan.deployments).map_err(|err| anyhow::anyhow!("{err}"))?;

    for (source, target) in &plan.copied_artifacts {
        if target.exists() {
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(target, fs::read(source)?)?;
    }

    for (path, operations) in &plan.queued_artifacts {
        treb_registry::io::write_json_file(path, operations)
            .with_context(|| format!("failed to write queued artifact {}", path.display()))?;
    }

    Ok(())
}

fn render_output(output: MigrateOutput, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    if output.dry_run {
        println!("Migration dry-run:");
    } else {
        println!("Migration complete.");
    }
    println!("  Deployments: {}", output.deployments);
    println!("  Direct execution refs: {}", output.direct_refs);
    println!("  Queued execution refs: {}", output.queued_refs);
    println!("  External execution refs: {}", output.external_refs);
    println!("  Orphaned deployments: {}", output.orphaned_refs);
    println!("  Active queued entries: {}", output.queued_entries);
    println!("  Queued artifacts: {}", output.queued_artifacts);
    println!("  Promoted broadcast artifacts: {}", output.promoted_broadcast_artifacts);
    println!("  Promoted queued artifacts: {}", output.promoted_queued_artifacts);
    if let Some(backup_path) = output.backup_path {
        println!("  Backup: {backup_path}");
    }
    if !output.orphaned_deployments.is_empty() {
        println!("  Orphaned IDs: {}", output.orphaned_deployments.join(", "));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::Utc;
    use tempfile::TempDir;
    use treb_core::types::{
        ArtifactInfo, DeploymentMethod, DeploymentStrategy, DeploymentType, VerificationInfo,
        VerificationStatus,
    };

    use super::*;

    fn sample_deployment(id: &str, tx_id: &str) -> Deployment {
        let now = Utc::now();
        Deployment {
            id: id.to_string(),
            namespace: "default".to_string(),
            chain_id: 1,
            contract_name: "Counter".to_string(),
            label: "v1".to_string(),
            address: "0x1234567890123456789012345678901234567890".to_string(),
            deployment_type: DeploymentType::Singleton,
            execution: None,
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
                path: "src/Counter.sol".into(),
                compiler_version: "0.8.24".into(),
                bytecode_hash: "0xabc".into(),
                script_path: "script/Deploy.s.sol".into(),
                git_commit: "deadbee".into(),
            },
            verification: VerificationInfo {
                status: VerificationStatus::Unverified,
                etherscan_url: String::new(),
                verified_at: None,
                reason: String::new(),
                verifiers: HashMap::new(),
            },
            tags: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn sample_transaction(id: &str, hash: &str) -> Transaction {
        Transaction {
            id: id.to_string(),
            chain_id: 1,
            hash: hash.to_string(),
            status: TransactionStatus::Executed,
            block_number: 1,
            sender: "0x1".into(),
            nonce: 0,
            deployments: vec!["default/1/Counter:v1".into()],
            operations: vec![],
            safe_context: None,
            broadcast_file: Some("broadcast/Deploy.s.sol/1/run-latest.json".into()),
            environment: "test".into(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn plan_migrates_direct_deployment_to_archived_broadcast_file() {
        let dir = TempDir::new().unwrap();
        let project_root = dir.path();
        let broadcast_dir = project_root.join("broadcast/Deploy.s.sol/1");
        fs::create_dir_all(&broadcast_dir).unwrap();
        fs::write(
            broadcast_dir.join("run-latest.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "timestamp": 123,
                "transactions": [
                    { "hash": "0xaaa" }
                ]
            }))
            .unwrap(),
        )
        .unwrap();

        let legacy = LegacyState {
            deployments: HashMap::from([(
                "default/1/Counter:v1".into(),
                sample_deployment("default/1/Counter:v1", "tx-1"),
            )]),
            transactions: HashMap::from([("tx-1".into(), sample_transaction("tx-1", "0xaaa"))]),
            ..Default::default()
        };

        let plan = build_migration_plan(
            project_root,
            legacy,
            scan_broadcast_artifacts(project_root).unwrap(),
        )
        .unwrap();

        let deployment = plan.deployments.get("default/1/Counter:v1").unwrap();
        let execution = deployment.execution.as_ref().unwrap();
        assert_eq!(execution.kind, ExecutionKind::Tx);
        assert_eq!(execution.status, ExecutionStatus::Executed);
        assert_eq!(execution.artifact_file, "broadcast/Deploy.s.sol/1/run-123.json");
        assert!(plan.copied_artifacts.keys().any(|path| path.ends_with("run-latest.json")));
    }

    #[test]
    fn plan_synthesizes_queued_artifact_and_index_for_safe_governor_pair() {
        let dir = TempDir::new().unwrap();
        let project_root = dir.path();
        let deployment = sample_deployment("default/1/Counter:v1", "tx-1");
        let transaction = sample_transaction("tx-1", "0xaaa");
        let safe_tx = SafeTransaction {
            safe_tx_hash: "0xsafe".into(),
            safe_address: "0xsafeaddr".into(),
            chain_id: 1,
            status: TransactionStatus::Queued,
            nonce: 7,
            transactions: vec![],
            transaction_ids: vec!["tx-1".into()],
            proposed_by: "0xprop".into(),
            proposed_at: Utc::now(),
            confirmations: vec![],
            executed_at: None,
            execution_tx_hash: String::new(),
            fork_executed_at: None,
        };
        let governor = GovernorProposal {
            proposal_id: String::new(),
            governor_address: "0xgov".into(),
            timelock_address: String::new(),
            chain_id: 1,
            status: ProposalStatus::Pending,
            transaction_ids: vec!["tx-1".into()],
            proposed_by: "0xprop".into(),
            proposed_at: Utc::now(),
            description: String::new(),
            executed_at: None,
            execution_tx_hash: String::new(),
            fork_executed_at: None,
            actions: vec![],
        };

        let legacy = LegacyState {
            deployments: HashMap::from([(deployment.id.clone(), deployment)]),
            transactions: HashMap::from([("tx-1".into(), transaction)]),
            safe_transactions: HashMap::from([("0xsafe".into(), safe_tx)]),
            governor_proposals: HashMap::from([("proposal".into(), governor)]),
            ..Default::default()
        };

        let plan = build_migration_plan(project_root, legacy, ArtifactScan::default()).unwrap();
        let deployment = plan.deployments.get("default/1/Counter:v1").unwrap();
        let execution = deployment.execution.as_ref().unwrap();
        assert_eq!(execution.kind, ExecutionKind::GovernorProposalViaSafe);
        assert_eq!(execution.propose_safe_tx_hash.as_deref(), Some("0xsafe"));
        assert_eq!(plan.queued_index.entries.len(), 2);
        assert_eq!(plan.queued_artifacts.len(), 1);
    }
}
