//! Integration tests for treb-forge compilation and artifact indexing.
//!
//! These tests compile a real Foundry project fixture and verify the
//! compilation pipeline, artifact indexing, and version detection work end-to-end.

use std::path::PathBuf;

use foundry_config::Config;
use treb_forge::{compile_project, detect_forge_version, ArtifactIndex};

fn fixture_config() -> Config {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sample-project");
    Config::load_with_root(path)
        .expect("fixture config should load")
        .sanitized()
}

#[test]
fn compile_project_against_fixture_succeeds() {
    let config = fixture_config();
    let output = compile_project(&config).expect("compilation should succeed");

    assert!(
        !output.has_compiler_errors(),
        "compilation should produce no errors"
    );

    // Verify Counter artifact is present
    let artifact_names: Vec<String> = output.artifact_ids().map(|id| id.name.clone()).collect();
    assert!(
        artifact_names.contains(&"Counter".to_string()),
        "expected Counter artifact, found: {artifact_names:?}"
    );
}

#[test]
fn artifact_index_find_by_name_counter() {
    let config = fixture_config();
    let output = compile_project(&config).expect("compilation should succeed");

    let index = ArtifactIndex::from_compile_output(output);
    let result = index
        .find_by_name("Counter")
        .expect("find_by_name should not error");

    assert!(result.is_some(), "Counter should be found in artifact index");
    let artifact = result.unwrap();
    assert_eq!(artifact.name, "Counter");
    assert!(artifact.has_bytecode, "Counter should have bytecode");
    assert!(
        artifact.has_deployed_bytecode,
        "Counter should have deployed bytecode"
    );
}

#[test]
fn detect_forge_version_returns_non_empty() {
    let version = detect_forge_version();
    assert!(
        !version.version.is_empty(),
        "forge version should not be empty"
    );

    let display = version.display_string();
    assert!(
        display.starts_with("forge v"),
        "display string should start with 'forge v', got: {display}"
    );
}
