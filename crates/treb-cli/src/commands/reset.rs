//! `treb reset` command implementation.
//!
//! Wipes registry state with optional --network and --namespace scoping,
//! creating a timestamped backup before any mutation.

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
struct ResetScopeDisplay {
    header_scope: String,
    confirmation_target: String,
}

fn describe_reset_scope(
    namespace: &str,
    requested_chain_id: Option<u64>,
    targeted_chain_ids: &BTreeSet<u64>,
) -> ResetScopeDisplay {
    let resolved_chain_id = requested_chain_id.or_else(|| {
        if targeted_chain_ids.len() == 1 { targeted_chain_ids.iter().next().copied() } else { None }
    });

    match resolved_chain_id {
        Some(chain_id) => ResetScopeDisplay {
            header_scope: format!(
                "for namespace '{namespace}' on network '{chain_id}' (chain {chain_id})"
            ),
            confirmation_target: format!(
                "the registry for namespace '{namespace}' on network '{chain_id}'"
            ),
        },
        None => ResetScopeDisplay {
            header_scope: format!("for namespace '{namespace}' across all networks"),
            confirmation_target: format!(
                "the registry for namespace '{namespace}' across all networks"
            ),
        },
    }
}

// ── Args ─────────────────────────────────────────────────────────────────────

/// Arguments for the `treb reset` command.
#[derive(Args, Debug)]
#[command(long_about = "Clear all deployments and transactions from the registry, \
optionally scoped to a specific network (by chain ID) or namespace. A timestamped \
backup is created under `.treb/backups/` before removing any data.")]
pub struct ResetArgs {
    /// Network name or chain ID
    #[arg(long)]
    pub network: Option<String>,

    /// Deployment namespace
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
pub async fn run(args: ResetArgs, non_interactive: bool) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;

