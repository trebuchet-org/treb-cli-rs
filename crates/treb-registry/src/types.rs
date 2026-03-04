//! Registry-specific types: metadata and lookup index.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Metadata stored in `registry.json` at the root of the `.treb/` directory.
///
/// # Schema compatibility with Go CLI
///
/// The Go CLI writes a `SolidityRegistry` map to `registry.json` with the
/// shape `{chainId: {namespace: {name: address}}}`, tracking on-chain
/// registry contract addresses. Rust does not use on-chain registries and
/// instead stores versioning metadata (`version`, `createdAt`, `updatedAt`).
///
/// Resolution: these schemas serve different purposes and the Go CLI does
/// not read Rust's `registry.json` for its own operation (and vice versa).
/// Both CLIs tolerate unknown keys via serde's default behavior, so the
/// files can coexist if a project switches between CLIs. If future
/// interop requires it, the `SolidityRegistry` map can be added as an
/// optional field here or written to a separate file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryMeta {
    pub version: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl RegistryMeta {
    /// Create a new `RegistryMeta` at the current [`REGISTRY_VERSION`](crate::REGISTRY_VERSION).
    pub fn new() -> Self {
        let now = Utc::now();
        Self { version: crate::REGISTRY_VERSION, created_at: now, updated_at: now }
    }
}

impl Default for RegistryMeta {
    fn default() -> Self {
        Self::new()
    }
}

/// In-memory index for fast deployment lookups by name, address, or tag.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookupIndex {
    /// Maps lowercase contract name → list of deployment IDs.
    pub by_name: HashMap<String, Vec<String>>,
    /// Maps lowercase address → deployment ID.
    pub by_address: HashMap<String, String>,
    /// Maps tag → list of deployment IDs.
    pub by_tag: HashMap<String, Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_meta_new_sets_current_version() {
        let meta = RegistryMeta::new();
        assert_eq!(meta.version, crate::REGISTRY_VERSION);
    }

    #[test]
    fn registry_meta_new_sets_timestamps() {
        let before = Utc::now();
        let meta = RegistryMeta::new();
        let after = Utc::now();

        assert!(meta.created_at >= before && meta.created_at <= after);
        assert!(meta.updated_at >= before && meta.updated_at <= after);
    }

    #[test]
    fn registry_meta_serde_round_trip() {
        let meta = RegistryMeta::new();
        let json = serde_json::to_string_pretty(&meta).unwrap();
        let deserialized: RegistryMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(meta, deserialized);
    }

    #[test]
    fn registry_meta_camel_case_fields() {
        let meta = RegistryMeta::new();
        let json = serde_json::to_value(&meta).unwrap();
        let obj = json.as_object().unwrap();
        assert!(obj.contains_key("version"));
        assert!(obj.contains_key("createdAt"));
        assert!(obj.contains_key("updatedAt"));
        assert!(!obj.contains_key("created_at"));
        assert!(!obj.contains_key("updated_at"));
    }

    #[test]
    fn lookup_index_default_is_empty() {
        let index = LookupIndex::default();
        assert!(index.by_name.is_empty());
        assert!(index.by_address.is_empty());
        assert!(index.by_tag.is_empty());
    }

    #[test]
    fn lookup_index_serde_round_trip() {
        let mut index = LookupIndex::default();
        index.by_name.insert("mytoken".to_string(), vec!["dep-1".to_string()]);
        index.by_address.insert("0xabc".to_string(), "dep-1".to_string());
        index.by_tag.insert("core".to_string(), vec!["dep-1".to_string()]);

        let json = serde_json::to_string_pretty(&index).unwrap();
        let deserialized: LookupIndex = serde_json::from_str(&json).unwrap();
        assert_eq!(index, deserialized);
    }

    #[test]
    fn lookup_index_camel_case_fields() {
        let index = LookupIndex::default();
        let json = serde_json::to_value(&index).unwrap();
        let obj = json.as_object().unwrap();
        assert!(obj.contains_key("byName"));
        assert!(obj.contains_key("byAddress"));
        assert!(obj.contains_key("byTag"));
        assert!(!obj.contains_key("by_name"));
        assert!(!obj.contains_key("by_address"));
        assert!(!obj.contains_key("by_tag"));
    }
}
