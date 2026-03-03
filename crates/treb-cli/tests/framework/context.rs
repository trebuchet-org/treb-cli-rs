//! TestContext — composition struct for integration tests.
//!
//! Combines [`TestWorkdir`], [`TrebRunner`], and optional [`AnvilNode`] instances
//! into a single convenient handle for writing integration tests.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use super::anvil_node::AnvilNode;
use super::runner::TrebRunner;
use super::workdir::{port_rewrite_foundry_toml_single, TestWorkdir};

/// A complete test environment composing workdir, runner, and optional Anvil nodes.
pub struct TestContext {
    workdir: TestWorkdir,
    runner: TrebRunner,
    anvil_nodes: HashMap<String, AnvilNode>,
}

impl TestContext {
    /// Create a new test context from the named fixture directory.
    ///
    /// Creates a [`TestWorkdir`] from `tests/fixtures/{fixture_name}` and a
    /// [`TrebRunner`] pointing at it.
    pub fn new(fixture_name: &str) -> Self {
        let fixture_dir = TestWorkdir::fixture_dir(fixture_name);
        let workdir = TestWorkdir::new(&fixture_dir);
        let runner = TrebRunner::new(workdir.path());

        Self {
            workdir,
            runner,
            anvil_nodes: HashMap::new(),
        }
    }

    /// Spawn a named Anvil node and rewrite `foundry.toml` ports to reach it.
    ///
    /// The node is stored under `name` and can be retrieved with [`anvil`].
    /// All default RPC ports (8545, 9545) in `foundry.toml` are rewritten to
    /// point at the new node's port.
    pub async fn with_anvil(
        mut self,
        name: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let node = AnvilNode::spawn().await?;
        port_rewrite_foundry_toml_single(self.workdir.path(), node.port())?;
        self.anvil_nodes.insert(name.to_string(), node);
        Ok(self)
    }

    /// Run `treb <args>` in the test workdir.
    pub fn run<I, S>(&self, args: I) -> assert_cmd::assert::Assert
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.runner.run(args)
    }

    /// Run `treb <args>` with additional environment variables.
    pub fn run_with_env<I, S, E, K, V>(
        &self,
        args: I,
        env_vars: E,
    ) -> assert_cmd::assert::Assert
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
        E: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.runner.run_with_env(args, env_vars)
    }

    /// The workdir path.
    pub fn path(&self) -> &Path {
        self.workdir.path()
    }

    /// The `.treb/` directory path.
    pub fn treb_dir(&self) -> PathBuf {
        self.workdir.treb_dir()
    }

    /// Get a named Anvil node, if registered.
    pub fn anvil(&self, name: &str) -> Option<&AnvilNode> {
        self.anvil_nodes.get(name)
    }
}
