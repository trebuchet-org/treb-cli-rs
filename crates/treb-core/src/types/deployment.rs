//! Deployment model and nested types.
//!
//! Field names and serialization semantics match the Go implementation
//! at `treb-cli/internal/domain/models/deployment.go`.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::enums::{DeploymentMethod, DeploymentType, VerificationStatus};

// ---------------------------------------------------------------------------
// Deployment
// ---------------------------------------------------------------------------

/// A deployed contract instance on a specific chain.
///
/// Go registry files may emit RFC3339 timestamps with a source offset, while
/// Rust stores these as `DateTime<Utc>`. Round trips therefore preserve the
/// instant and field names, but serialize back out normalized to `Z`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Deployment {
    pub id: String,
    pub namespace: String,
    #[serde(rename = "chainId")]
    pub chain_id: u64,
    pub contract_name: String,
    pub label: String,
    pub address: String,
    #[serde(rename = "type")]
    pub deployment_type: DeploymentType,
    pub transaction_id: String,
    pub deployment_strategy: DeploymentStrategy,
    pub proxy_info: Option<ProxyInfo>,
    pub artifact: ArtifactInfo,
    pub verification: VerificationInfo,
    pub tags: Option<Vec<String>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// DeploymentStrategy
// ---------------------------------------------------------------------------

/// How a contract was deployed (CREATE / CREATE2 / CREATE3 + optional params).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentStrategy {
    pub method: DeploymentMethod,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub salt: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub init_code_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub factory: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub constructor_args: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub entropy: String,
}

// ---------------------------------------------------------------------------
// ProxyInfo
// ---------------------------------------------------------------------------

/// Proxy deployment details.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyInfo {
    #[serde(rename = "type")]
    pub proxy_type: String,
    pub implementation: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub admin: String,
    pub history: Vec<ProxyUpgrade>,
}

// ---------------------------------------------------------------------------
// ProxyUpgrade
// ---------------------------------------------------------------------------

/// A record of a proxy implementation upgrade.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyUpgrade {
    pub implementation_id: String,
    pub upgraded_at: DateTime<Utc>,
    pub upgrade_tx_id: String,
}

// ---------------------------------------------------------------------------
// ArtifactInfo
// ---------------------------------------------------------------------------

/// Metadata about the compiled contract artifact used in a deployment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactInfo {
    pub path: String,
    pub compiler_version: String,
    pub bytecode_hash: String,
    pub script_path: String,
    pub git_commit: String,
}

// ---------------------------------------------------------------------------
// VerificationInfo
// ---------------------------------------------------------------------------

/// Source-code verification status on block explorers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationInfo {
    pub status: VerificationStatus,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub etherscan_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub verifiers: HashMap<String, VerifierStatus>,
}

// ---------------------------------------------------------------------------
// VerifierStatus
// ---------------------------------------------------------------------------

