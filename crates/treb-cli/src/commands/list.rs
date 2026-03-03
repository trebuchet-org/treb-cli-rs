//! `treb list` command implementation.

use std::env;

use anyhow::{bail, Context};
use treb_core::types::Deployment;
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

/// Truncate an address to `0xABCD...EFGH` format (first 4 + last 4 hex chars).
fn truncate_address(address: &str) -> String {
    if address.len() >= 10 {
        format!("{}...{}", &address[..6], &address[address.len() - 4..])
    } else {
        address.to_string()
    }
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
                &truncate_address(&d.address),
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
        assert!(result
            .iter()
            .all(|d| d.deployment_type == DeploymentType::Singleton));
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
        assert!(result
            .iter()
            .all(|d| d.contract_name.eq_ignore_ascii_case("fpmm")));
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
}
