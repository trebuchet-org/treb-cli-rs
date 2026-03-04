//! Compilation verification for the `project` fixture.
//!
//! These tests are `#[ignore]` because they require `forge` to be installed.
//! Run them explicitly with: `cargo test -p treb-cli --test fixture_compile -- --ignored`

mod framework;

use framework::workdir::TestWorkdir;
use std::process::Command;

fn project_fixture() -> std::path::PathBuf {
    TestWorkdir::fixture_dir("project")
}

/// Verify that `forge build` succeeds on the project fixture.
///
/// This ensures all Solidity source contracts, library stubs, remappings,
/// and the deploy script compile without errors.
#[test]
#[ignore]
fn fixture_forge_build() {
    let fixture = project_fixture();
    let w = TestWorkdir::new(&fixture);

    let output = Command::new("forge")
        .arg("build")
        .current_dir(w.path())
        .output()
        .expect("failed to execute forge — is it installed?");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "forge build failed.\nstdout:\n{stdout}\nstderr:\n{stderr}");
}
