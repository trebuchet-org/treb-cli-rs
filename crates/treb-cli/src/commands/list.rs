//! `treb list` command implementation.

use std::collections::BTreeMap;
use std::env;

use anyhow::{Context, bail};
use treb_core::types::{Deployment, DeploymentType};
use treb_registry::Registry;

use crate::output;

/// Filter criteria for deployments. All specified filters are combined with AND logic.
pub struct DeploymentFilters {
    pub network: Option<String>,
    pub namespace: Option<String>,
    pub deployment_type: Option<String>,
    pub tag: Option<String>,
    pub contract: Option<String>,
    pub label: Option<String>,
    pub fork: bool,
    pub no_fork: bool,
}

/// Filter a slice of deployments by the given criteria.
///
/// All specified filters are combined with AND logic — a deployment must match
/// every active filter to be included in the result.
pub fn filter_deployments<'a>(
    deployments: &[&'a Deployment],
    filters: &DeploymentFilters,
) -> Vec<&'a Deployment> {
    deployments
        .iter()
        .copied()
        .filter(|d| {
            if let Some(ref ns) = filters.namespace {
                if !d.namespace.eq_ignore_ascii_case(ns) {
                    return false;
                }
            }

            if let Some(ref network) = filters.network {
                // Match against chain_id (as string) — case-insensitive
                if !d.chain_id.to_string().eq_ignore_ascii_case(network) {
                    return false;
                }
            }

            if let Some(ref dtype) = filters.deployment_type {
                if !d.deployment_type.to_string().eq_ignore_ascii_case(dtype) {
                    return false;
                }
            }

            if let Some(ref tag) = filters.tag {
                match &d.tags {
                    Some(tags) => {
                        if !tags.iter().any(|t| t == tag) {
                            return false;
                        }
                    }
                    None => return false,
                }
            }

            if let Some(ref contract) = filters.contract {
                if !d.contract_name.eq_ignore_ascii_case(contract) {
                    return false;
                }
            }

            if let Some(ref label) = filters.label {
                if d.label != *label {
                    return false;
                }
            }

            if filters.fork && !d.namespace.starts_with("fork/") {
                return false;
            }

            if filters.no_fork && d.namespace.starts_with("fork/") {
                return false;
            }

            true
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Deployment grouping: namespace > chain_id > deployment type
// ---------------------------------------------------------------------------

/// A group of deployments sharing the same deployment type within a chain.
pub struct TypeGroup<'a> {
    pub deployment_type: DeploymentType,
    pub deployments: Vec<&'a Deployment>,
}

/// Deployments organized by namespace > chain_id > deployment type category.
///
/// - Namespace keys sort alphabetically (BTreeMap).
/// - Chain IDs sort numerically (BTreeMap<u64, …>).
/// - Type groups are in fixed display order: Proxy, Singleton, Library, Unknown.
/// - Deployments within each type group are sorted by contract name.
pub type GroupedDeployments<'a> = BTreeMap<String, BTreeMap<u64, Vec<TypeGroup<'a>>>>;

/// Returns the sort key for the fixed display order of deployment types.
fn type_sort_key(dt: &DeploymentType) -> u8 {
    match dt {
        DeploymentType::Proxy => 0,
        DeploymentType::Singleton => 1,
        DeploymentType::Library => 2,
        DeploymentType::Unknown => 3,
    }
}

/// Organize a flat list of deployments into a hierarchical grouping:
/// namespace → chain_id → deployment type category.
///
/// The output is suitable for consumption by the tree renderer.
pub fn group_deployments<'a>(deployments: &[&'a Deployment]) -> GroupedDeployments<'a> {
    let mut result: GroupedDeployments<'a> = BTreeMap::new();

    for &d in deployments {
        let chain_map = result.entry(d.namespace.clone()).or_default();
        let type_groups = chain_map.entry(d.chain_id).or_default();

        if let Some(group) = type_groups
            .iter_mut()
            .find(|g| g.deployment_type == d.deployment_type)
        {
            group.deployments.push(d);
        } else {
            type_groups.push(TypeGroup {
                deployment_type: d.deployment_type.clone(),
                deployments: vec![d],
            });
        }
    }

    // Sort type groups by fixed order and deployments by contract name
    for chain_map in result.values_mut() {
        for type_groups in chain_map.values_mut() {
            type_groups.sort_by_key(|g| type_sort_key(&g.deployment_type));
            for group in type_groups.iter_mut() {
                group
                    .deployments
                    .sort_by(|a, b| a.contract_name.cmp(&b.contract_name));
            }
        }
    }

    result
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    network: Option<String>,
    namespace: Option<String>,
    deployment_type: Option<String>,
    tag: Option<String>,
    contract: Option<String>,
    label: Option<String>,
    fork: bool,
    no_fork: bool,
    json: bool,
) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;

    if !cwd.join("foundry.toml").exists() {
        bail!(
            "no foundry.toml found in {}\n\n\
             Run `forge init` to create a Foundry project, then `treb init`.",
            cwd.display()
        );
    }
    if !cwd.join(".treb").exists() {
        bail!(
            "project not initialized — .treb/ directory not found in {}\n\n\
             Run `treb init` first.",
            cwd.display()
        );
    }

    let registry = Registry::open(&cwd).context("failed to open registry")?;
    let all_deployments = registry.list_deployments();

    let filters = DeploymentFilters {
        network,
        namespace,
        deployment_type,
        tag,
        contract,
        label,
        fork,
        no_fork,
    };

    let filtered = filter_deployments(&all_deployments, &filters);

    if json {
        output::print_json(&filtered)?;
    } else if filtered.is_empty() {
        println!("No deployments found.");
    } else {
        let mut table = output::build_table(&[
            "Name",
            "Label",
            "Namespace",
            "Chain",
            "Type",
            "Address",
            "Verification",
        ]);

        for d in &filtered {
            table.add_row(vec![
                d.contract_name.as_str(),
                d.label.as_str(),
                d.namespace.as_str(),
                &d.chain_id.to_string(),
                &d.deployment_type.to_string(),
                &output::truncate_address(&d.address),
                &d.verification.status.to_string(),
            ]);
        }

        output::print_table(&table);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use std::collections::HashMap;
    use treb_core::types::{
        ArtifactInfo, DeploymentMethod, DeploymentStrategy, DeploymentType, VerificationInfo,
        VerificationStatus,
    };

    fn make_deployment(
        id: &str,
        namespace: &str,
        chain_id: u64,
        contract_name: &str,
        label: &str,
        dtype: DeploymentType,
        tags: Option<Vec<String>>,
    ) -> Deployment {
        Deployment {
            id: id.into(),
            namespace: namespace.into(),
            chain_id,
            contract_name: contract_name.into(),
            label: label.into(),
            address: format!("0x{:040x}", chain_id),
            deployment_type: dtype,
            transaction_id: "tx-001".into(),
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
                path: "out/Contract.json".into(),
                compiler_version: "0.8.24".into(),
                bytecode_hash: "0xabc".into(),
                script_path: "script/Deploy.s.sol".into(),
                git_commit: "abc1234".into(),
            },
            verification: VerificationInfo {
                status: VerificationStatus::Unverified,
                etherscan_url: String::new(),
                verified_at: None,
                reason: String::new(),
                verifiers: HashMap::new(),
            },
            tags,
            created_at: Utc.with_ymd_and_hms(2025, 1, 15, 10, 30, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2025, 1, 15, 10, 30, 0).unwrap(),
        }
    }

    fn no_filters() -> DeploymentFilters {
        DeploymentFilters {
            network: None,
            namespace: None,
            deployment_type: None,
            tag: None,
            contract: None,
            label: None,
            fork: false,
            no_fork: false,
        }
    }

    fn sample_deployments() -> Vec<Deployment> {
        vec![
            make_deployment(
                "mainnet/42220/FPMM:v3.0.0",
                "mainnet",
                42220,
                "FPMM",
                "v3.0.0",
                DeploymentType::Singleton,
                Some(vec!["core".into()]),
            ),
            make_deployment(
                "mainnet/42220/FPMMFactory:v3.0.0",
                "mainnet",
                42220,
                "FPMMFactory",
                "v3.0.0",
                DeploymentType::Singleton,
                None,
            ),
            make_deployment(
                "testnet/11155111/Counter:v1",
                "testnet",
                11155111,
                "Counter",
                "v1",
                DeploymentType::Library,
                Some(vec!["test".into(), "core".into()]),
            ),
            make_deployment(
                "fork/42220/FPMM:dev",
                "fork/42220",
                42220,
                "FPMM",
                "dev",
                DeploymentType::Proxy,
                None,
            ),
        ]
    }

    #[test]
    fn no_filters_returns_all() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let result = filter_deployments(&refs, &no_filters());
        assert_eq!(result.len(), 4);
    }

    #[test]
    fn filter_by_namespace() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.namespace = Some("mainnet".into());
        let result = filter_deployments(&refs, &filters);
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|d| d.namespace == "mainnet"));
    }

    #[test]
    fn filter_by_network_chain_id() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.network = Some("11155111".into());
        let result = filter_deployments(&refs, &filters);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].chain_id, 11155111);
    }

    #[test]
    fn filter_by_deployment_type() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.deployment_type = Some("SINGLETON".into());
        let result = filter_deployments(&refs, &filters);
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|d| d.deployment_type == DeploymentType::Singleton));
    }

    #[test]
    fn filter_by_deployment_type_case_insensitive() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.deployment_type = Some("proxy".into());
        let result = filter_deployments(&refs, &filters);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].deployment_type, DeploymentType::Proxy);
    }

    #[test]
    fn filter_by_tag() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.tag = Some("core".into());
        let result = filter_deployments(&refs, &filters);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn filter_by_tag_no_match() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.tag = Some("nonexistent".into());
        let result = filter_deployments(&refs, &filters);
        assert!(result.is_empty());
    }

    #[test]
    fn filter_by_contract_name() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.contract = Some("fpmm".into());
        let result = filter_deployments(&refs, &filters);
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|d| d.contract_name.eq_ignore_ascii_case("fpmm")));
    }

    #[test]
    fn filter_by_label() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.label = Some("v3.0.0".into());
        let result = filter_deployments(&refs, &filters);
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|d| d.label == "v3.0.0"));
    }

    #[test]
    fn filter_fork_only() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.fork = true;
        let result = filter_deployments(&refs, &filters);
        assert_eq!(result.len(), 1);
        assert!(result[0].namespace.starts_with("fork/"));
    }

    #[test]
    fn filter_no_fork() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.no_fork = true;
        let result = filter_deployments(&refs, &filters);
        assert_eq!(result.len(), 3);
        assert!(result.iter().all(|d| !d.namespace.starts_with("fork/")));
    }

    #[test]
    fn combined_filters_and_logic() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.namespace = Some("mainnet".into());
        filters.contract = Some("FPMM".into());
        let result = filter_deployments(&refs, &filters);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "mainnet/42220/FPMM:v3.0.0");
    }

    #[test]
    fn combined_filters_empty_result() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.namespace = Some("mainnet".into());
        filters.deployment_type = Some("LIBRARY".into());
        let result = filter_deployments(&refs, &filters);
        assert!(result.is_empty());
    }

    #[test]
    fn empty_deployments_returns_empty() {
        let refs: Vec<&Deployment> = vec![];
        let result = filter_deployments(&refs, &no_filters());
        assert!(result.is_empty());
    }

    // -----------------------------------------------------------------------
    // Grouping tests
    // -----------------------------------------------------------------------

    #[test]
    fn group_empty_returns_empty_btreemap() {
        let refs: Vec<&Deployment> = vec![];
        let grouped = group_deployments(&refs);
        assert!(grouped.is_empty());
    }

    #[test]
    fn group_single_namespace_chain_mixed_types() {
        // AC: 2 singletons + 1 proxy, all mainnet/42220
        let deployments = vec![
            make_deployment(
                "mainnet/42220/FPMM",
                "mainnet",
                42220,
                "FPMM",
                "",
                DeploymentType::Singleton,
                None,
            ),
            make_deployment(
                "mainnet/42220/FPMMFactory",
                "mainnet",
                42220,
                "FPMMFactory",
                "",
                DeploymentType::Singleton,
                None,
            ),
            make_deployment(
                "mainnet/42220/TransparentUpgradeableProxy",
                "mainnet",
                42220,
                "TransparentUpgradeableProxy",
                "",
                DeploymentType::Proxy,
                None,
            ),
        ];
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let grouped = group_deployments(&refs);

        // One namespace
        assert_eq!(grouped.len(), 1);
        assert!(grouped.contains_key("mainnet"));

        // One chain
        let chains = &grouped["mainnet"];
        assert_eq!(chains.len(), 1);
        assert!(chains.contains_key(&42220));

        // Two type groups: Proxy first (sort key 0), then Singleton (sort key 1)
        let type_groups = &chains[&42220];
        assert_eq!(type_groups.len(), 2);
        assert_eq!(type_groups[0].deployment_type, DeploymentType::Proxy);
        assert_eq!(type_groups[0].deployments.len(), 1);
        assert_eq!(
            type_groups[0].deployments[0].contract_name,
            "TransparentUpgradeableProxy"
        );
        assert_eq!(type_groups[1].deployment_type, DeploymentType::Singleton);
        assert_eq!(type_groups[1].deployments.len(), 2);
        assert_eq!(type_groups[1].deployments[0].contract_name, "FPMM");
        assert_eq!(type_groups[1].deployments[1].contract_name, "FPMMFactory");
    }

    #[test]
    fn group_multiple_namespaces_sorted_alphabetically() {
        let deployments = vec![
            make_deployment("z-ns/1/A", "z-ns", 1, "A", "", DeploymentType::Singleton, None),
            make_deployment("a-ns/1/B", "a-ns", 1, "B", "", DeploymentType::Singleton, None),
            make_deployment("m-ns/1/C", "m-ns", 1, "C", "", DeploymentType::Singleton, None),
        ];
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let grouped = group_deployments(&refs);

        let namespaces: Vec<&String> = grouped.keys().collect();
        assert_eq!(namespaces, vec!["a-ns", "m-ns", "z-ns"]);
    }

    #[test]
    fn group_chain_ids_sorted_numerically() {
        let deployments = vec![
            make_deployment("ns/999/A", "ns", 999, "A", "", DeploymentType::Singleton, None),
            make_deployment("ns/1/B", "ns", 1, "B", "", DeploymentType::Singleton, None),
            make_deployment("ns/42/C", "ns", 42, "C", "", DeploymentType::Singleton, None),
        ];
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let grouped = group_deployments(&refs);

        let chain_ids: Vec<u64> = grouped["ns"].keys().copied().collect();
        assert_eq!(chain_ids, vec![1, 42, 999]);
    }

    #[test]
    fn group_type_order_proxy_singleton_library() {
        let deployments = vec![
            make_deployment("ns/1/Lib", "ns", 1, "Lib", "", DeploymentType::Library, None),
            make_deployment("ns/1/Sing", "ns", 1, "Sing", "", DeploymentType::Singleton, None),
            make_deployment("ns/1/Prox", "ns", 1, "Prox", "", DeploymentType::Proxy, None),
        ];
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let grouped = group_deployments(&refs);

        let type_groups = &grouped["ns"][&1];
        assert_eq!(type_groups.len(), 3);
        assert_eq!(type_groups[0].deployment_type, DeploymentType::Proxy);
        assert_eq!(type_groups[1].deployment_type, DeploymentType::Singleton);
        assert_eq!(type_groups[2].deployment_type, DeploymentType::Library);
    }

    #[test]
    fn group_deployments_sorted_by_contract_name() {
        let deployments = vec![
            make_deployment("ns/1/Zeta", "ns", 1, "Zeta", "", DeploymentType::Singleton, None),
            make_deployment("ns/1/Alpha", "ns", 1, "Alpha", "", DeploymentType::Singleton, None),
            make_deployment("ns/1/Mid", "ns", 1, "Mid", "", DeploymentType::Singleton, None),
        ];
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let grouped = group_deployments(&refs);

        let names: Vec<&str> = grouped["ns"][&1][0]
            .deployments
            .iter()
            .map(|d| d.contract_name.as_str())
            .collect();
        assert_eq!(names, vec!["Alpha", "Mid", "Zeta"]);
    }

    #[test]
    fn group_mixed_namespaces_chains_and_types() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let grouped = group_deployments(&refs);

        // 3 namespaces: fork/42220, mainnet, testnet
        assert_eq!(grouped.len(), 3);
        let namespaces: Vec<&String> = grouped.keys().collect();
        assert_eq!(
            namespaces,
            vec!["fork/42220", "mainnet", "testnet"]
        );

        // mainnet has 1 chain (42220) with 1 type group (Singleton, 2 entries)
        let mainnet_chains = &grouped["mainnet"];
        assert_eq!(mainnet_chains.len(), 1);
        let mainnet_types = &mainnet_chains[&42220];
        assert_eq!(mainnet_types.len(), 1);
        assert_eq!(mainnet_types[0].deployment_type, DeploymentType::Singleton);
        assert_eq!(mainnet_types[0].deployments.len(), 2);

        // testnet has 1 chain (11155111) with 1 type group (Library, 1 entry)
        let testnet_types = &grouped["testnet"][&11155111];
        assert_eq!(testnet_types.len(), 1);
        assert_eq!(testnet_types[0].deployment_type, DeploymentType::Library);

        // fork/42220 has 1 chain (42220) with 1 type group (Proxy, 1 entry)
        let fork_types = &grouped["fork/42220"][&42220];
        assert_eq!(fork_types.len(), 1);
        assert_eq!(fork_types[0].deployment_type, DeploymentType::Proxy);
    }
}
