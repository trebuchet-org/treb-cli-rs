//! IntegrationTest — declarative multi-step CLI test scenarios.
//!
//! Provides [`IntegrationTest`] with builder methods and [`run_integration_test`]/
//! [`run_integration_tests`] entry points for defining and executing CLI test
//! scenarios that run commands in order and compare output against golden files.

use std::path::PathBuf;

use super::context::TestContext;
use super::golden::GoldenFile;
use super::normalizer::{Normalizer, NormalizerChain};

/// Callback that operates on a [`TestContext`] during test execution.
type Hook = Box<dyn Fn(&TestContext)>;

/// A declarative multi-step CLI test scenario.
///
/// Commands execute in order: `pre_setup` → `setup_cmds` → `post_setup` →
/// `test_cmds` → `post_test`.  Test command output is collected, normalized,
/// and compared against a golden file (unless `skip_golden` is set).
pub struct IntegrationTest {
    /// Test name — used as the golden file subdirectory.
    pub name: String,
    /// Hook that runs before setup commands.
    pub pre_setup: Option<Hook>,
    /// CLI arg lists executed as setup (always assert success).
    pub setup_cmds: Vec<Vec<String>>,
    /// Hook that runs after setup commands but before test commands.
    pub post_setup: Option<Hook>,
    /// CLI arg lists whose output is collected for golden comparison.
    pub test_cmds: Vec<Vec<String>>,
    /// Hook that runs after test commands.
    pub post_test: Option<Hook>,
    /// If true, test commands are expected to exit with non-zero status.
    pub expect_err: bool,
    /// Additional normalizers appended after the default chain.
    pub extra_normalizers: Vec<Box<dyn Normalizer>>,
    /// Relative paths to files in the workdir whose contents are appended
    /// to the golden output (only when the file exists).
    pub output_artifacts: Vec<String>,
    /// If true, skip golden file comparison entirely.
    pub skip_golden: bool,
}

impl IntegrationTest {
    /// Create a new test with the given name and all defaults.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            pre_setup: None,
            setup_cmds: Vec::new(),
            post_setup: None,
            test_cmds: Vec::new(),
            post_test: None,
            expect_err: false,
            extra_normalizers: Vec::new(),
            output_artifacts: Vec::new(),
            skip_golden: false,
        }
    }

    /// Add a setup command (arg list).  Setup commands always assert success.
    pub fn setup(mut self, args: &[&str]) -> Self {
        self.setup_cmds
            .push(args.iter().map(|s| s.to_string()).collect());
        self
    }

    /// Add a test command (arg list).  Output is collected for golden comparison.
    pub fn test(mut self, args: &[&str]) -> Self {
        self.test_cmds
            .push(args.iter().map(|s| s.to_string()).collect());
        self
    }

    /// Set the pre-setup hook.
    pub fn pre_setup_hook(mut self, hook: impl Fn(&TestContext) + 'static) -> Self {
        self.pre_setup = Some(Box::new(hook));
        self
    }

    /// Set the post-setup hook.
    pub fn post_setup_hook(mut self, hook: impl Fn(&TestContext) + 'static) -> Self {
        self.post_setup = Some(Box::new(hook));
        self
    }

    /// Set the post-test hook.
    pub fn post_test_hook(mut self, hook: impl Fn(&TestContext) + 'static) -> Self {
        self.post_test = Some(Box::new(hook));
        self
    }

    /// Set whether test commands are expected to fail (non-zero exit).
    pub fn expect_err(mut self, expect: bool) -> Self {
        self.expect_err = expect;
        self
    }

    /// Skip golden file comparison.
    pub fn skip_golden(mut self, skip: bool) -> Self {
        self.skip_golden = skip;
        self
    }

    /// Append an extra normalizer after the default chain.
    pub fn extra_normalizer(mut self, normalizer: Box<dyn Normalizer>) -> Self {
        self.extra_normalizers.push(normalizer);
        self
    }

    /// Add an output artifact path (relative to workdir) to include in golden output.
    pub fn output_artifact(mut self, path: &str) -> Self {
        self.output_artifacts.push(path.to_string());
        self
    }
}

/// Returns the golden file directory for this crate's tests.
fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
}

