//! `treb registry drop` command implementation.
//!
//! Drops registry entries with optional query, --network, and --namespace scoping,
//! creating a timestamped backup before any mutation. Linked transactions and
//! proposals are only removed when all their linked deployments are being dropped
//! (orphan cascade).

use std::{
    collections::{BTreeSet, HashSet},
    env, io,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, bail};
use clap::Args;
use serde::Serialize;
use treb_registry::{REGISTRY_DIR, Registry, snapshot_registry};

use crate::output;

#[derive(Debug, Clone, PartialEq, Eq)]
struct DropScopeDisplay {
    header_scope: String,
    confirmation_target: String,
}

fn describe_drop_scope(
    query: Option<&str>,
    namespace: Option<&str>,
    requested_chain_id: Option<u64>,
    targeted_chain_ids: &BTreeSet<u64>,
) -> DropScopeDisplay {
    let resolved_chain_id = requested_chain_id.or_else(|| {
        if targeted_chain_ids.len() == 1 { targeted_chain_ids.iter().next().copied() } else { None }
    });

    let mut parts = Vec::new();
    if let Some(q) = query {
        parts.push(format!("matching query \"{q}\""));
    }
    if let Some(ns) = namespace {
        parts.push(format!("in namespace '{ns}'"));
    }
    match resolved_chain_id {
        Some(chain_id) => parts.push(format!("on network '{chain_id}' (chain {chain_id})")),
        None if namespace.is_some() || query.is_some() => {
            // Only add "across all networks" when there are other filters
        }
        None => parts.push("across all networks".to_string()),
    }

    let scope = if parts.is_empty() { String::new() } else { parts.join(" ") };

    DropScopeDisplay {
        header_scope: scope.clone(),
        confirmation_target: if scope.is_empty() {
            "the entire registry".to_string()
        } else {
            format!("registry entries {scope}")
        },
    }
}

/// Returns true if the deployment matches the given query string.
/// Matches against contract name (case-insensitive), label, or full deployment ID.
fn deployment_matches_query(d: &treb_core::types::Deployment, query: &str) -> bool {
    // Exact ID match
    if d.id == query {
        return true;
    }
    // Contract name match (case-insensitive)
    if d.contract_name.eq_ignore_ascii_case(query) {
        return true;
    }
    // Name:label match
    let name_label = format!("{}:{}", d.contract_name, d.label);
    if name_label.eq_ignore_ascii_case(query) {
        return true;
    }
    false
}

// ── Args ─────────────────────────────────────────────────────────────────────

/// Arguments for the `treb registry drop` command.
#[derive(Args, Debug)]
#[command(long_about = "Drop deployments and linked transactions from the registry, \
optionally scoped by query, network (chain ID), or namespace. At least one filter must \
be provided. Linked transactions and proposals are only removed when all their linked \
deployments are being dropped. A timestamped backup is created under `.treb/backups/` \
before removing any data.")]
pub struct DropArgs {
    /// Deployment query (contract name, label, or ID)
    pub query: Option<String>,

    /// Network name or chain ID
    #[arg(long, short = 'n')]
    pub network: Option<String>,

    /// Deployment namespace
    #[arg(long, short = 's')]
    pub namespace: Option<String>,

    /// Skip confirmation prompt
    #[arg(long, short = 'y')]
    pub yes: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

// ── run ───────────────────────────────────────────────────────────────────────

/// Entry point for `treb registry drop`.
pub async fn run(args: DropArgs, non_interactive: bool) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;

    if !cwd.join("foundry.toml").exists() {
        bail!("no foundry.toml found in {}\n\nRun `forge init`, then `treb init`.", cwd.display());
    }
    if !cwd.join(".treb").exists() {
        bail!("project not initialized — .treb/ directory not found\n\nRun `treb init` first.");
    }

    // At least one filter must be provided.
    if args.query.is_none() && args.network.is_none() && args.namespace.is_none() {
        bail!(
            "at least one of <query>, --network, or --namespace must be provided\n\n\
             Run 'treb registry drop --help' for usage."
        );
    }

    let scope = super::resolve_command_scope(&cwd, args.namespace.clone(), args.network.clone())?;
    let namespace = scope.namespace;
    let network = scope.network;

    // Resolve chain ID filter.
    let chain_id_filter = super::resolve_chain_id_for_network(&cwd, network.as_deref()).await?;

