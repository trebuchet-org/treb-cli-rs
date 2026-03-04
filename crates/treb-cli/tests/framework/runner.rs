//! TrebRunner — CLI subprocess wrapper for integration tests.
//!
//! Wraps `assert_cmd::Command` with project defaults: workdir, 60 s timeout,
//! and optional debug output (`TREB_TEST_DEBUG=1`).

use assert_cmd::Command;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    time::Duration,
};

/// Default timeout for CLI invocations.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// CLI subprocess wrapper with sensible defaults for integration tests.
///
/// Every invocation automatically:
/// - sets the working directory to the test workdir
/// - applies a 60-second timeout
/// - prints debug diagnostics to stderr when `TREB_TEST_DEBUG=1`
pub struct TrebRunner {
    workdir: PathBuf,
}

impl TrebRunner {
    /// Create a new runner targeting the given workdir.
    pub fn new(workdir: &Path) -> Self {
        Self { workdir: workdir.to_path_buf() }
    }

    /// Run `treb <args>` with no extra environment variables.
    pub fn run<I, S>(&self, args: I) -> assert_cmd::assert::Assert
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.run_with_env(args, std::iter::empty::<(&str, &str)>())
    }

    /// Run `treb <args>` with additional environment variables.
    pub fn run_with_env<I, S, E, K, V>(&self, args: I, env_vars: E) -> assert_cmd::assert::Assert
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
        E: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        let args: Vec<S> = args.into_iter().collect();

        let debug = std::env::var("TREB_TEST_DEBUG").map(|v| v == "1").unwrap_or(false);

        #[allow(deprecated)]
        let bin_path = assert_cmd::cargo::cargo_bin("treb-cli");

        if debug {
            eprintln!("[TrebRunner] binary: {}", bin_path.display());
            eprintln!(
                "[TrebRunner] args:   {:?}",
                args.iter().map(|a| a.as_ref().to_string_lossy()).collect::<Vec<_>>()
            );
            eprintln!("[TrebRunner] cwd:    {}", self.workdir.display());
        }

        let mut cmd = Command::new(&bin_path);
        cmd.current_dir(&self.workdir).timeout(DEFAULT_TIMEOUT).args(args);

        for (k, v) in env_vars {
            cmd.env(k, v);
        }

        let assertion = cmd.assert();

        if debug {
            eprintln!("[TrebRunner] exit:   {:?}", assertion.get_output().status);
        }

        assertion
    }

    /// Returns the workdir path.
    pub fn workdir(&self) -> &Path {
        &self.workdir
    }
}