    if !cwd.join("foundry.toml").exists() {
        bail!("no foundry.toml found in {}\n\nRun `forge init`, then `treb init`.", cwd.display());
    }
    if !cwd.join(".treb").exists() {
        bail!("project not initialized — .treb/ directory not found\n\nRun `treb init` first.");
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
            let ns_ok =
                args.namespace.as_deref().is_none_or(|ns| d.namespace.eq_ignore_ascii_case(ns));
            chain_ok && ns_ok
        })
        .map(|d| d.id.clone())
        .collect();
    let deployments_to_remove_set: HashSet<&str> =
        deployments_to_remove.iter().map(String::as_str).collect();

    let transactions_to_remove: Vec<String> = registry
        .list_transactions()
        .iter()
        .filter(|t| {
            let chain_ok = chain_id_filter.is_none_or(|id| t.chain_id == id);
            if !chain_ok {
                return false;
            }

            match args.namespace.as_deref() {
                Some(_) => {
                    !t.deployments.is_empty()
                        && t.deployments
                            .iter()
                            .all(|dep_id| deployments_to_remove_set.contains(dep_id.as_str()))
                }
                None => true,
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

            match args.namespace.as_deref() {
                Some(_) => {
                    !t.transaction_ids.is_empty()
                        && t.transaction_ids
                            .iter()
                            .all(|tx_id| transactions_to_remove_set.contains(tx_id.as_str()))
                }
                None => true,
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

            match args.namespace.as_deref() {
                Some(_) => {
                    !p.transaction_ids.is_empty()
                        && p.transaction_ids
                            .iter()
                            .all(|tx_id| transactions_to_remove_set.contains(tx_id.as_str()))
                }
                None => true,
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
            println!(
                "Nothing to reset. No registry entries found for the current namespace and network."
            );
        }
        return Ok(());
    }

    // Summary header with aligned per-type counts.
    let ns = args.namespace.as_deref().unwrap_or("default");
    let scope = describe_reset_scope(ns, chain_id_filter, &targeted_chain_ids);

    if !args.json {
        println!("Found {} items to reset {}:\n", total, scope.header_scope);
        if !deployments_to_remove.is_empty() {
            println!("  Deployments:        {}", deployments_to_remove.len());
        }
        if !transactions_to_remove.is_empty() {
            println!("  Transactions:       {}", transactions_to_remove.len());
        }
        if !safe_txs_to_remove.is_empty() {
            println!("  Safe Transactions:  {}", safe_txs_to_remove.len());
        }
        if !governor_proposals_to_remove.is_empty() {
            println!("  Governor Proposals: {}", governor_proposals_to_remove.len());
        }
        println!();
    }

    // Confirm.
    let skip_confirmation = args.yes || crate::ui::interactive::is_non_interactive(non_interactive);
    if skip_confirmation {
        if !args.json {
            println!("Running in non-interactive mode. Proceeding with reset...");
        }
    } else {
        let prompt = format!(
            "Are you sure you want to reset {}? This cannot be undone. [y/N]: ",
            scope.confirmation_target
        );
        let confirmed = if args.json {
            let stdin = io::stdin();
            let mut stdin = stdin.lock();
            let stderr = io::stderr();
            let mut stderr = stderr.lock();
            crate::ui::prompt::confirm_raw(&mut stderr, &mut stdin, &prompt)
                .context("failed to read reset confirmation")?
        } else {
            let stdin = io::stdin();
            let mut stdin = stdin.lock();
            let stdout = io::stdout();
            let mut stdout = stdout.lock();
            crate::ui::prompt::confirm_raw(&mut stdout, &mut stdin, &prompt)
                .context("failed to read reset confirmation")?
        };

        if !confirmed {
            if !args.json {
                println!("Reset cancelled.");
            }
            return Ok(());
        }
    }

    // Backup (created internally, path not displayed).
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
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
        println!("Successfully reset {} items from the registry.", total);
    }

    Ok(())
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::describe_reset_scope;
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

        registry.insert_deployment(make_deployment("dep-1", 1, "default")).unwrap();
        registry.insert_deployment(make_deployment("dep-2", 42220, "default")).unwrap();

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

        registry.insert_deployment(make_deployment("dep-1", 1, "default")).unwrap();
        registry.insert_deployment(make_deployment("dep-2", 1, "staging")).unwrap();

        // Verify lookup index is non-empty before reset.
        let index_before = registry.load_lookup_index().unwrap();
        assert!(!index_before.by_name.is_empty(), "lookup should be populated");

        // Simulate a full reset: remove all deployments.
        let ids: Vec<String> = registry.list_deployments().iter().map(|d| d.id.clone()).collect();
        for id in ids {
            registry.remove_deployment(&id).unwrap();
        }

        // Lookup index should now be empty.
        let index_after = registry.load_lookup_index().unwrap();
        assert!(index_after.by_name.is_empty(), "lookup.by_name should be empty after full reset");
        assert!(
            index_after.by_address.is_empty(),
            "lookup.by_address should be empty after full reset"
        );
    }

    #[test]
    fn describe_reset_scope_uses_target_chain_for_unfiltered_single_chain_resets() {
        let chain_ids = BTreeSet::from([1u64]);

        let scope = describe_reset_scope("staging", None, &chain_ids);

        assert_eq!(scope.header_scope, "for namespace 'staging' on network '1' (chain 1)");
        assert_eq!(
            scope.confirmation_target,
            "the registry for namespace 'staging' on network '1'"
        );
    }

    #[test]
    fn describe_reset_scope_uses_all_networks_for_unfiltered_mixed_chain_resets() {
        let chain_ids = BTreeSet::from([1u64, 42220u64]);

        let scope = describe_reset_scope("default", None, &chain_ids);

        assert_eq!(scope.header_scope, "for namespace 'default' across all networks");
        assert_eq!(
            scope.confirmation_target,
            "the registry for namespace 'default' across all networks"
        );
    }

    #[test]
    fn prompt_for_reset_confirmation_writes_go_style_prompt() {
        let mut output = Vec::new();
        let mut input = Cursor::new(b"y\n");

        let confirmed = confirm_raw(
            &mut output,
            &mut input,
            "Are you sure you want to reset the registry for namespace 'default' on network '1'? This cannot be undone. [y/N]: ",
        )
        .unwrap();

        assert!(confirmed);
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "Are you sure you want to reset the registry for namespace 'default' on network '1'? This cannot be undone. [y/N]: "
        );
    }
}