    let mut registry = Registry::open(&cwd).context("failed to open registry")?;

    // Collect entries to remove.
    let deployments_to_remove: Vec<String> = registry
        .list_deployments()
        .iter()
        .filter(|d| {
            let chain_ok = chain_id_filter.is_none_or(|id| d.chain_id == id);
            let ns_ok = namespace.as_deref().is_none_or(|ns| d.namespace.eq_ignore_ascii_case(ns));
            let query_ok = args.query.as_deref().is_none_or(|q| deployment_matches_query(d, q));
            chain_ok && ns_ok && query_ok
        })
        .map(|d| d.id.clone())
        .collect();
    let deployments_to_remove_set: HashSet<&str> =
        deployments_to_remove.iter().map(String::as_str).collect();

    // Orphan cascade: transactions are only removed when ALL their linked
    // deployments are being dropped.
    let has_scoping = args.query.is_some() || namespace.is_some();
    let transactions_to_remove: Vec<String> = registry
        .list_transactions()
        .iter()
        .filter(|t| {
            let chain_ok = chain_id_filter.is_none_or(|id| t.chain_id == id);
            if !chain_ok {
                return false;
            }

            if has_scoping {
                !t.deployments.is_empty()
                    && t.deployments
                        .iter()
                        .all(|dep_id| deployments_to_remove_set.contains(dep_id.as_str()))
            } else {
                true
            }
        })
        .map(|t| t.id.clone())
        .collect();
    let transactions_to_remove_set: HashSet<&str> =
        transactions_to_remove.iter().map(String::as_str).collect();

    let safe_txs_to_remove: Vec<String> = registry
        .list_safe_transactions()
        .iter()
        .filter(|t| {
            let chain_ok = chain_id_filter.is_none_or(|id| t.chain_id == id);
            if !chain_ok {
                return false;
            }

            if has_scoping {
                !t.transaction_ids.is_empty()
                    && t.transaction_ids
                        .iter()
                        .all(|tx_id| transactions_to_remove_set.contains(tx_id.as_str()))
            } else {
                true
            }
        })
        .map(|t| t.safe_tx_hash.clone())
        .collect();

    let governor_proposals_to_remove: Vec<String> = registry
        .list_governor_proposals()
        .iter()
        .filter(|p| {
            let chain_ok = chain_id_filter.is_none_or(|id| p.chain_id == id);
            if !chain_ok {
                return false;
            }

            if has_scoping {
                !p.transaction_ids.is_empty()
                    && p.transaction_ids
                        .iter()
                        .all(|tx_id| transactions_to_remove_set.contains(tx_id.as_str()))
            } else {
                true
            }
        })
        .map(|p| p.proposal_id.clone())
        .collect();

    let targeted_chain_ids: BTreeSet<u64> = deployments_to_remove
        .iter()
        .filter_map(|id| registry.get_deployment(id).map(|d| d.chain_id))
        .chain(
            transactions_to_remove
                .iter()
                .filter_map(|id| registry.get_transaction(id).map(|t| t.chain_id)),
        )
        .chain(
            safe_txs_to_remove
                .iter()
                .filter_map(|hash| registry.get_safe_transaction(hash).map(|t| t.chain_id)),
        )
        .chain(
            governor_proposals_to_remove
                .iter()
                .filter_map(|id| registry.get_governor_proposal(id).map(|p| p.chain_id)),
        )
        .collect();

    let total = deployments_to_remove.len()
        + transactions_to_remove.len()
        + safe_txs_to_remove.len()
        + governor_proposals_to_remove.len();

    if total == 0 {
        if !args.json {
            println!("Nothing to drop. No registry entries found matching the given filters.");
        }
        return Ok(());
    }

    // Summary header with aligned per-type counts.
    let scope = describe_drop_scope(
        args.query.as_deref(),
        namespace.as_deref(),
        chain_id_filter,
        &targeted_chain_ids,
    );

