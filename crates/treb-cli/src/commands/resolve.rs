//! Shared deployment resolution utility.
//!
//! Provides [`resolve_deployment`] which resolves a user-supplied query string
//! to a single [`Deployment`] using a 5-strategy resolution cascade.

use anyhow::bail;
use treb_core::types::Deployment;
use treb_registry::Registry;
use treb_registry::types::LookupIndex;

/// Resolve a user-supplied deployment query to a single deployment ID.
///
/// Resolution strategies (tried in order):
/// 1. Exact full ID match (e.g. `mainnet/42220/FPMM:v3.0.0`)
/// 2. Address match — query starts with `0x` (case-insensitive)
/// 3. Name:label match (e.g. `FPMM:v3.0.0`)
/// 4. Namespace/name match (e.g. `mainnet/FPMM`)
/// 5. Contract name match (e.g. `FPMM`, case-insensitive)
///
/// Returns an error if no match is found or if multiple candidates match.
pub fn resolve_deployment<'a>(
    query: &str,
    registry: &'a Registry,
    lookup: &LookupIndex,
) -> anyhow::Result<&'a Deployment> {
    // 1. Exact full ID
    if let Some(d) = registry.get_deployment(query) {
        return Ok(d);
    }

    // 2. Address (starts with 0x)
    if query.starts_with("0x") || query.starts_with("0X") {
        if let Some(id) = lookup.find_by_address(query) {
            if let Some(d) = registry.get_deployment(id) {
                return Ok(d);
            }
        }
        bail!("no deployment found with address '{query}'\n\nRun `treb list` to see available deployments.");
    }

    // 3. Name:label (contains `:`)
    if let Some((name, label)) = query.split_once(':') {
        if let Some(ids) = lookup.find_by_name(name) {
            let matches: Vec<&Deployment> = ids
                .iter()
                .filter_map(|id| registry.get_deployment(id))
                .filter(|d| d.label == label)
                .collect();
            return resolve_candidates(&matches, query);
        }
        bail!("no deployment found matching '{query}'\n\nRun `treb list` to see available deployments.");
    }

    // 4. Namespace/name (contains `/` but not a full ID which would have matched in step 1)
    if query.contains('/') {
        let all = registry.list_deployments();
        let matches: Vec<&Deployment> = all
            .into_iter()
            .filter(|d| {
                let prefix = format!("{}/{}", d.namespace, d.contract_name);
                prefix.eq_ignore_ascii_case(query)
            })
            .collect();
        return resolve_candidates(&matches, query);
    }

    // 5. Contract name (case-insensitive)
    if let Some(ids) = lookup.find_by_name(query) {
        let matches: Vec<&Deployment> = ids
            .iter()
            .filter_map(|id| registry.get_deployment(id))
            .collect();
        return resolve_candidates(&matches, query);
    }

    bail!("no deployment found matching '{query}'\n\nRun `treb list` to see available deployments.");
}

