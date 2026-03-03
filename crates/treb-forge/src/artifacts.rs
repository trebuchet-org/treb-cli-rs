//! Artifact indexing and lookup for compiled contracts.
//!
//! Wraps foundry's `ContractsByArtifact` to provide name-based and
//! bytecode-based contract lookups through a stable API.

// TODO: Implement ArtifactIndex wrapping ContractsByArtifact
// TODO: Implement from_compile_output(output) -> Result<Self>
// TODO: Implement find_by_name, find_by_creation_code, find_by_deployed_code
// TODO: Implement ArtifactMatch struct

/// Index over compiled contract artifacts for efficient lookups.
pub struct ArtifactIndex {
    // TODO: Add ContractsByArtifact inner field
}
