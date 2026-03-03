//! Forge version detection from foundry crate metadata.
//!
//! Reads the foundry-config crate version to report which forge version
//! is linked into the treb binary.

// TODO: Implement ForgeVersion struct (version, commit)
// TODO: Implement detect_forge_version() reading CARGO_PKG_VERSION
// TODO: Implement ForgeVersion::display_string()

/// Detected forge version from linked foundry crates.
pub struct ForgeVersion {
    // TODO: Add version and commit fields
}
