//! Domain newtypes for identifiers.

use std::fmt;

use serde::{Deserialize, Serialize};

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
}
