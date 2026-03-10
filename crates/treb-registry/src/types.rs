//! Registry-specific types.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// In-memory secondary indexes for fast deployment lookups by name, address,
/// or tag.
///
/// Deployment IDs remain the canonical keys in `deployments.json`; this index
/// intentionally does not duplicate them under a separate `byId` map.
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