/// Per-verifier status entry (e.g. etherscan, sourcify).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VerifierStatus {
    pub status: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn sample_deployment() -> Deployment {
        Deployment {
            id: "production/1/Counter:v1".into(),
            namespace: "production".into(),
            chain_id: 1,
            contract_name: "Counter".into(),
            label: "v1".into(),
            address: "0x1234567890abcdef1234567890abcdef12345678".into(),
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
                path: "out/Counter.sol/Counter.json".into(),
                compiler_version: "0.8.24".into(),
                bytecode_hash: "0xabcdef".into(),
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

    #[test]
    fn deployment_camel_case_field_names() {
        let d = sample_deployment();
        let json = serde_json::to_value(&d).unwrap();
        let obj = json.as_object().unwrap();

        // Verify camelCase keys
        assert!(obj.contains_key("id"));
        assert!(obj.contains_key("chainId"));
        assert!(obj.contains_key("contractName"));
        assert!(obj.contains_key("transactionId"));
        assert!(obj.contains_key("deploymentStrategy"));
        assert!(obj.contains_key("proxyInfo"));
        assert!(obj.contains_key("createdAt"));
        assert!(obj.contains_key("updatedAt"));
        assert!(obj.contains_key("type"));

        // Verify no snake_case keys leaked
        assert!(!obj.contains_key("chain_id"));
        assert!(!obj.contains_key("contract_name"));
        assert!(!obj.contains_key("transaction_id"));
        assert!(!obj.contains_key("deployment_type"));
    }

    #[test]
    fn tags_null_when_none() {
        let d = sample_deployment();
        let json = serde_json::to_value(&d).unwrap();
        assert!(json["tags"].is_null(), "tags should be null when None");
    }

    #[test]
    fn tags_array_when_some() {
        let mut d = sample_deployment();
        d.tags = Some(vec!["core".into(), "v2".into()]);
        let json = serde_json::to_value(&d).unwrap();
        assert_eq!(json["tags"], serde_json::json!(["core", "v2"]));
    }

    #[test]
    fn omitempty_fields_omitted_when_empty() {
        let d = sample_deployment();
        let json = serde_json::to_value(&d).unwrap();

        // DeploymentStrategy omitempty fields should be absent
        let strategy = &json["deploymentStrategy"];
        assert!(!strategy.as_object().unwrap().contains_key("salt"));
        assert!(!strategy.as_object().unwrap().contains_key("initCodeHash"));
        assert!(!strategy.as_object().unwrap().contains_key("factory"));
        assert!(!strategy.as_object().unwrap().contains_key("constructorArgs"));
        assert!(!strategy.as_object().unwrap().contains_key("entropy"));

        // VerificationInfo omitempty fields should be absent
        let verification = &json["verification"];
        assert!(!verification.as_object().unwrap().contains_key("etherscanUrl"));
        assert!(!verification.as_object().unwrap().contains_key("verifiedAt"));
        assert!(!verification.as_object().unwrap().contains_key("reason"));
        assert!(!verification.as_object().unwrap().contains_key("verifiers"));
    }

    #[test]
    fn omitempty_fields_present_when_populated() {
        let mut d = sample_deployment();
        d.deployment_strategy.salt = "0x1234".into();
        d.verification.etherscan_url = "https://etherscan.io/address/0x123".into();
        d.verification.verified_at = Some(Utc.with_ymd_and_hms(2025, 2, 1, 12, 0, 0).unwrap());
        d.verification.verifiers.insert(
            "etherscan".into(),
            VerifierStatus {
                status: "VERIFIED".into(),
                url: "https://etherscan.io".into(),
                reason: String::new(),
            },
        );

        let json = serde_json::to_value(&d).unwrap();
        assert!(json["deploymentStrategy"].as_object().unwrap().contains_key("salt"));
        assert!(json["verification"].as_object().unwrap().contains_key("etherscanUrl"));
        assert!(json["verification"].as_object().unwrap().contains_key("verifiedAt"));
        assert!(json["verification"].as_object().unwrap().contains_key("verifiers"));

        // VerifierStatus omitempty: reason should be absent
        let etherscan = &json["verification"]["verifiers"]["etherscan"];
        assert!(!etherscan.as_object().unwrap().contains_key("reason"));
        assert!(etherscan.as_object().unwrap().contains_key("url"));
    }

    #[test]
    fn deployment_serde_round_trip() {
        let d = sample_deployment();
        let json_str = serde_json::to_string_pretty(&d).unwrap();
        let deserialized: Deployment = serde_json::from_str(&json_str).unwrap();
        assert_eq!(d, deserialized);
    }

    #[test]
    fn deployment_with_proxy_serde_round_trip() {
        let mut d = sample_deployment();
        d.deployment_type = DeploymentType::Proxy;
        d.proxy_info = Some(ProxyInfo {
            proxy_type: "TransparentUpgradeableProxy".into(),
            implementation: "0xabcdef".into(),
            admin: "0xadmin".into(),
            history: vec![ProxyUpgrade {
                implementation_id: "impl-001".into(),
                upgraded_at: Utc.with_ymd_and_hms(2025, 1, 20, 8, 0, 0).unwrap(),
                upgrade_tx_id: "tx-002".into(),
            }],
        });

        let json_str = serde_json::to_string_pretty(&d).unwrap();
        let deserialized: Deployment = serde_json::from_str(&json_str).unwrap();
        assert_eq!(d, deserialized);
    }
}
