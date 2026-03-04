//! Domain newtypes for identifiers.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Generate a deployment ID string.
///
/// Returns `namespace/chainId/ContractName` when `label` is empty,
/// `namespace/chainId/ContractName:label` when non-empty. This matches
/// the Go CLI's convention exactly.
pub fn generate_deployment_id(
    namespace: &str,
    chain_id: u64,
    contract_name: &str,
    label: &str,
) -> String {
    if label.is_empty() {
        format!("{namespace}/{chain_id}/{contract_name}")
    } else {
        format!("{namespace}/{chain_id}/{contract_name}:{label}")
    }
}

/// Return a human-readable display name for a contract.
///
/// Returns `ContractName` when `label` is empty,
/// `ContractName:label` when non-empty.
pub fn contract_display_name(contract_name: &str, label: &str) -> String {
    if label.is_empty() { contract_name.to_string() } else { format!("{contract_name}:{label}") }
}

/// A deployment identifier, typically in the form `namespace/chainId/ContractName:Label`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeploymentId(pub String);

impl DeploymentId {
    /// Create a new `DeploymentId` from a string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Return the inner string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DeploymentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for DeploymentId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for DeploymentId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deployment_id_serde_transparent() {
        let id = DeploymentId::new("production/1/Counter:v1");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"production/1/Counter:v1\"");
        let parsed: DeploymentId = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn deployment_id_display() {
        let id = DeploymentId::new("ns/42/Token:main");
        assert_eq!(id.to_string(), "ns/42/Token:main");
    }

    #[test]
    fn deployment_id_from_conversions() {
        let from_str: DeploymentId = "a/b/c".into();
        let from_string: DeploymentId = String::from("a/b/c").into();
        assert_eq!(from_str, from_string);
    }

    #[test]
    fn generate_id_empty_label_omits_colon() {
        assert_eq!(
            generate_deployment_id("default", 31337, "Counter", ""),
            "default/31337/Counter"
        );
    }

    #[test]
    fn generate_id_non_empty_label_includes_colon() {
        assert_eq!(
            generate_deployment_id("default", 31337, "Counter", "v2"),
            "default/31337/Counter:v2"
        );
    }

    #[test]
    fn generate_id_various_namespaces_and_chains() {
        assert_eq!(
            generate_deployment_id("production", 1, "Token", "main"),
            "production/1/Token:main"
        );
        assert_eq!(
            generate_deployment_id("staging", 11155111, "Token", ""),
            "staging/11155111/Token"
        );
    }

    #[test]
    fn display_name_empty_label() {
        assert_eq!(contract_display_name("Counter", ""), "Counter");
    }

    #[test]
    fn display_name_non_empty_label() {
        assert_eq!(contract_display_name("Counter", "v2"), "Counter:v2");
    }
}
