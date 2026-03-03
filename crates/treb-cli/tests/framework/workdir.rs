//! TestWorkdir — isolated temp directory with symlinked Foundry project.
//!
//! Creates a lightweight workspace by symlinking immutable dependencies and
//! copying mutable files from a fixture directory.  Each test gets a fast,
//! independent workspace.

use std::fs;
use std::path::{Path, PathBuf};

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

        let skip_cleanup = std::env::var("TREB_TEST_SKIP_CLEANUP")
            .map(|v| v == "1")
            .unwrap_or(false);

        if skip_cleanup {
            let persisted = temp.keep();
            eprintln!("TREB_TEST_SKIP_CLEANUP: preserving temp dir at {}", persisted.display());
            Self {
                _temp: None,
                path: persisted,
            }
        } else {
            Self {
                path: root,
                _temp: Some(temp),
            }
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
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name)
    }
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

