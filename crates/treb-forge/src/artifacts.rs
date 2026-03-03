//! Artifact indexing and lookup for compiled contracts.
//!
//! Wraps foundry's `ContractsByArtifact` to provide name-based and
//! bytecode-based contract lookups through a stable API.

use alloy_json_abi::JsonAbi;
use foundry_common::ContractsByArtifact;
use foundry_compilers::ArtifactId;
use treb_core::error::TrebError;

use crate::compiler::CompilationOutput;

/// A match result from artifact queries.
#[derive(Debug)]
pub struct ArtifactMatch {
    /// The artifact identifier from the compilation output.
    pub artifact_id: ArtifactId,
    /// The contract name.
    pub name: String,
    /// The contract ABI.
    pub abi: JsonAbi,
    /// Whether creation bytecode is available.
    pub has_bytecode: bool,
    /// Whether deployed bytecode is available.
    pub has_deployed_bytecode: bool,
}

/// Index over compiled contract artifacts for efficient lookups.
pub struct ArtifactIndex {
    inner: ContractsByArtifact,
}

impl ArtifactIndex {
    /// Build an artifact index from compilation output.
    pub fn from_compile_output(output: CompilationOutput) -> Self {
        let contracts: ContractsByArtifact = output.into_output().into();
        Self { inner: contracts }
    }

    /// Find a contract by name or identifier.
    ///
    /// Returns an error if multiple contracts match the given name.
    pub fn find_by_name(&self, name: &str) -> treb_core::Result<Option<ArtifactMatch>> {
        self.inner
            .find_by_name_or_identifier(name)
            .map(|opt| {
                opt.map(|(id, data)| ArtifactMatch {
                    artifact_id: id.clone(),
                    name: data.name.clone(),
                    abi: data.abi.clone(),
                    has_bytecode: data.bytecode.is_some(),
                    has_deployed_bytecode: data.deployed_bytecode.is_some(),
                })
            })
            .map_err(|e| TrebError::Forge(format!("artifact lookup failed: {e}")))
    }

    /// Find a contract by creation (init) bytecode.
    ///
    /// Uses fuzzy matching with a 10% difference tolerance.
    pub fn find_by_creation_code(&self, code: &[u8]) -> Option<ArtifactMatch> {
        self.inner
            .find_by_creation_code(code)
            .map(|(id, data)| ArtifactMatch {
                artifact_id: id.clone(),
                name: data.name.clone(),
                abi: data.abi.clone(),
                has_bytecode: data.bytecode.is_some(),
                has_deployed_bytecode: data.deployed_bytecode.is_some(),
            })
    }

    /// Find a contract by deployed (runtime) bytecode.
    ///
    /// Uses fuzzy matching with a 15% difference tolerance.
    pub fn find_by_deployed_code(&self, code: &[u8]) -> Option<ArtifactMatch> {
        self.inner
            .find_by_deployed_code(code)
            .map(|(id, data)| ArtifactMatch {
                artifact_id: id.clone(),
                name: data.name.clone(),
                abi: data.abi.clone(),
                has_bytecode: data.bytecode.is_some(),
                has_deployed_bytecode: data.deployed_bytecode.is_some(),
            })
    }

    /// Access the underlying `ContractsByArtifact` for foundry API interop.
    pub fn inner(&self) -> &ContractsByArtifact {
        &self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn artifact_match_construction_and_field_access() {
        let artifact_match = ArtifactMatch {
            artifact_id: ArtifactId {
                path: PathBuf::from("out/Counter.sol/Counter.json"),
                name: "Counter".to_string(),
                source: PathBuf::from("src/Counter.sol"),
                version: foundry_config::semver::Version::new(0, 8, 19),
                build_id: String::new(),
                profile: "default".to_string(),
            },
            name: "Counter".to_string(),
            abi: JsonAbi::default(),
            has_bytecode: true,
            has_deployed_bytecode: true,
        };

        assert_eq!(artifact_match.name, "Counter");
        assert_eq!(artifact_match.artifact_id.name, "Counter");
        assert_eq!(
            artifact_match.artifact_id.source,
            PathBuf::from("src/Counter.sol")
        );
        assert!(artifact_match.has_bytecode);
        assert!(artifact_match.has_deployed_bytecode);
        assert!(artifact_match.abi.functions.is_empty());
    }
}