    if !args.json {
        println!("Dropping {} registry entries {}:\n", total, scope.header_scope);

        // Show deployment details
        if !deployments_to_remove.is_empty() {
            println!("  Deployments:        {}", deployments_to_remove.len());
            for id in &deployments_to_remove {
                if let Some(d) = registry.get_deployment(id) {
                    let addr_short = if d.address.len() > 10 {
                        format!("{}...{}", &d.address[..6], &d.address[d.address.len() - 4..])
                    } else {
                        d.address.clone()
                    };
                    let label =
                        if d.label.is_empty() { String::new() } else { format!(":{}", d.label) };
                    println!(
                        "    {}{} ({}) on chain {}",
                        d.contract_name, label, addr_short, d.chain_id
                    );
                }
            }
        }
        if !transactions_to_remove.is_empty() {
            let orphan_note = if has_scoping { "  (orphaned)" } else { "" };
            println!("  Transactions:       {}{}", transactions_to_remove.len(), orphan_note);
        }
        if !safe_txs_to_remove.is_empty() {
            let orphan_note = if has_scoping { "  (orphaned)" } else { "" };
            println!("  Safe Transactions:  {}{}", safe_txs_to_remove.len(), orphan_note);
        }
        if !governor_proposals_to_remove.is_empty() {
            let orphan_note = if has_scoping { "  (orphaned)" } else { "" };
            println!("  Governor Proposals: {}{}", governor_proposals_to_remove.len(), orphan_note);
        }
        println!();
    }

    // Confirm.
    let skip_confirmation = args.yes || crate::ui::interactive::is_non_interactive(non_interactive);
    if skip_confirmation {
        if !args.json {
            println!("Running in non-interactive mode. Proceeding with drop...");
        }
    } else {
        let prompt = format!(
            "Are you sure you want to drop {}? This cannot be undone. [y/N]: ",
            scope.confirmation_target
        );
        let confirmed = if args.json {
            let stdin = io::stdin();
            let mut stdin = stdin.lock();
            let stderr = io::stderr();
            let mut stderr = stderr.lock();
            crate::ui::prompt::confirm_raw(&mut stderr, &mut stdin, &prompt)
                .context("failed to read drop confirmation")?
        } else {
            let stdin = io::stdin();
            let mut stdin = stdin.lock();
            let stdout = io::stdout();
            let mut stdout = stdout.lock();
            crate::ui::prompt::confirm_raw(&mut stdout, &mut stdin, &prompt)
                .context("failed to read drop confirmation")?
        };

        if !confirmed {
            if !args.json {
                println!("Drop cancelled.");
            }
            return Ok(());
        }
    }

