//! Integration tests for `treb gen-deploy`.
//!
//! These tests verify end-to-end behavior: CLI argument parsing, generated
//! script correctness, Solidity compilation, JSON output, error handling,
//! and custom output paths.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::{fs, path::Path, process::Command};

fn treb() -> assert_cmd::Command {
    cargo_bin_cmd!("treb-cli")
}

/// Recursively copy a directory tree.
fn copy_dir_recursive(src: &Path, dst: &Path) {
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            fs::create_dir_all(&dst_path).unwrap();
            copy_dir_recursive(&src_path, &dst_path);
        } else {
            fs::copy(&src_path, &dst_path).unwrap();
        }
    }
}

/// Copy the gen-deploy fixture project into a fresh temp directory.
fn setup_project() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("gen-deploy-project");
    copy_dir_recursive(&fixture, tmp.path());
    tmp
}

/// Run `forge build` in *dir* and assert compilation succeeds.
fn assert_forge_build_succeeds(dir: &Path) {
    let output = Command::new("forge")
        .arg("build")
        .current_dir(dir)
        .output()
        .expect("forge binary should be available");
    assert!(
        output.status.success(),
        "forge build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

// ── Strategy scripts compile ────────────────────────────────────────────

#[test]
fn gen_deploy_create_create2_create3_scripts_compile() {
    let tmp = setup_project();

    // CREATE — contract with constructor
    treb()
        .args([
            "gen-deploy",
            "Counter",
            "--strategy",
            "create",
            "--output",
            "script/DeployCounter_create.s.sol",
        ])
        .current_dir(tmp.path())
        .assert()
        .success();

    // CREATE — contract without constructor
    treb()
        .args([
            "gen-deploy",
            "SimpleContract",
            "--strategy",
            "create",
            "--output",
            "script/DeploySimple_create.s.sol",
        ])
        .current_dir(tmp.path())
        .assert()
        .success();

    // CREATE — library
    treb()
        .args([
            "gen-deploy",
            "MathLib",
            "--strategy",
            "create",
            "--output",
            "script/DeployMathLib_create.s.sol",
        ])
        .current_dir(tmp.path())
        .assert()
        .success();

    // CREATE2 — contract with constructor
    treb()
        .args([
            "gen-deploy",
            "Counter",
            "--strategy",
            "create2",
            "--output",
            "script/DeployCounter_create2.s.sol",
        ])
        .current_dir(tmp.path())
        .assert()
        .success();

    // CREATE2 — library
    treb()
        .args([
            "gen-deploy",
            "MathLib",
            "--strategy",
            "create2",
            "--output",
            "script/DeployMathLib_create2.s.sol",
        ])
        .current_dir(tmp.path())
        .assert()
        .success();

    // CREATE3 — contract with constructor
    treb()
        .args([
            "gen-deploy",
            "Counter",
            "--strategy",
            "create3",
            "--output",
            "script/DeployCounter_create3.s.sol",
        ])
        .current_dir(tmp.path())
        .assert()
        .success();

    // CREATE3 — contract without constructor
    treb()
        .args([
            "gen-deploy",
            "SimpleContract",
            "--strategy",
            "create3",
            "--output",
            "script/DeploySimple_create3.s.sol",
        ])
        .current_dir(tmp.path())
        .assert()
        .success();

    // All seven generated scripts should compile together.
    assert_forge_build_succeeds(tmp.path());
}

// ── Proxy scripts compile ───────────────────────────────────────────────

#[test]
fn gen_deploy_proxy_scripts_compile() {
    let tmp = setup_project();

    // ERC1967 proxy
    treb()
        .args([
            "gen-deploy",
            "Counter",
            "--proxy",
            "erc1967",
            "--output",
            "script/DeployCounter_erc1967.s.sol",
        ])
        .current_dir(tmp.path())
        .assert()
        .success();

    // UUPS proxy (no constructor contract)
    treb()
        .args([
            "gen-deploy",
            "SimpleContract",
            "--proxy",
            "uups",
            "--output",
            "script/DeploySimple_uups.s.sol",
        ])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Transparent proxy
    treb()
        .args([
            "gen-deploy",
            "Counter",
            "--proxy",
            "transparent",
            "--output",
            "script/DeployCounter_transparent.s.sol",
        ])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Beacon proxy
    treb()
        .args([
            "gen-deploy",
            "Counter",
            "--proxy",
            "beacon",
            "--output",
            "script/DeployCounter_beacon.s.sol",
        ])
        .current_dir(tmp.path())
        .assert()
        .success();

    // CREATE2 + UUPS composition
    treb()
        .args([
            "gen-deploy",
            "Counter",
            "--strategy",
            "create2",
            "--proxy",
            "uups",
            "--output",
            "script/DeployCounter_create2_uups.s.sol",
        ])
        .current_dir(tmp.path())
        .assert()
        .success();

    // CREATE3 + Transparent composition
    treb()
        .args([
            "gen-deploy",
            "Counter",
            "--strategy",
            "create3",
            "--proxy",
            "transparent",
            "--output",
            "script/DeployCounter_create3_transparent.s.sol",
        ])
        .current_dir(tmp.path())
        .assert()
        .success();

    // All six proxy scripts should compile together.
    assert_forge_build_succeeds(tmp.path());
}

// ── JSON output ─────────────────────────────────────────────────────────

#[test]
fn gen_deploy_json_output_has_expected_fields() {
    let tmp = setup_project();

    let output = treb()
        .args(["gen-deploy", "Counter", "--json"])
        .current_dir(tmp.path())
        .output()
        .expect("command should run");

    assert!(output.status.success(), "command should succeed");

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");

    assert_eq!(json["contract_name"], "Counter");
    assert_eq!(json["strategy"], "create");
    assert_eq!(json["output_path"], "script/DeployCounter.s.sol");
    assert!(json["proxy"].is_null());
    assert!(json["code"].is_string());

    let code = json["code"].as_str().unwrap();
    assert!(code.contains("// SPDX-License-Identifier: UNLICENSED"));
    assert!(code.contains("pragma solidity ^0.8.0;"));
    assert!(code.contains("contract DeployCounter is Script"));
}

#[test]
fn gen_deploy_json_output_with_proxy() {
    let tmp = setup_project();

    let output = treb()
        .args(["gen-deploy", "Counter", "--strategy", "create2", "--proxy", "uups", "--json"])
        .current_dir(tmp.path())
        .output()
        .expect("command should run");

    assert!(output.status.success());

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");

    assert_eq!(json["contract_name"], "Counter");
    assert_eq!(json["strategy"], "create2");
    assert_eq!(json["proxy"], "uups");
    assert!(json["code"].as_str().unwrap().contains("UUPSUpgradeable"));
}

#[test]
fn gen_deploy_json_does_not_write_file() {
    let tmp = setup_project();

    treb().args(["gen-deploy", "Counter", "--json"]).current_dir(tmp.path()).assert().success();

    // --json should NOT write a file
    assert!(
        !tmp.path().join("script/DeployCounter.s.sol").exists(),
        "file should not be written when --json is used"
    );
}

// ── Error cases ─────────────────────────────────────────────────────────

#[test]
fn gen_deploy_missing_artifact_argument_fails() {
    treb()
        .arg("gen-deploy")
        .assert()
        .failure()
        .stderr(predicate::str::contains("<ARTIFACT>").or(predicate::str::contains("required")));
}

#[test]
fn gen_deploy_invalid_artifact_lists_available() {
    let tmp = setup_project();

    treb()
        .args(["gen-deploy", "NonExistentContract"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found").and(predicate::str::contains("Counter")));
}

#[test]
fn gen_deploy_invalid_strategy_lists_valid() {
    let tmp = setup_project();

    treb()
        .args(["gen-deploy", "Counter", "--strategy", "invalid"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("invalid strategy")
                .and(predicate::str::contains("create"))
                .and(predicate::str::contains("create2"))
                .and(predicate::str::contains("create3")),
        );
}

#[test]
fn gen_deploy_invalid_proxy_lists_valid() {
    let tmp = setup_project();

    treb()
        .args(["gen-deploy", "Counter", "--proxy", "invalid"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("invalid proxy")
                .and(predicate::str::contains("erc1967"))
                .and(predicate::str::contains("uups"))
                .and(predicate::str::contains("transparent"))
                .and(predicate::str::contains("beacon")),
        );
}

#[test]
fn gen_deploy_library_with_proxy_fails() {
    let tmp = setup_project();

    treb()
        .args(["gen-deploy", "MathLib", "--proxy", "erc1967"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("libraries cannot be deployed behind proxies"));
}

// ── Custom output path ──────────────────────────────────────────────────

#[test]
fn gen_deploy_custom_output_creates_file() {
    let tmp = setup_project();
    let custom_path = "custom/nested/Deploy.s.sol";

    treb()
        .args(["gen-deploy", "Counter", "--output", custom_path])
        .current_dir(tmp.path())
        .assert()
        .success();

    let full_path = tmp.path().join(custom_path);
    assert!(full_path.exists(), "file should be at custom path");

    let content = fs::read_to_string(&full_path).unwrap();
    assert!(content.contains("contract DeployCounter is Script"));
}

#[test]
fn gen_deploy_default_output_path() {
    let tmp = setup_project();

    treb().args(["gen-deploy", "Counter"]).current_dir(tmp.path()).assert().success();

    let default_path = tmp.path().join("script/DeployCounter.s.sol");
    assert!(default_path.exists(), "file should be at default path script/DeployCounter.s.sol");
}

// ── Proxy contract override ─────────────────────────────────────────────

#[test]
fn gen_deploy_custom_proxy_contract_name() {
    let tmp = setup_project();

    let output = treb()
        .args([
            "gen-deploy",
            "Counter",
            "--proxy",
            "erc1967",
            "--proxy-contract",
            "MyCustomProxy",
            "--json",
        ])
        .current_dir(tmp.path())
        .output()
        .expect("command should run");

    assert!(output.status.success());

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");

    let code = json["code"].as_str().unwrap();
    assert!(code.contains("MyCustomProxy"));
    assert!(code.contains("// TODO: Import your custom proxy contract: MyCustomProxy"));
    assert!(!code.contains("@openzeppelin"));
}
