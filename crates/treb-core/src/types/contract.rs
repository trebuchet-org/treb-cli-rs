//! Contract model and artifact types.
//!
//! Field names and serialization semantics match the Go implementation
//! at `treb-cli/internal/domain/models/contract.go`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A network name (plain string alias).
pub type Network = String;

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

/// A discovered contract with optional compiled artifact data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Contract {
    pub name: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub artifact_path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<Artifact>,
}

// ---------------------------------------------------------------------------
// Artifact
// ---------------------------------------------------------------------------

/// A Foundry compilation artifact.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub abi: serde_json::Value,
    pub bytecode: BytecodeObject,
    pub deployed_bytecode: BytecodeObject,
    pub method_identifiers: HashMap<String, String>,
    pub raw_metadata: String,
    pub metadata: ArtifactMetadata,
}

// ---------------------------------------------------------------------------
// BytecodeObject
// ---------------------------------------------------------------------------

/// Bytecode information in a Foundry artifact.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BytecodeObject {
    pub object: String,
    pub source_map: String,
    pub link_references: HashMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// ArtifactMetadata
// ---------------------------------------------------------------------------

/// The metadata section of a Foundry artifact.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtifactMetadata {
    pub compiler: ArtifactCompiler,
    pub language: String,
    pub output: ArtifactOutput,
    pub settings: ArtifactSettings,
}

/// Compiler info within artifact metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtifactCompiler {
    pub version: String,
}

/// Output section of artifact metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtifactOutput {
    pub abi: serde_json::Value,
    pub devdoc: serde_json::Value,
    pub userdoc: serde_json::Value,
    pub metadata: String,
}

/// Settings section of artifact metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactSettings {
    pub compilation_target: HashMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_contract() -> Contract {
        Contract {
            name: "Counter".into(),
            path: "src/Counter.sol".into(),
            artifact_path: String::new(),
            version: String::new(),
            artifact: None,
        }
    }

    fn sample_artifact() -> Artifact {
        Artifact {
            abi: serde_json::json!([
                {
                    "type": "function",
                    "name": "increment",
                    "inputs": [],
                    "outputs": [],
                    "stateMutability": "nonpayable"
                }
            ]),
            bytecode: BytecodeObject {
                object: "0x6080604052".into(),
                source_map: "".into(),
                link_references: HashMap::new(),
            },
            deployed_bytecode: BytecodeObject {
                object: "0x6080604052".into(),
                source_map: "".into(),
                link_references: HashMap::new(),
            },
            method_identifiers: {
                let mut m = HashMap::new();
                m.insert("increment()".into(), "d09de08a".into());
                m
            },
            raw_metadata: r#"{"compiler":{"version":"0.8.24"}}"#.into(),
            metadata: ArtifactMetadata {
                compiler: ArtifactCompiler { version: "0.8.24".into() },
                language: "Solidity".into(),
                output: ArtifactOutput {
                    abi: serde_json::json!([]),
                    devdoc: serde_json::json!({}),
                    userdoc: serde_json::json!({}),
                    metadata: String::new(),
                },
                settings: ArtifactSettings {
                    compilation_target: {
                        let mut m = HashMap::new();
                        m.insert("src/Counter.sol".into(), "Counter".into());
                        m
                    },
                },
            },
        }
    }

    #[test]
    fn contract_camel_case_field_names() {
        let c = sample_contract();
        let json = serde_json::to_value(&c).unwrap();
        let obj = json.as_object().unwrap();

        assert!(obj.contains_key("name"));
        assert!(obj.contains_key("path"));
        assert!(!obj.contains_key("artifact_path"));
        assert!(!obj.contains_key("artifactPath"), "empty artifactPath should be omitted");
    }

    #[test]
    fn contract_omitempty_fields_omitted() {
        let c = sample_contract();
        let json = serde_json::to_value(&c).unwrap();
        let obj = json.as_object().unwrap();

        assert!(!obj.contains_key("artifactPath"));
        assert!(!obj.contains_key("version"));
        assert!(!obj.contains_key("artifact"));
    }

    #[test]
    fn contract_omitempty_fields_present_when_populated() {
        let mut c = sample_contract();
        c.artifact_path = "out/Counter.sol/Counter.json".into();
        c.version = "0.8.24".into();
        c.artifact = Some(sample_artifact());

        let json = serde_json::to_value(&c).unwrap();
        let obj = json.as_object().unwrap();

        assert_eq!(obj["artifactPath"], "out/Counter.sol/Counter.json");
        assert_eq!(obj["version"], "0.8.24");
        assert!(obj.contains_key("artifact"));
    }

    #[test]
    fn artifact_json_structure() {
        let a = sample_artifact();
        let json = serde_json::to_value(&a).unwrap();
        let obj = json.as_object().unwrap();

        assert!(obj.contains_key("abi"));
        assert!(obj.contains_key("bytecode"));
        assert!(obj.contains_key("deployedBytecode"));
        assert!(obj.contains_key("methodIdentifiers"));
        assert!(obj.contains_key("rawMetadata"));
        assert!(obj.contains_key("metadata"));

        // No snake_case leaks
        assert!(!obj.contains_key("deployed_bytecode"));
        assert!(!obj.contains_key("method_identifiers"));
        assert!(!obj.contains_key("raw_metadata"));
    }

    #[test]
    fn bytecode_object_json_structure() {
        let b = BytecodeObject {
            object: "0x6080".into(),
            source_map: "1:2:3".into(),
            link_references: HashMap::new(),
        };
        let json = serde_json::to_value(&b).unwrap();
        let obj = json.as_object().unwrap();

        assert!(obj.contains_key("object"));
        assert!(obj.contains_key("sourceMap"));
        assert!(obj.contains_key("linkReferences"));
        assert!(!obj.contains_key("source_map"));
        assert!(!obj.contains_key("link_references"));
    }

    #[test]
    fn artifact_metadata_json_structure() {
        let m = ArtifactMetadata {
            compiler: ArtifactCompiler { version: "0.8.24".into() },
            language: "Solidity".into(),
            output: ArtifactOutput {
                abi: serde_json::json!([]),
                devdoc: serde_json::json!({}),
                userdoc: serde_json::json!({}),
                metadata: String::new(),
            },
            settings: ArtifactSettings { compilation_target: HashMap::new() },
        };
        let json = serde_json::to_value(&m).unwrap();
        let obj = json.as_object().unwrap();

        assert!(obj.contains_key("compiler"));
        assert!(obj.contains_key("language"));
        assert!(obj.contains_key("output"));
        assert!(obj.contains_key("settings"));

        assert_eq!(json["compiler"]["version"], "0.8.24");
        assert_eq!(json["settings"]["compilationTarget"], serde_json::json!({}));
    }

    #[test]
    fn contract_with_artifact_serde_round_trip() {
        let mut c = sample_contract();
        c.artifact_path = "out/Counter.sol/Counter.json".into();
        c.version = "0.8.24".into();
        c.artifact = Some(sample_artifact());

        let json_str = serde_json::to_string_pretty(&c).unwrap();
        let deserialized: Contract = serde_json::from_str(&json_str).unwrap();
        assert_eq!(c, deserialized);
    }

    #[test]
    fn contract_without_artifact_serde_round_trip() {
        let c = sample_contract();
        let json_str = serde_json::to_string_pretty(&c).unwrap();
        let deserialized: Contract = serde_json::from_str(&json_str).unwrap();
        assert_eq!(c, deserialized);
    }
}