/// Execute a single [`IntegrationTest`] against the given [`TestContext`].
///
/// Runs commands in order: pre_setup → setup → post_setup → test → post_test.
/// Collects test command output, applies normalization, and compares against
/// the golden file at `tests/golden/{test.name}/output.golden`.
pub fn run_integration_test(test: &IntegrationTest, ctx: &TestContext) {
    // 1. pre_setup hook
    if let Some(hook) = &test.pre_setup {
        hook(ctx);
    }

    // 2. Setup commands — always assert success
    for cmd in &test.setup_cmds {
        ctx.run(cmd).success();
    }

    // 3. post_setup hook
    if let Some(hook) = &test.post_setup {
        hook(ctx);
    }

    // 4. Test commands — collect output
    let mut output = String::new();
    for (i, cmd) in test.test_cmds.iter().enumerate() {
        let assertion = ctx.run(cmd);
        let stdout = String::from_utf8_lossy(&assertion.get_output().stdout).to_string();
        let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();

        if test.expect_err {
            assertion.failure();
        } else {
            assertion.success();
        }

        output.push_str(&format!("=== cmd {}: [{}] ===\n", i, cmd.join(" ")));
        if !stdout.is_empty() {
            output.push_str(&stdout);
        }
        if !stderr.is_empty() {
            output.push_str(&stderr);
        }
        output.push('\n');
    }

    // 5. post_test hook
    if let Some(hook) = &test.post_test {
        hook(ctx);
    }

    // 6. Read output artifacts from workdir
    for artifact_path in &test.output_artifacts {
        let full_path = ctx.path().join(artifact_path);
        if full_path.exists() {
            let content = std::fs::read_to_string(&full_path)
                .unwrap_or_else(|e| panic!("failed to read artifact {artifact_path}: {e}"));
            output.push_str(&format!("\n--- {artifact_path} ---\n{content}"));
        }
    }

    // 7. Golden file comparison (unless skipped)
    if !test.skip_golden {
        // Apply default normalizer chain
        let default_chain = NormalizerChain::default_chain();
        let mut normalized = default_chain.normalize(&output);

        // Apply extra normalizers
        for n in &test.extra_normalizers {
            normalized = n.normalize(&normalized);
        }

        let golden = GoldenFile::new(golden_dir());
        golden.compare(&test.name, "output", &normalized);
    }
}

/// Execute multiple [`IntegrationTest`]s in sequence against the given [`TestContext`].
pub fn run_integration_tests(tests: &[IntegrationTest], ctx: &TestContext) {
    for test in tests {
        run_integration_test(test, ctx);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_sets_all_fields() {
        let test = IntegrationTest::new("my-test")
            .setup(&["init"])
            .setup(&["config", "set", "key", "value"])
            .test(&["list"])
            .test(&["show", "Contract"])
            .expect_err(true)
            .skip_golden(true)
            .output_artifact("broadcast/deploy.json")
            .output_artifact(".treb/state.json");

        assert_eq!(test.name, "my-test");
        assert_eq!(test.setup_cmds.len(), 2);
        assert_eq!(test.setup_cmds[0], vec!["init"]);
        assert_eq!(
            test.setup_cmds[1],
            vec!["config", "set", "key", "value"]
        );
        assert_eq!(test.test_cmds.len(), 2);
        assert_eq!(test.test_cmds[0], vec!["list"]);
        assert_eq!(test.test_cmds[1], vec!["show", "Contract"]);
        assert!(test.expect_err);
        assert!(test.skip_golden);
        assert_eq!(test.output_artifacts.len(), 2);
        assert_eq!(test.output_artifacts[0], "broadcast/deploy.json");
        assert_eq!(test.output_artifacts[1], ".treb/state.json");
        assert!(test.pre_setup.is_none());
        assert!(test.post_setup.is_none());
        assert!(test.post_test.is_none());
        assert!(test.extra_normalizers.is_empty());
    }

    #[test]
    fn builder_defaults() {
        let test = IntegrationTest::new("defaults");

        assert_eq!(test.name, "defaults");
        assert!(test.setup_cmds.is_empty());
        assert!(test.test_cmds.is_empty());
        assert!(!test.expect_err);
        assert!(!test.skip_golden);
        assert!(test.output_artifacts.is_empty());
        assert!(test.pre_setup.is_none());
        assert!(test.post_setup.is_none());
        assert!(test.post_test.is_none());
        assert!(test.extra_normalizers.is_empty());
    }

    #[test]
    fn builder_with_hooks() {
        let test = IntegrationTest::new("hooks")
            .pre_setup_hook(|_ctx| { /* custom pre-setup */ })
            .post_setup_hook(|_ctx| { /* custom post-setup */ })
            .post_test_hook(|_ctx| { /* custom post-test */ });

        assert!(test.pre_setup.is_some());
        assert!(test.post_setup.is_some());
        assert!(test.post_test.is_some());
    }
}
