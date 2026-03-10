//! Focused compatibility coverage for renamed CLI commands and shorthand forms.

mod helpers;

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::{fs, path::Path};

fn treb() -> assert_cmd::Command {
    cargo_bin_cmd!("treb-cli")
}

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

fn setup_gen_deploy_project() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("gen-deploy-project");
    copy_dir_recursive(&fixture, tmp.path());
    tmp
}

const MINIMAL_FOUNDRY_TOML: &str = "[profile.default]\n";

fn setup_config_project() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("foundry.toml"), MINIMAL_FOUNDRY_TOML).unwrap();
    treb().arg("init").current_dir(tmp.path()).assert().success();
    tmp
}

fn setup_seeded_config_project() -> tempfile::TempDir {
    let tmp = setup_config_project();
    helpers::seed_registry(tmp.path());
    tmp
}

#[test]
fn gen_deploy_json_output_is_valid_for_primary_and_compat_forms() {
    let tmp = setup_gen_deploy_project();

    let run = |args: &[&str]| -> Vec<u8> {
        let output =
            treb().args(args).current_dir(tmp.path()).output().expect("command should run");

        assert!(output.status.success(), "command should succeed for args: {args:?}");
        let _: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON");
        output.stdout
    };

    let nested = run(&["gen", "deploy", "Counter", "--json"]);
    let alias = run(&["generate", "deploy", "Counter", "--json"]);
    let compat = run(&["gen-deploy", "Counter", "--json"]);

    assert_eq!(nested, alias, "generate alias should match gen deploy output");
    assert_eq!(nested, compat, "gen-deploy compatibility command should match gen deploy output");
}

#[test]
fn completion_bash_primary_and_compat_forms_succeed() {
    let primary =
        treb().args(["completion", "bash"]).output().expect("completion command should run");
    assert!(primary.status.success(), "treb completion bash should succeed");
    assert!(
        String::from_utf8_lossy(&primary.stdout).contains("treb"),
        "bash completions should mention treb"
    );

    let compat = treb()
        .args(["completions", "bash"])
        .output()
        .expect("legacy completions command should run");
    assert!(compat.status.success(), "treb completions bash should succeed");
    assert_eq!(
        primary.stdout, compat.stdout,
        "legacy completions alias should match completion output"
    );
}

#[test]
fn bare_config_matches_config_show() {
    let tmp = setup_config_project();

    let bare = treb().args(["config"]).current_dir(tmp.path()).output().expect("run treb config");
    let explicit = treb()
        .args(["config", "show"])
        .current_dir(tmp.path())
        .output()
        .expect("run treb config show");

    assert!(bare.status.success(), "treb config should succeed");
    assert!(explicit.status.success(), "treb config show should succeed");

    let bare_stdout = String::from_utf8_lossy(&bare.stdout);
    assert!(
        bare_stdout.contains("Current config"),
        "bare config output should include the config summary"
    );
    assert_eq!(bare.stdout, explicit.stdout);
    assert_eq!(bare.stderr, explicit.stderr);
}

#[test]
fn compatibility_suite_still_exposes_completion_output_shape() {
    treb().args(["completion", "bash"]).assert().success().stdout(predicate::str::contains("treb"));
}

#[test]
fn list_short_flags_match_long_filter_output() {
    let tmp = setup_seeded_config_project();

    let long = treb()
        .args(["list", "--json", "--network", "1", "--namespace", "mainnet"])
        .current_dir(tmp.path())
        .output()
        .expect("run long-form list filters");
    let short = treb()
        .args(["list", "--json", "-n", "1", "-s", "mainnet"])
        .current_dir(tmp.path())
        .output()
        .expect("run short-form list filters");

    assert!(long.status.success(), "long-form list filters should succeed");
    assert!(short.status.success(), "short-form list filters should succeed");
    assert_eq!(long.stdout, short.stdout, "short-form list filters should match long-form stdout");
    assert_eq!(long.stderr, short.stderr, "short-form list filters should match long-form stderr");
}
