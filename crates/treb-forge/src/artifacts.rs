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
    /// Whether this artifact is a Solidity library, detected by the standard
    /// `PUSH20 <placeholder>` prefix in the deployed bytecode.
    pub is_library: bool,
}

/// Check if raw bytecode bytes have the standard Solidity library prefix:
/// `PUSH20 (0x73)` followed by 20 zero bytes (the library address placeholder).
fn is_library_bytecode_pattern(bytes: &[u8]) -> bool {
    bytes.len() > 21 && bytes[0] == 0x73 && bytes[1..21].iter().all(|&b| b == 0)
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
                opt.map(|(id, data)| {
                    let is_library = data
                        .deployed_bytecode
                        .as_ref()
                        .and_then(|db| db.object.as_ref())
                        .and_then(|obj| obj.as_bytes())
                        .is_some_and(|bytes| is_library_bytecode_pattern(bytes));
                    ArtifactMatch {
                        artifact_id: id.clone(),
                        name: data.name.clone(),
                        abi: data.abi.clone(),
                        has_bytecode: data.bytecode.is_some(),
                        has_deployed_bytecode: data.deployed_bytecode.is_some(),
                        is_library,
                    }
                })
            })
            .map_err(|e| TrebError::Forge(format!("artifact lookup failed: {e}")))
    }

    /// Find a contract by creation (init) bytecode.
    ///
    /// Uses fuzzy matching with a 10% difference tolerance.
    pub fn find_by_creation_code(&self, code: &[u8]) -> Option<ArtifactMatch> {
        self.inner.find_by_creation_code(code).map(|(id, data)| {
            let is_library = data
                .deployed_bytecode
                .as_ref()
                .and_then(|db| db.object.as_ref())
                .and_then(|obj| obj.as_bytes())
                .is_some_and(|bytes| is_library_bytecode_pattern(bytes));
            ArtifactMatch {
                artifact_id: id.clone(),
                name: data.name.clone(),
                abi: data.abi.clone(),
                has_bytecode: data.bytecode.is_some(),
                has_deployed_bytecode: data.deployed_bytecode.is_some(),
                is_library,
            }
        })
    }

    /// Find a contract by deployed (runtime) bytecode.
    ///
    /// Uses fuzzy matching with a 15% difference tolerance.
    pub fn find_by_deployed_code(&self, code: &[u8]) -> Option<ArtifactMatch> {
        self.inner.find_by_deployed_code(code).map(|(id, data)| {
            let is_library = data
                .deployed_bytecode
                .as_ref()
                .and_then(|db| db.object.as_ref())
                .and_then(|obj| obj.as_bytes())
                .is_some_and(|bytes| is_library_bytecode_pattern(bytes));
            ArtifactMatch {
                artifact_id: id.clone(),
                name: data.name.clone(),
                abi: data.abi.clone(),
                has_bytecode: data.bytecode.is_some(),
                has_deployed_bytecode: data.deployed_bytecode.is_some(),
                is_library,
            }
        })
    }

    /// Match creation code and decode constructor arguments.
    ///
    /// Returns the matched artifact name and a list of formatted constructor
    /// arg strings (e.g. `["name: \"Test Token\"", "decimals: 6"]`).
    /// Returns `None` if no artifact matches.
    pub fn decode_creation_code(&self, code: &[u8]) -> Option<(ArtifactMatch, Vec<String>)> {
        use alloy_dyn_abi::JsonAbiExt;

        let (_, data) = self.inner.find_by_creation_code(code)?;

        let constructor = match data.abi.constructor() {
            Some(c) if !c.inputs.is_empty() => c,
            _ => {
                let matched = self.find_by_creation_code(code)?;
                return Some((matched, Vec::new()));
            }
        };

        // The artifact's compiled bytecode length tells us where constructor args start
        let artifact_bytecode = data.bytecode()?;
        if code.len() <= artifact_bytecode.len() {
            let matched = self.find_by_creation_code(code)?;
            return Some((matched, Vec::new()));
        }

        let arg_bytes = &code[artifact_bytecode.len()..];
        let matched = self.find_by_creation_code(code)?;

        let decoded = match constructor.abi_decode_input(arg_bytes) {
            Ok(vals) => vals,
            Err(_) => return Some((matched, Vec::new())),
        };

        let args: Vec<String> = decoded
            .iter()
            .zip(&constructor.inputs)
            .map(|(val, param)| {
                let name = if param.name.is_empty() { &param.ty } else { &param.name };
                format!("{}: {}", name, format_dyn_sol_value(val))
            })
            .collect();

        Some((matched, args))
    }

    /// Access the underlying `ContractsByArtifact` for foundry API interop.
    pub fn inner(&self) -> &ContractsByArtifact {
        &self.inner
    }
}

/// Format a decoded ABI value for concise display.
fn format_dyn_sol_value(val: &alloy_dyn_abi::DynSolValue) -> String {
    use alloy_dyn_abi::DynSolValue;
    match val {
        DynSolValue::Address(a) => format!("{:#x}", a),
        DynSolValue::String(s) => format!("\"{}\"", s),
        DynSolValue::Uint(n, _) => format!("{}", n),
        DynSolValue::Int(n, _) => format!("{}", n),
        DynSolValue::Bool(b) => format!("{}", b),
        DynSolValue::Bytes(b) => {
            if b.len() > 32 {
                format!("0x{}… ({} bytes)", alloy_primitives::hex::encode(&b[..4]), b.len())
            } else {
                format!("0x{}", alloy_primitives::hex::encode(b))
            }
        }
        DynSolValue::FixedBytes(w, _) => format!("0x{}", alloy_primitives::hex::encode(w)),
        DynSolValue::Array(items) | DynSolValue::FixedArray(items) => {
            let inner: Vec<String> = items.iter().map(format_dyn_sol_value).collect();
            format!("[{}]", inner.join(", "))
        }
        DynSolValue::Tuple(items) => {
            let inner: Vec<String> = items.iter().map(format_dyn_sol_value).collect();
            format!("({})", inner.join(", "))
        }
        DynSolValue::CustomStruct { tuple, .. } => {
            let inner: Vec<String> = tuple.iter().map(format_dyn_sol_value).collect();
            format!("({})", inner.join(", "))
        }
        DynSolValue::Function(f) => format!("0x{}", alloy_primitives::hex::encode(f)),
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
            is_library: false,
        };

        assert_eq!(artifact_match.name, "Counter");
        assert_eq!(artifact_match.artifact_id.name, "Counter");
        assert_eq!(artifact_match.artifact_id.source, PathBuf::from("src/Counter.sol"));
        assert!(artifact_match.has_bytecode);
        assert!(artifact_match.has_deployed_bytecode);
        assert!(!artifact_match.is_library);
        assert!(artifact_match.abi.functions.is_empty());
    }
}
