//! TestWorkdir — isolated temp directory with symlinked Foundry project.
//!
//! Creates a lightweight workspace by symlinking immutable dependencies and
//! copying mutable files from a fixture directory.  Each test gets a fast,
//! independent workspace.

use std::{
    fs,
    path::{Path, PathBuf},
};

/// The default fixture name used by integration tests.
pub const DEFAULT_FIXTURE: &str = "project";

/// Items that are symlinked (immutable deps — never modified by tests).
const SYMLINK_ITEMS: &[&str] = &["lib", "test", ".gitignore", "remappings.txt"];

/// Items that are recursively copied (mutable — tests may modify these).
const COPY_ITEMS: &[&str] = &["foundry.toml", "treb.toml", "src", "script"];

/// An isolated temporary workspace created from a fixture directory.
///
/// Immutable items (`lib/`, `test/`, `.gitignore`, `remappings.txt`) are
/// symlinked for speed.  Mutable items (`foundry.toml`, `treb.toml`, `src/`,
/// `script/`) are copied so each test can modify them independently.
///
/// The `.treb/` directory is created automatically.
pub struct TestWorkdir {
    /// The temp dir handle.  Dropping this removes the directory unless
    /// `TREB_TEST_SKIP_CLEANUP` is set.
    _temp: Option<tempfile::TempDir>,
    /// Absolute path to the workspace root.
    path: PathBuf,
}

impl TestWorkdir {
    /// Create a new workspace from the given fixture directory.
    ///
    /// Missing fixture items are silently skipped.
    pub fn new(fixture_dir: &Path) -> Self {
        let temp = tempfile::tempdir().expect("failed to create temp dir");
        let root = temp.path().to_path_buf();

        // Symlink immutable items.
        for item in SYMLINK_ITEMS {
            let src = fixture_dir.join(item);
            if src.exists() {
                let dst = root.join(item);
                #[cfg(unix)]
                std::os::unix::fs::symlink(&src, &dst)
                    .unwrap_or_else(|e| panic!("symlink {src:?} -> {dst:?}: {e}"));
                #[cfg(windows)]
                {
                    if src.is_dir() {
                        std::os::windows::fs::symlink_dir(&src, &dst)
                            .unwrap_or_else(|e| panic!("symlink_dir {src:?} -> {dst:?}: {e}"));
                    } else {
                        std::os::windows::fs::symlink_file(&src, &dst)
                            .unwrap_or_else(|e| panic!("symlink_file {src:?} -> {dst:?}: {e}"));
                    }
                }
            }
        }

        // Copy mutable items.
        for item in COPY_ITEMS {
            let src = fixture_dir.join(item);
            if src.exists() {
                let dst = root.join(item);
                if src.is_dir() {
                    copy_dir_recursive(&src, &dst)
                        .unwrap_or_else(|e| panic!("copy dir {src:?} -> {dst:?}: {e}"));
                } else {
                    fs::copy(&src, &dst)
                        .unwrap_or_else(|e| panic!("copy file {src:?} -> {dst:?}: {e}"));
                }
            }
        }

        // Create .treb/ directory.
        fs::create_dir_all(root.join(".treb")).expect("failed to create .treb dir");

        let skip_cleanup =
            std::env::var("TREB_TEST_SKIP_CLEANUP").map(|v| v == "1").unwrap_or(false);

        if skip_cleanup {
            let persisted = temp.keep();
            eprintln!("TREB_TEST_SKIP_CLEANUP: preserving temp dir at {}", persisted.display());
            Self { _temp: None, path: persisted }
        } else {
            Self { path: root, _temp: Some(temp) }
        }
    }

    /// Returns the absolute path to the workspace root.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the path to the `.treb/` directory.
    pub fn treb_dir(&self) -> PathBuf {
        self.path.join(".treb")
    }

    /// Returns the path to the fixture directory used by tests in this crate.
    pub fn fixture_dir(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)
    }
}

/// Rewrite localhost RPC ports in the workdir's `foundry.toml`.
///
/// Each entry in `rewrites` is `(old_port, new_port)`.  All occurrences of
/// `localhost:{old}` and `127.0.0.1:{old}` are replaced with the new port.
/// This naturally covers `http://localhost:{old}` URLs as well.
pub fn port_rewrite_foundry_toml(workdir: &Path, rewrites: &[(u16, u16)]) -> std::io::Result<()> {
    let toml_path = workdir.join("foundry.toml");
    let mut content = fs::read_to_string(&toml_path)?;

    for &(old_port, new_port) in rewrites {
        let old = old_port.to_string();
        let new = new_port.to_string();
        content = content
            .replace(&format!("localhost:{old}"), &format!("localhost:{new}"))
            .replace(&format!("127.0.0.1:{old}"), &format!("127.0.0.1:{new}"));
    }

    fs::write(&toml_path, content)
}

/// Convenience: rewrite all common default ports (8545, 9545) to `new_port`.
pub fn port_rewrite_foundry_toml_single(workdir: &Path, new_port: u16) -> std::io::Result<()> {
    port_rewrite_foundry_toml(workdir, &[(8545, new_port), (9545, new_port)])
}

/// Recursively copy a directory tree.
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
