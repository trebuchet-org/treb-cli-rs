//! Forge version detection from foundry crate metadata.
//!
//! Reads version information from the linked foundry crates to report
//! which forge version is compiled into the treb binary.

/// Detected forge version from linked foundry crates.
pub struct ForgeVersion {
    /// The semver version string (e.g. "1.5.1").
    pub version: String,
    /// The git commit hash, if available.
    pub commit: Option<String>,
}

impl ForgeVersion {
    /// Format the version as a display string.
    ///
    /// Returns `"forge v1.5.1"` or `"forge v1.5.1 (abc1234)"` if a commit hash is available.
    pub fn display_string(&self) -> String {
        match &self.commit {
            Some(hash) => format!("forge v{} ({})", self.version, hash),
            None => format!("forge v{}", self.version),
        }
    }
}

/// Detect the forge version from linked foundry crate metadata.
///
/// Reads the version from `foundry-common`'s compile-time version info,
/// which reflects the foundry-config `CARGO_PKG_VERSION` since all foundry
/// crates share the same version within a release tag.
pub fn detect_forge_version() -> ForgeVersion {
    // SHORT_VERSION is set at compile time by foundry-common's build.rs.
    // Format: "{pkg_version}-{tag_or_dev} ({git_sha_short} {timestamp})"
    // Example: "1.5.1-dev (b0a9dd9ced 2025-01-16T15:04:03.522Z)"
    let short = foundry_common::version::SHORT_VERSION;

    // Extract base semver (before first '-' or space).
    let version = short
        .split(['-', ' '])
        .next()
        .unwrap_or(short)
        .to_string();

    // Extract commit hash from within parentheses (first word after '(').
    let commit = short
        .find('(')
        .and_then(|start| {
            let rest = &short[start + 1..];
            let end = rest.find([' ', ')'])?;
            Some(rest[..end].to_string())
        })
        .filter(|s| !s.is_empty());

    ForgeVersion { version, commit }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_forge_version_returns_non_empty() {
        let version = detect_forge_version();
        assert!(!version.version.is_empty(), "version should not be empty");
    }

    #[test]
    fn display_string_without_commit() {
        let v = ForgeVersion {
            version: "1.5.1".to_string(),
            commit: None,
        };
        assert_eq!(v.display_string(), "forge v1.5.1");
    }

    #[test]
    fn display_string_with_commit() {
        let v = ForgeVersion {
            version: "1.5.1".to_string(),
            commit: Some("abc1234".to_string()),
        };
        assert_eq!(v.display_string(), "forge v1.5.1 (abc1234)");
    }
}
