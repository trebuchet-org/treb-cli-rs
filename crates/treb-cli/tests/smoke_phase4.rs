//! Phase 4 smoke tests — validate the "project" fixture wiring in TestWorkdir.
//!
//! Verifies that the fixture is correctly assembled into a temp workspace:
//! expected files are present, symlinks resolve, port rewriting works, and
//! treb.toml is copied with the right content.

mod framework;

use framework::workdir::{port_rewrite_foundry_toml, TestWorkdir, DEFAULT_FIXTURE};

use std::fs;

// ---------------------------------------------------------------------------
// Smoke tests
// ---------------------------------------------------------------------------

/// All expected files and directories are present in the temp workspace.
#[test]
fn fixture_workdir_structure() {
    let fixture = TestWorkdir::fixture_dir(DEFAULT_FIXTURE);
    let wd = TestWorkdir::new(&fixture);
    let root = wd.path();

    // Copied items (mutable).
    assert!(root.join("foundry.toml").is_file(), "foundry.toml missing");
    assert!(root.join("treb.toml").is_file(), "treb.toml missing");
    assert!(root.join("src").is_dir(), "src/ missing");
    assert!(root.join("script").is_dir(), "script/ missing");
    assert!(
        root.join("script/Deploy.s.sol").is_file(),
        "script/Deploy.s.sol missing"
    );
    assert!(
        root.join("script/deploy/.gitkeep").is_file(),
        "script/deploy/.gitkeep missing"
    );

    // Symlinked items (immutable).
    assert!(root.join("lib").exists(), "lib/ missing");
    assert!(
        root.join("lib").read_link().is_ok(),
        "lib/ should be a symlink"
    );
    assert!(root.join("remappings.txt").exists(), "remappings.txt missing");
    assert!(
        root.join("remappings.txt").read_link().is_ok(),
        "remappings.txt should be a symlink"
    );
    assert!(root.join(".gitignore").exists(), ".gitignore missing");
    assert!(
        root.join(".gitignore").read_link().is_ok(),
        ".gitignore should be a symlink"
    );

    // Auto-created directory.
    assert!(root.join(".treb").is_dir(), ".treb/ missing");

    // Source contracts.
    let expected_sources = [
        "Counter.sol",
        "SampleToken.sol",
        "StringUtils.sol",
        "StringUtilsV2.sol",
        "UpgradeableCounter.sol",
        "MessageStorageV07.sol",
        "MessageStorageV08.sol",
    ];
    for name in &expected_sources {
        assert!(
            root.join("src").join(name).is_file(),
            "src/{name} missing"
        );
    }

    // Symlinked lib contents resolve.
    assert!(
        root.join("lib/forge-std/src/Script.sol").is_file(),
        "lib/forge-std/src/Script.sol not reachable through symlink"
    );
}

/// Port rewriting replaces both anvil-31337 and anvil-31338 endpoints.
#[test]
fn fixture_foundry_toml_port_rewrite() {
    let fixture = TestWorkdir::fixture_dir(DEFAULT_FIXTURE);
    let wd = TestWorkdir::new(&fixture);
    let root = wd.path();

    // Verify original ports are present.
    let before = fs::read_to_string(root.join("foundry.toml")).unwrap();
    assert!(
        before.contains("localhost:8545"),
        "original should have port 8545"
    );
    assert!(
        before.contains("localhost:9545"),
        "original should have port 9545"
    );

    // Rewrite both ports.
    port_rewrite_foundry_toml(root, &[(8545, 11111), (9545, 22222)]).unwrap();

    let after = fs::read_to_string(root.join("foundry.toml")).unwrap();
    assert!(
        after.contains("localhost:11111"),
        "port 8545 should be rewritten to 11111"
    );
    assert!(
        after.contains("localhost:22222"),
        "port 9545 should be rewritten to 22222"
    );
    assert!(
        !after.contains("localhost:8545"),
        "old port 8545 should be gone"
    );
    assert!(
        !after.contains("localhost:9545"),
        "old port 9545 should be gone"
    );
}

/// treb.toml is copied and contains the expected anvil account configuration.
#[test]
fn fixture_treb_toml_present() {
    let fixture = TestWorkdir::fixture_dir(DEFAULT_FIXTURE);
    let wd = TestWorkdir::new(&fixture);
    let root = wd.path();

    let content = fs::read_to_string(root.join("treb.toml")).unwrap();
    assert!(
        content.contains("[accounts.anvil]"),
        "treb.toml should have [accounts.anvil]"
    );
    assert!(
        content.contains("0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"),
        "treb.toml should contain Anvil's well-known private key"
    );
}