    // Backup.
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
    let registry_dir = cwd.join(REGISTRY_DIR);
    let backup_dir = registry_dir.join(format!("backups/drop-{ts}"));
    snapshot_registry(&registry_dir, &backup_dir)
        .with_context(|| format!("failed to create backup at {}", backup_dir.display()))?;

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
        struct DropResult {
            removed_deployments: usize,
            removed_transactions: usize,
            removed_safe_transactions: usize,
            removed_governor_proposals: usize,
            backup_path: String,
        }
        output::print_json(&DropResult {
            removed_deployments: deployments_to_remove.len(),
            removed_transactions: transactions_to_remove.len(),
            removed_safe_transactions: safe_txs_to_remove.len(),
            removed_governor_proposals: governor_proposals_to_remove.len(),
            backup_path: backup_dir.display().to_string(),
        })?;
    } else {
        println!("Successfully dropped {} items from the registry.", total);
    }

    Ok(())
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::{deployment_matches_query, describe_drop_scope};
    use crate::ui::prompt::confirm_raw;
    use std::{
        collections::{BTreeSet, HashMap},
        io::Cursor,
    };

    use chrono::Utc;
    use tempfile::TempDir;
    use treb_core::types::{
        ArtifactInfo, DeploymentMethod, DeploymentStrategy, DeploymentType, VerificationInfo,
        VerificationStatus,
    };
    use treb_registry::Registry;

    fn make_deployment(id: &str, chain_id: u64, namespace: &str) -> treb_core::types::Deployment {
        make_deployment_named(id, chain_id, namespace, "TestContract", "v1")
    }

    fn make_deployment_named(
        id: &str,
        chain_id: u64,
        namespace: &str,
        contract_name: &str,
        label: &str,
    ) -> treb_core::types::Deployment {
        let ts = Utc::now();
        treb_core::types::Deployment {
            id: id.to_string(),
            namespace: namespace.to_string(),
            chain_id,
            contract_name: contract_name.to_string(),
            label: label.to_string(),
            address: format!("0x{:040x}", 1u64),
            deployment_type: DeploymentType::Singleton,
            execution: None,
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

        registry.insert_deployment(make_deployment("dep-1", 1, "default")).unwrap();
        registry.insert_deployment(make_deployment("dep-2", 42220, "default")).unwrap();

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

        registry.insert_deployment(make_deployment("dep-1", 1, "default")).unwrap();
        registry.insert_deployment(make_deployment("dep-2", 1, "staging")).unwrap();

        let to_remove: Vec<String> = registry
            .list_deployments()
            .iter()
            .filter(|d| d.namespace.eq_ignore_ascii_case("default"))
            .map(|d| d.id.clone())
            .collect();

        assert_eq!(to_remove, vec!["dep-1"]);
    }

    #[test]
    fn empty_registry_returns_nothing_to_drop() {
        let dir = TempDir::new().unwrap();
        let registry = Registry::init(dir.path()).unwrap();

        let total: usize = registry.deployment_count()
            + registry.transaction_count()
            + registry.safe_transaction_count()
            + registry.governor_proposal_count();
        assert_eq!(total, 0);
    }

    #[test]
    fn full_drop_empties_lookup_index() {
        let dir = TempDir::new().unwrap();
        let mut registry = Registry::init(dir.path()).unwrap();

        registry.insert_deployment(make_deployment("dep-1", 1, "default")).unwrap();
        registry.insert_deployment(make_deployment("dep-2", 1, "staging")).unwrap();

        let index_before = registry.load_lookup_index().unwrap();
        assert!(!index_before.by_name.is_empty(), "lookup should be populated");

        let ids: Vec<String> = registry.list_deployments().iter().map(|d| d.id.clone()).collect();
        for id in ids {
            registry.remove_deployment(&id).unwrap();
        }

        let index_after = registry.load_lookup_index().unwrap();
        assert!(index_after.by_name.is_empty(), "lookup.by_name should be empty after full drop");
        assert!(
            index_after.by_address.is_empty(),
            "lookup.by_address should be empty after full drop"
        );
    }

    #[test]
    fn describe_drop_scope_with_network_filter() {
        let chain_ids = BTreeSet::from([1u64]);

        let scope = describe_drop_scope(None, Some("staging"), Some(1), &chain_ids);

        assert_eq!(scope.header_scope, "in namespace 'staging' on network '1' (chain 1)");
    }

    #[test]
    fn describe_drop_scope_with_query() {
        let chain_ids = BTreeSet::from([42220u64]);

        let scope = describe_drop_scope(Some("Counter"), None, None, &chain_ids);

        assert_eq!(
            scope.header_scope,
            "matching query \"Counter\" on network '42220' (chain 42220)"
        );
    }

    #[test]
    fn describe_drop_scope_all_networks() {
        let chain_ids = BTreeSet::from([1u64, 42220u64]);

        let scope = describe_drop_scope(None, Some("default"), None, &chain_ids);

        assert_eq!(scope.header_scope, "in namespace 'default'");
    }

    #[test]
    fn deployment_matches_query_by_contract_name() {
        let d = make_deployment_named("id-1", 1, "default", "Counter", "v1");
        assert!(deployment_matches_query(&d, "Counter"));
        assert!(deployment_matches_query(&d, "counter"));
        assert!(!deployment_matches_query(&d, "Token"));
    }

    #[test]
    fn deployment_matches_query_by_name_label() {
        let d = make_deployment_named("id-1", 1, "default", "Counter", "v2");
        assert!(deployment_matches_query(&d, "Counter:v2"));
        assert!(!deployment_matches_query(&d, "Counter:v1"));
    }

    #[test]
    fn deployment_matches_query_by_id() {
        let d = make_deployment_named("dep-abc-123", 1, "default", "Counter", "v1");
        assert!(deployment_matches_query(&d, "dep-abc-123"));
        assert!(!deployment_matches_query(&d, "dep-abc-456"));
    }

    #[test]
    fn prompt_for_drop_confirmation_writes_prompt() {
        let mut output = Vec::new();
        let mut input = Cursor::new(b"y\n");

        let confirmed = confirm_raw(
            &mut output,
            &mut input,
            "Are you sure you want to drop registry entries in namespace 'default' on network '1' (chain 1)? This cannot be undone. [y/N]: ",
        )
        .unwrap();

        assert!(confirmed);
    }
}
