use std::env;
use std::fs;
use std::path::PathBuf;

use similar::TextDiff;

use super::normalizer::{Normalizer, NormalizerChain};

// ---------------------------------------------------------------------------
// GoldenFile
// ---------------------------------------------------------------------------

/// Compares CLI output against baseline golden files, with optional
/// auto-update via the `UPDATE_GOLDEN=1` environment variable.
pub struct GoldenFile {
    golden_dir: PathBuf,
}

impl GoldenFile {
    pub fn new(golden_dir: impl Into<PathBuf>) -> Self {
        Self {
            golden_dir: golden_dir.into(),
        }
    }

    /// Build the path: `{golden_dir}/{test_name}/{case_name}.golden`
    fn path(&self, test_name: &str, case_name: &str) -> PathBuf {
        self.golden_dir
            .join(test_name)
            .join(format!("{case_name}.golden"))
    }

    /// Compare `actual` against the golden file at
    /// `{golden_dir}/{test_name}/{case_name}.golden`.
    ///
    /// - If `UPDATE_GOLDEN=1`, writes `actual` to the golden file (creating
    ///   parent dirs) and returns without comparing.
    /// - If the golden file is missing, panics with an instructive message.
    /// - If the content differs, panics with a unified diff.
    pub fn compare(&self, test_name: &str, case_name: &str, actual: &str) {
        let update = env::var("UPDATE_GOLDEN")
            .map(|v| v == "1")
            .unwrap_or(false);
        self.compare_inner(test_name, case_name, actual, update);
    }

    /// Core comparison logic, parameterised on `update` to avoid env-var
    /// races in parallel unit tests.
    fn compare_inner(
        &self,
        test_name: &str,
        case_name: &str,
        actual: &str,
        update: bool,
    ) {
        let path = self.path(test_name, case_name);

        if update {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("failed to create golden file directory");
            }
            fs::write(&path, actual).expect("failed to write golden file");
            return;
        }

        if !path.exists() {
            panic!(
                "Golden file not found: {}\n\
                 Run with UPDATE_GOLDEN=1 to create it.",
                path.display()
            );
        }

        let expected = fs::read_to_string(&path).expect("failed to read golden file");
        if expected != actual {
            let diff = TextDiff::from_lines(expected.as_str(), actual);
            let unified = diff
                .unified_diff()
                .header("expected (golden)", "actual")
                .to_string();
            panic!(
                "Golden file mismatch: {}\n\n{}\nRun with UPDATE_GOLDEN=1 to update.",
                path.display(),
                unified
            );
        }
    }

    /// Normalize `actual` with the default normalizer chain, then compare.
    pub fn compare_normalized(&self, test_name: &str, case_name: &str, actual: &str) {
        let chain = NormalizerChain::default_chain();
        let normalized = chain.normalize(actual);
        self.compare(test_name, case_name, &normalized);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::AssertUnwindSafe;
    use tempfile::TempDir;

    #[test]
    fn round_trip_write_then_compare() {
        let tmp = TempDir::new().unwrap();
        let golden = GoldenFile::new(tmp.path());

        // Write golden file via update mode
        golden.compare_inner("mytest", "output", "hello world\n", true);

        // Now compare succeeds silently
        golden.compare_inner("mytest", "output", "hello world\n", false);

        // Verify the file exists at the expected path
        let path = tmp.path().join("mytest").join("output.golden");
        assert!(path.exists());
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello world\n");
    }

    #[test]
    fn mismatch_produces_readable_diff() {
        let tmp = TempDir::new().unwrap();
        let golden = GoldenFile::new(tmp.path());

        // Write a golden file directly
        let dir = tmp.path().join("mytest");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("diff_case.golden"), "line1\nline2\n").unwrap();

        // Compare with different content — should panic with diff
        let golden = AssertUnwindSafe(golden);
        let result = std::panic::catch_unwind(move || {
            golden.compare_inner("mytest", "diff_case", "line1\nchanged\n", false);
        });
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = if let Some(s) = err.downcast_ref::<String>() {
            s.clone()
        } else if let Some(s) = err.downcast_ref::<&str>() {
            s.to_string()
        } else {
            panic!("unexpected panic type");
        };
        assert!(
            msg.contains("Golden file mismatch"),
            "expected mismatch message, got: {msg}"
        );
        assert!(
            msg.contains("UPDATE_GOLDEN=1"),
            "expected update instructions, got: {msg}"
        );
    }

    #[test]
    fn missing_golden_file_panics_with_instructions() {
        let tmp = TempDir::new().unwrap();
        let golden = AssertUnwindSafe(GoldenFile::new(tmp.path()));

        let result = std::panic::catch_unwind(move || {
            golden.compare_inner("nonexistent", "case", "anything", false);
        });
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = if let Some(s) = err.downcast_ref::<String>() {
            s.clone()
        } else if let Some(s) = err.downcast_ref::<&str>() {
            s.to_string()
        } else {
            panic!("unexpected panic type");
        };
        assert!(
            msg.contains("Golden file not found"),
            "expected 'not found' message, got: {msg}"
        );
        assert!(
            msg.contains("UPDATE_GOLDEN=1"),
            "expected update instructions, got: {msg}"
        );
    }
}
