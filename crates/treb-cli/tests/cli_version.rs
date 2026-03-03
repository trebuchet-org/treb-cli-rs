//! Integration tests for `treb version`.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;

fn treb() -> assert_cmd::Command {
    cargo_bin_cmd!("treb-cli")
}

#[test]
fn version_displays_expected_fields() {
    treb()
        .arg("version")
        .assert()
        .success()
        .stdout(predicate::str::contains("Version"))
        .stdout(predicate::str::contains("Git Commit"))
        .stdout(predicate::str::contains("Build Date"))
        .stdout(predicate::str::contains("Rust Version"))
        .stdout(predicate::str::contains("Forge Version"));
}

#[test]
fn version_json_parses_with_expected_fields() {
    let output = treb()
        .args(["version", "--json"])
        .output()
        .expect("failed to run treb version --json");

    assert!(output.status.success());

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("output is not valid JSON");

    let obj = json.as_object().expect("JSON output is not an object");

    for field in ["version", "git_commit", "build_date", "rust_version", "forge_version"] {
        let val = obj
            .get(field)
            .unwrap_or_else(|| panic!("missing field: {field}"));
        let s = val
            .as_str()
            .unwrap_or_else(|| panic!("field {field} is not a string"));
        assert!(!s.is_empty(), "field {field} is empty");
    }
}
