//! Workspace cleanup utilities for pool-based test reuse.
//!
//! `clean_workspace()` resets a [`TestWorkdir`] to a pristine state by removing
//! build artifacts and treb state so the next test gets a clean workspace.

use std::{fs, path::Path};

/// Directories removed entirely during cleanup.
const REMOVE_DIRS: &[&str] = &[".treb", "broadcast", "cache", "out"];

/// Reset a test workspace to pristine state.
///
/// - Removes `.treb/`, `broadcast/`, `cache/`, `out/` directories.
/// - Cleans `script/deploy/` contents but preserves `.gitkeep`.
/// - Recreates `.treb/` as an empty directory.
/// - Missing directories are silently skipped (best-effort cleanup).
pub fn clean_workspace(workdir: &Path) {
    // Remove artifact directories.
    for dir in REMOVE_DIRS {
        let p = workdir.join(dir);
        if p.exists() {
            let _ = fs::remove_dir_all(&p);
        }
    }

    // Clean script/deploy/ but preserve .gitkeep.
    let deploy_dir = workdir.join("script").join("deploy");
    if deploy_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&deploy_dir) {
            for entry in entries.flatten() {
                if entry.file_name() == ".gitkeep" {
                    continue;
                }
                let path = entry.path();
                if path.is_dir() {
                    let _ = fs::remove_dir_all(&path);
                } else {
                    let _ = fs::remove_file(&path);
                }
            }
        }
    }

    // Recreate .treb/ as empty directory.
    let _ = fs::create_dir_all(workdir.join(".treb"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Helper: create a minimal workspace layout for testing cleanup.
    fn setup_workspace(root: &Path) {
        // Mutable items that should survive cleanup.
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/Counter.sol"), "// solidity").unwrap();
        fs::write(root.join("foundry.toml"), "[profile.default]").unwrap();

        // Artifact directories that should be removed.
        fs::create_dir_all(root.join(".treb")).unwrap();
        fs::write(root.join(".treb/registry.json"), "{}").unwrap();
        fs::create_dir_all(root.join("broadcast")).unwrap();
        fs::write(root.join("broadcast/run-latest.json"), "{}").unwrap();
        fs::create_dir_all(root.join("cache")).unwrap();
        fs::write(root.join("cache/solidity-files-cache.json"), "{}").unwrap();
        fs::create_dir_all(root.join("out")).unwrap();
        fs::write(root.join("out/Counter.json"), "{}").unwrap();

        // script/deploy/ with .gitkeep and a deploy script.
        fs::create_dir_all(root.join("script/deploy")).unwrap();
        fs::write(root.join("script/deploy/.gitkeep"), "").unwrap();
        fs::write(root.join("script/deploy/Deploy.s.sol"), "// deploy").unwrap();
    }

    #[test]
    fn removes_artifact_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        setup_workspace(root);

        clean_workspace(root);

        // Artifact dirs should be gone (except .treb which is recreated empty).
        assert!(!root.join("broadcast").exists());
        assert!(!root.join("cache").exists());
        assert!(!root.join("out").exists());

        // .treb/ is recreated but empty.
        assert!(root.join(".treb").is_dir());
        assert!(fs::read_dir(root.join(".treb")).unwrap().next().is_none());
    }

    #[test]
    fn preserves_source_and_config() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        setup_workspace(root);

        clean_workspace(root);

        // Source and config must survive.
        assert!(root.join("foundry.toml").exists());
        assert!(root.join("src/Counter.sol").exists());
    }

    #[test]
    fn cleans_script_deploy_preserving_gitkeep() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        setup_workspace(root);

        clean_workspace(root);

        // .gitkeep preserved, deploy script removed.
        assert!(root.join("script/deploy/.gitkeep").exists());
        assert!(!root.join("script/deploy/Deploy.s.sol").exists());
    }

    #[test]
    fn missing_directories_are_silently_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // Only create foundry.toml — no artifact dirs at all.
        fs::write(root.join("foundry.toml"), "[profile.default]").unwrap();

        // Should not panic.
        clean_workspace(root);

        // .treb/ is still recreated.
        assert!(root.join(".treb").is_dir());
    }
}
