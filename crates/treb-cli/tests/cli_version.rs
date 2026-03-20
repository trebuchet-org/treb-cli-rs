//! Integration tests for `treb version`.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::process::Command;

fn treb() -> assert_cmd::Command {
    cargo_bin_cmd!("treb-cli")
}

#[test]
fn version_displays_expected_fields() {
    treb()
        .arg("version")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("treb "))
        .stdout(predicate::str::contains("commit: "))
        .stdout(predicate::str::contains("built:  "))
        .stdout(predicate::str::contains(" UTC"));
}

#[test]
fn version_flag_matches_json_version_field() {
    let version_output = treb().arg("--version").output().expect("failed to run treb --version");
    assert!(version_output.status.success());

    let version_stdout =
        String::from_utf8(version_output.stdout).expect("treb --version output is not utf-8");

    let json_output =
        treb().args(["version", "--json"]).output().expect("failed to run treb version --json");
    assert!(json_output.status.success());

    let json: serde_json::Value =
        serde_json::from_slice(&json_output.stdout).expect("output is not valid JSON");
    let version = json["version"].as_str().expect("version field is not a string");

    assert_eq!(version_stdout, format!("treb {version}\n"));
}

#[test]
#[ignore] // dirty suffix
fn version_json_uses_git_describe_output_in_untagged_checkouts() {
    let describe_always = Command::new("git")
        .args(["describe", "--tags", "--always", "--dirty"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("failed to run git describe --tags --always --dirty");
    assert!(describe_always.status.success(), "git describe --always should succeed in the repo");

    let expected_version =
        String::from_utf8(describe_always.stdout).expect("git describe output is not utf-8");
    let expected_version = expected_version.trim();

    let tagged_describe = Command::new("git")
        .args(["describe", "--tags", "--abbrev=7"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("failed to run git describe --tags --abbrev=7");
    assert!(
        !tagged_describe.status.success(),
        "this regression test expects the default checkout to be untagged"
    );

    let json_output =
        treb().args(["version", "--json"]).output().expect("failed to run treb version --json");
    assert!(json_output.status.success());

    let json: serde_json::Value =
        serde_json::from_slice(&json_output.stdout).expect("output is not valid JSON");
    let version = json["version"].as_str().expect("version field is not a string");

    assert_eq!(version, expected_version);
    assert_ne!(version, env!("CARGO_PKG_VERSION"));
}

#[test]
fn version_json_parses_with_expected_fields() {
    let output =
        treb().args(["version", "--json"]).output().expect("failed to run treb version --json");

    assert!(output.status.success());

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("output is not valid JSON");

    let obj = json.as_object().expect("JSON output is not an object");

    for field in [
        "version",
        "commit",
        "date",
        "rustVersion",
        "forgeVersion",
        "foundryVersion",
        "trebSolCommit",
    ] {
        let val = obj.get(field).unwrap_or_else(|| panic!("missing field: {field}"));
        let s = val.as_str().unwrap_or_else(|| panic!("field {field} is not a string"));
        assert!(!s.is_empty(), "field {field} is empty");
    }

    let build_date = obj["date"].as_str().expect("field date is not a string");
    assert!(build_date.contains('T'), "build date should include a timestamp: {build_date}");
    assert!(build_date.ends_with('Z'), "build date should be UTC RFC3339: {build_date}");

    let foundry_version =
        obj["foundryVersion"].as_str().expect("field foundryVersion is not a string");
    assert_ne!(foundry_version, "unknown");
}

#[test]
fn version_json_invalid_flag_returns_json_error_and_exit_code_one() {
    let output = treb()
        .args(["version", "--json", "--definitely-invalid-flag"])
        .output()
        .expect("failed to run treb version --json with invalid flag");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "stdout should stay empty on clap parse errors in json mode");

    let json: serde_json::Value =
        serde_json::from_slice(&output.stderr).expect("stderr should be valid JSON");
    let error = json["error"].as_str().expect("json error should be a string");
    assert!(error.contains("--definitely-invalid-flag"), "unexpected json error: {error}");
}