/// Given a list of candidate deployments, return exactly one or error.
fn resolve_candidates<'a>(
    candidates: &[&'a Deployment],
    query: &str,
) -> anyhow::Result<&'a Deployment> {
    match candidates.len() {
        0 => bail!("no deployment found matching '{query}'\n\nRun `treb list` to see available deployments."),
        1 => Ok(candidates[0]),
        _ => {
            let mut msg = format!("ambiguous deployment query '{query}' matches {} deployments:\n", candidates.len());
            for d in candidates {
                msg.push_str(&format!("  - {}\n", d.id));
            }
            msg.push_str("\nSpecify a more precise identifier to narrow the match.");
            bail!(msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use std::collections::HashMap;
    use tempfile::TempDir;
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
        address: &str,
    ) -> Deployment {
        Deployment {
            id: id.into(),
            namespace: namespace.into(),
            chain_id,
            contract_name: contract_name.into(),
            label: label.into(),
            address: address.into(),
            deployment_type: DeploymentType::Singleton,
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
            tags: None,
            created_at: Utc.with_ymd_and_hms(2025, 1, 15, 10, 30, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2025, 1, 15, 10, 30, 0).unwrap(),
        }
    }

    fn setup_registry() -> (TempDir, Registry) {
        let tmp = TempDir::new().unwrap();
        let mut registry = Registry::init(tmp.path()).unwrap();

        let deployments = vec![
            make_deployment(
                "mainnet/42220/FPMM:v3.0.0",
                "mainnet",
                42220,
                "FPMM",
                "v3.0.0",
                "0x42eDa75c4AC3fCf6eA20D091Ad1Ff79e9c52833D",
            ),
            make_deployment(
                "mainnet/42220/FPMMFactory:v3.0.0",
                "mainnet",
                42220,
                "FPMMFactory",
                "v3.0.0",
                "0x1234567890abcdef1234567890abcdef12345678",
            ),
            make_deployment(
                "testnet/11155111/FPMM:v2.0.0",
                "testnet",
                11155111,
                "FPMM",
                "v2.0.0",
                "0xabcdef1234567890abcdef1234567890abcdef12",
            ),
        ];

        for d in deployments {
            registry.insert_deployment(d).unwrap();
        }

        (tmp, registry)
    }

    #[test]
    fn resolve_exact_full_id() {
        let (_tmp, registry) = setup_registry();
        let lookup = registry.load_lookup_index().unwrap();
        let result = resolve_deployment("mainnet/42220/FPMM:v3.0.0", &registry, &lookup);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().id, "mainnet/42220/FPMM:v3.0.0");
    }

    #[test]
    fn resolve_by_address() {
        let (_tmp, registry) = setup_registry();
        let lookup = registry.load_lookup_index().unwrap();
        let result = resolve_deployment(
            "0x42eDa75c4AC3fCf6eA20D091Ad1Ff79e9c52833D",
            &registry,
            &lookup,
        );
        assert!(result.is_ok());
        assert_eq!(result.unwrap().id, "mainnet/42220/FPMM:v3.0.0");
    }

    #[test]
    fn resolve_by_address_case_insensitive() {
        let (_tmp, registry) = setup_registry();
        let lookup = registry.load_lookup_index().unwrap();
        let result = resolve_deployment(
            "0x42EDA75C4AC3FCF6EA20D091AD1FF79E9C52833D",
            &registry,
            &lookup,
        );
        assert!(result.is_ok());
        assert_eq!(result.unwrap().id, "mainnet/42220/FPMM:v3.0.0");
    }

    #[test]
    fn resolve_by_address_no_match() {
        let (_tmp, registry) = setup_registry();
        let lookup = registry.load_lookup_index().unwrap();
        let result = resolve_deployment("0xdeadbeef", &registry, &lookup);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no deployment found"));
        assert!(err.contains("treb list"));
    }

    #[test]
    fn resolve_by_contract_name_unique() {
        let (_tmp, registry) = setup_registry();
        let lookup = registry.load_lookup_index().unwrap();
        let result = resolve_deployment("FPMMFactory", &registry, &lookup);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().id, "mainnet/42220/FPMMFactory:v3.0.0");
    }

    #[test]
    fn resolve_by_contract_name_case_insensitive() {
        let (_tmp, registry) = setup_registry();
        let lookup = registry.load_lookup_index().unwrap();
        let result = resolve_deployment("fpmmfactory", &registry, &lookup);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().id, "mainnet/42220/FPMMFactory:v3.0.0");
    }

    #[test]
    fn resolve_by_contract_name_ambiguous() {
        let (_tmp, registry) = setup_registry();
        let lookup = registry.load_lookup_index().unwrap();
        let result = resolve_deployment("FPMM", &registry, &lookup);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("ambiguous"));
        assert!(err.contains("mainnet/42220/FPMM:v3.0.0"));
        assert!(err.contains("testnet/11155111/FPMM:v2.0.0"));
    }

    #[test]
    fn resolve_by_name_label() {
        let (_tmp, registry) = setup_registry();
        let lookup = registry.load_lookup_index().unwrap();
        let result = resolve_deployment("FPMM:v3.0.0", &registry, &lookup);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().id, "mainnet/42220/FPMM:v3.0.0");
    }

    #[test]
    fn resolve_by_name_label_no_match() {
        let (_tmp, registry) = setup_registry();
        let lookup = registry.load_lookup_index().unwrap();
        let result = resolve_deployment("FPMM:v99.0.0", &registry, &lookup);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no deployment found"));
    }

    #[test]
    fn resolve_by_namespace_name() {
        let (_tmp, registry) = setup_registry();
        let lookup = registry.load_lookup_index().unwrap();
        let result = resolve_deployment("mainnet/FPMM", &registry, &lookup);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().id, "mainnet/42220/FPMM:v3.0.0");
    }

    #[test]
    fn resolve_no_match() {
        let (_tmp, registry) = setup_registry();
        let lookup = registry.load_lookup_index().unwrap();
        let result = resolve_deployment("NonexistentContract", &registry, &lookup);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no deployment found"));
        assert!(err.contains("treb list"));
    }
}
