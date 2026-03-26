//! Integration tests for `treb compose`.
//!
//! These tests verify argument parsing, error handling, YAML parsing,
//! validation, cycle detection, and simulation output. Full pipeline execution
//! tests require Solidity compilation and are not covered here.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use regex::Regex;
use std::{
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    path::Path,
};

/// Strip ANSI escape codes (colors, cursor control) and carriage returns
/// from a string for plain-text assertions.
fn strip_ansi(s: &str) -> String {
    let re = Regex::new(r"\x1b\[[0-9;]*[A-Za-z]|\r").unwrap();
    re.replace_all(s, "").to_string()
}

fn treb() -> assert_cmd::Command {
    cargo_bin_cmd!("treb-cli")
}

const MINIMAL_FOUNDRY_TOML: &str = "[profile.default]\n";
const COMPOSE_EXEC_FOUNDRY_TOML: &str = "[profile.default]\nscript = \"script\"\n\n[rpc_endpoints]\nlocalhost = \"http://localhost:8545\"\n";

/// Path to the compose fixtures directory.
fn fixtures_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("compose")
}

fn copy_fixture_to(name: &str, dst: &Path) {
    let src = fixtures_dir().join(name);
    fs::copy(src, dst.join(name)).unwrap();
}

fn write_compose_plan(
    project_root: &Path,
    compose_file: &str,
    chain_id: u64,
    compose_hash: &str,
    components: serde_json::Value,
) {
    let compose_name = Path::new(compose_file).file_name().unwrap().to_string_lossy().to_string();
    let plan_dir = project_root.join("broadcast").join(compose_name).join(chain_id.to_string());
    fs::create_dir_all(&plan_dir).unwrap();
    fs::write(
        plan_dir.join("compose-latest.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "composeFile": compose_file,
            "composeHash": compose_hash,
            "chainId": chain_id,
            "components": components,
        }))
        .unwrap(),
    )
    .unwrap();
}

// ── Help and argument parsing ─────────────────────────────────────────

#[test]
fn compose_help_shows_all_flags() {
    let output =
        treb().args(["compose", "--help"]).output().expect("failed to run treb compose --help");

    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("<FILE>") || stdout.contains("file"), "help should show file arg");
    assert!(stdout.contains("--network"), "help should show --network");
    assert!(stdout.contains("--namespace"), "help should show --namespace");
    assert!(stdout.contains("--profile"), "help should show --profile");
    assert!(stdout.contains("--broadcast"), "help should show --broadcast");
    assert!(stdout.contains("--resume"), "help should show --resume");
    assert!(stdout.contains("--verify"), "help should show --verify");
    assert!(stdout.contains("--json"), "help should show --json");
    assert!(stdout.contains("--env"), "help should show --env");
    assert!(stdout.contains("--non-interactive"), "help should show --non-interactive");
}

#[test]
fn compose_without_file_argument_fails() {
    treb()
        .arg("compose")
        .assert()
        .failure()
        .stderr(predicate::str::contains("<FILE>").or(predicate::str::contains("required")));
}

// ── Missing file ──────────────────────────────────────────────────────

#[test]
fn compose_missing_file_fails() {
    treb()
        .args(["compose", "nonexistent.yaml"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("compose file not found"));
}

#[test]
fn compose_requires_network_or_rpc_url_when_not_configured() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("foundry.toml"), MINIMAL_FOUNDRY_TOML).unwrap();
    treb().arg("init").current_dir(tmp.path()).assert().success();
    fs::write(
        tmp.path().join("simple.yaml"),
        "group: test\ncomponents:\n  deploy:\n    script: script/Deploy.s.sol\n",
    )
    .unwrap();

    treb()
        .args(["compose", "simple.yaml", "--non-interactive"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("no network configured"));
}

// ── Invalid YAML ──────────────────────────────────────────────────────

#[test]
fn compose_invalid_yaml_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let bad_yaml = tmp.path().join("bad.yaml");
    fs::write(&bad_yaml, "not: [valid: yaml: {{").unwrap();

    treb()
        .args(["compose", bad_yaml.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed to parse compose file"));
}

#[test]
fn compose_empty_group_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let yaml = tmp.path().join("empty-group.yaml");
    fs::write(&yaml, "group: \"\"\ncomponents:\n  a:\n    script: script/A.s.sol\n").unwrap();

    treb()
        .args(["compose", yaml.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("group name is required"));
}

#[test]
fn compose_empty_components_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let yaml = tmp.path().join("empty-components.yaml");
    fs::write(&yaml, "group: test\ncomponents: {}\n").unwrap();

    treb()
        .args(["compose", yaml.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("at least one component is required"));
}

// ── Cycle detection ───────────────────────────────────────────────────

#[test]
fn compose_cycle_detection_fails() {
    let fixture = fixtures_dir().join("cycle.yaml");

    treb()
        .args(["compose", fixture.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("circular dependency detected"));
}

// ── Unknown dependency ────────────────────────────────────────────────

#[test]
fn compose_unknown_dependency_fails() {
    let fixture = fixtures_dir().join("bad-dep.yaml");

    treb()
        .args(["compose", fixture.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("depends on non-existent component"));
}

// ── Dry-run ───────────────────────────────────────────────────────────

#[test]
#[ignore] // TODO: compose tests need foundry project setup
fn compose_dry_run_shows_plan() {
    let fixture = fixtures_dir().join("simple.yaml");

    treb().args(["compose", fixture.to_str().unwrap()]).assert().success().stderr(
        predicate::str::contains("Orchestrating")
            .and(predicate::str::contains("Execution plan"))
            .and(predicate::str::contains("token"))
            .and(predicate::str::contains("registry")),
    );
}

#[test]
#[ignore] // TODO: compose tests need foundry project setup
fn compose_dry_run_chain_shows_correct_order() {
    let fixture = fixtures_dir().join("chain.yaml");

    let output = treb()
        .args(["compose", fixture.to_str().unwrap()])
        .output()
        .expect("failed to run compose simulation");

    assert!(output.status.success());

    let stderr = strip_ansi(&String::from_utf8_lossy(&output.stderr));

    // Verify components appear and are in dependency order
    assert!(stderr.contains("libs"), "should show libs");
    assert!(stderr.contains("core"), "should show core");
    assert!(stderr.contains("periphery"), "should show periphery");

    // libs should appear before core, core before periphery
    let libs_pos = stderr.find("1.").expect("step 1 should exist");
    let core_pos = stderr.find("2.").expect("step 2 should exist");
    let periphery_pos = stderr.find("3.").expect("step 3 should exist");
    assert!(libs_pos < core_pos, "libs should come before core");
    assert!(core_pos < periphery_pos, "core should come before periphery");
}

#[test]
#[ignore] // TODO: compose tests need foundry project setup
fn compose_dry_run_diamond_shows_correct_order() {
    let fixture = fixtures_dir().join("diamond.yaml");

    let output = treb()
        .args(["compose", fixture.to_str().unwrap()])
        .output()
        .expect("failed to run compose simulation");

    assert!(output.status.success());

    let stderr = strip_ansi(&String::from_utf8_lossy(&output.stderr));

    // base must be first (step 1), top must be last (step 4)
    assert!(stderr.contains("base"), "should show base");
    assert!(stderr.contains("top"), "should show top");
    assert!(stderr.contains("left"), "should show left");
    assert!(stderr.contains("right"), "should show right");

    // Verify base is step 1 and top is step 4
    assert!(stderr.contains("1. base"), "base should be step 1: {}", stderr);
    assert!(stderr.contains("4. top"), "top should be step 4: {}", stderr);
}

// ── Dry-run --json ────────────────────────────────────────────────────

#[test]
#[ignore] // TODO: compose tests need foundry project setup
fn compose_dry_run_json_is_valid() {
    let fixture = fixtures_dir().join("simple.yaml");

    let output = treb()
        .args(["compose", fixture.to_str().unwrap(), "--json"])
        .output()
        .expect("failed to run compose simulation --json");

    assert!(output.status.success());

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("output is not valid JSON");

    let arr = json.as_array().expect("JSON output should be an array");
    assert_eq!(arr.len(), 2, "simple.yaml has 2 components");

    // Each entry should have step, component, script, deps
    for entry in arr {
        assert!(entry.get("step").is_some(), "entry should have step");
        assert!(entry.get("component").is_some(), "entry should have component");
        assert!(entry.get("script").is_some(), "entry should have script");
        assert!(entry.get("deps").is_some(), "entry should have deps");
    }
}

#[test]
#[ignore] // TODO: compose tests need foundry project setup
fn compose_dry_run_json_chain_has_correct_structure() {
    let fixture = fixtures_dir().join("chain.yaml");

    let output = treb()
        .args(["compose", fixture.to_str().unwrap(), "--json"])
        .output()
        .expect("failed to run compose simulation --json");

    assert!(output.status.success());

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("output is not valid JSON");

    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), 3, "chain.yaml has 3 components");

    // Verify the chain: libs -> core -> periphery
    assert_eq!(arr[0]["component"], "libs");
    assert_eq!(arr[0]["step"], 1);
    assert!(arr[0]["deps"].as_array().unwrap().is_empty());

    assert_eq!(arr[1]["component"], "core");
    assert_eq!(arr[1]["step"], 2);
    assert_eq!(arr[1]["deps"][0], "libs");

    assert_eq!(arr[2]["component"], "periphery");
    assert_eq!(arr[2]["step"], 3);
    assert_eq!(arr[2]["deps"][0], "core");
}

#[test]
#[ignore] // TODO: compose tests need foundry project setup
fn compose_dry_run_json_does_not_include_component_env() {
    let tmp = tempfile::tempdir().unwrap();
    let fixture = tmp.path().join("with-env.yaml");
    fs::write(
        &fixture,
        r#"group: test
components:
  token:
    script: script/DeployToken.s.sol
    env:
      FOO: bar
      BAZ: qux
"#,
    )
    .unwrap();

    let output = treb()
        .args(["compose", fixture.to_str().unwrap(), "--json"])
        .output()
        .expect("failed to run compose simulation --json");

    assert!(output.status.success());

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("output is not valid JSON");
    let arr = json.as_array().expect("JSON output should be an array");
    assert_eq!(arr.len(), 1, "fixture has 1 component");
    assert!(arr[0].get("env").is_none(), "simulation json should not expose component env");
}

#[test]
#[ignore] // TODO: compose tests need foundry project setup
fn compose_dry_run_human_formats_component_env_deterministically() {
    let tmp = tempfile::tempdir().unwrap();
    let fixture = tmp.path().join("with-env.yaml");
    fs::write(
        &fixture,
        r#"group: test
components:
  token:
    script: script/DeployToken.s.sol
    env:
      ZETA: last
      ALPHA: first
"#,
    )
    .unwrap();

    let output = treb()
        .args(["compose", fixture.to_str().unwrap()])
        .output()
        .expect("failed to run compose simulation");

    assert!(output.status.success());

    let stderr = strip_ansi(&String::from_utf8_lossy(&output.stderr));
    assert!(
        stderr.contains(r#"Env: {"ALPHA": "first", "ZETA": "last"}"#),
        "stderr should render env vars in sorted order: {stderr}"
    );
}

// ── Flag acceptance tests ─────────────────────────────────────────────

#[test]
#[ignore] // TODO: compose tests need foundry project setup
fn compose_all_flags_accepted() {
    let fixture = fixtures_dir().join("simple.yaml");

    let output = treb()
        .args([
            "compose",
            fixture.to_str().unwrap(),
            "--network",
            "sepolia",
            "--namespace",
            "production",
            "--profile",
            "optimized",
            "--verify",
            "--slow",
            "--legacy",
            "--verbose",
            "--json",
            "--env",
            "FOO=bar",
            "--env",
            "BAZ=qux",
            "--non-interactive",
        ])
        .output()
        .expect("failed to run compose");

    // Should succeed (simulation mode) with no clap parsing errors
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("error: unexpected argument"),
        "should not have arg parsing error: {stderr}"
    );
    assert!(output.status.success(), "simulation with all flags should succeed");
}

#[test]
#[ignore] // TODO: compose tests need foundry project setup
fn compose_resume_flag_accepted() {
    let fixture = fixtures_dir().join("simple.yaml");

    // --resume with --simulation should parse fine
    let output = treb()
        .args(["compose", fixture.to_str().unwrap(), "--resume"])
        .output()
        .expect("failed to run compose");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("error: unexpected argument"),
        "should not have arg parsing error: {stderr}"
    );
    assert!(output.status.success());
}

// ── Dump-command flag ──────────────────────────────────────────────────

#[test]
fn compose_resume_json_error_stderr_remains_valid_json_when_state_hash_is_stale() {
    let tmp = tempfile::tempdir().unwrap();
    copy_fixture_to("simple.yaml", tmp.path());

    write_compose_plan(
        tmp.path(),
        "simple.yaml",
        1,
        "0000000000000000",
        serde_json::json!([
            {
                "name": "registry",
                "step": 1,
                "script": "script/DeployRegistry.s.sol",
                "status": "broadcast"
            }
        ]),
    );

    let output = treb()
        .args([
            "compose",
            "simple.yaml",
            "--resume",
            "--json",
            "--broadcast",
            "--network",
            "localhost",
            "--non-interactive",
        ])
        .current_dir(tmp.path())
        .output()
        .expect("failed to run compose --resume --json");

    assert!(!output.status.success(), "command should fail without foundry.toml");
    assert!(output.stdout.is_empty(), "stdout should be empty on json error");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let json: serde_json::Value =
        serde_json::from_str(&stderr).expect("stderr should be valid JSON");
    let error = json["error"].as_str().expect("json error should be a string");

    assert!(error.contains("foundry.toml"), "unexpected error: {error}");
    assert!(
        !stderr.contains("compose file has changed"),
        "stderr should not include resume warning in json mode: {stderr}"
    );
}

#[test]
fn compose_json_execution_failure_emits_only_wrapped_json_error() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir_all(tmp.path().join("script")).unwrap();
    fs::write(tmp.path().join("foundry.toml"), COMPOSE_EXEC_FOUNDRY_TOML).unwrap();

    treb().arg("init").current_dir(tmp.path()).assert().success();

    fs::write(
        tmp.path().join("broken.yaml"),
        "group: broken\ncomponents:\n  missing:\n    script: script/NonExistent.s.sol\n",
    )
    .unwrap();

    let output = treb()
        .args([
            "compose",
            "broken.yaml",
            "--broadcast",
            "--network",
            "localhost",
            "--json",
            "--non-interactive",
        ])
        .current_dir(tmp.path())
        .output()
        .expect("failed to run compose execution failure --json");

    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stdout.is_empty(),
        "stdout should stay empty on compose execution failure in json mode"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    let json: serde_json::Value =
        serde_json::from_str(&stderr).expect("stderr should be valid JSON");
    let error = json["error"].as_str().expect("json error should be a string");

    assert!(
        error.contains("compose failed") && error.contains("0/1 completed"),
        "unexpected error: {error}"
    );
    assert!(
        !stderr.contains("Orchestration failed"),
        "stderr should not include human-readable summary lines in json mode: {stderr}"
    );
}

#[test]
fn compose_setup_failure_uses_component_failed_renderer() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir_all(tmp.path().join("script")).unwrap();
    fs::write(tmp.path().join("foundry.toml"), COMPOSE_EXEC_FOUNDRY_TOML).unwrap();

    treb().arg("init").current_dir(tmp.path()).assert().success();

    fs::write(
        tmp.path().join("broken.yaml"),
        "group: broken\ncomponents:\n  missing:\n    script: script/NonExistent.s.sol\n",
    )
    .unwrap();

    let output = treb()
        .args([
            "compose",
            "broken.yaml",
            "--broadcast",
            "--network",
            "localhost",
            "--non-interactive",
        ])
        .current_dir(tmp.path())
        .output()
        .expect("failed to run compose setup failure");

    assert_eq!(output.status.code(), Some(1));

    let stderr = strip_ansi(&String::from_utf8_lossy(&output.stderr));
    // Component failure renderer should show ❌ Failed: line
    assert!(stderr.contains("❌ Failed:"), "missing per-component failure line: {stderr}");
    // The banner should show the component in the plan
    assert!(stderr.contains("[1] missing"), "banner should list the component: {stderr}");
    let separator = "═".repeat(70);
    assert!(stderr.contains(&separator), "summary separator should be present: {stderr}");
    assert!(
        stderr.contains("❌ Orchestration failed"),
        "summary should show orchestration failed: {stderr}"
    );
    assert!(
        stderr.contains("Steps completed: 0/1"),
        "summary should show steps completed: {stderr}"
    );
    assert!(
        stderr.contains("compose failed"),
        "top-level error should mention compose failed: {stderr}"
    );
}

#[test]
fn compose_resume_failure_shows_resume_banner_and_step_start() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir_all(tmp.path().join("script")).unwrap();
    fs::write(tmp.path().join("foundry.toml"), COMPOSE_EXEC_FOUNDRY_TOML).unwrap();

    treb().arg("init").current_dir(tmp.path()).assert().success();

    let compose_path = tmp.path().join("resume.yaml");
    fs::write(
        &compose_path,
        concat!(
            "group: resume\n",
            "components:\n",
            "  done:\n",
            "    script: script/AlreadyDone.s.sol\n",
            "  missing:\n",
            "    script: script/NonExistent.s.sol\n",
        ),
    )
    .unwrap();

    let compose_contents = fs::read_to_string(&compose_path).unwrap();
    let mut hasher = DefaultHasher::new();
    compose_contents.hash(&mut hasher);
    let compose_hash = format!("{:016x}", hasher.finish());
    write_compose_plan(
        tmp.path(),
        "resume.yaml",
        1,
        &compose_hash,
        serde_json::json!([
            {
                "name": "done",
                "step": 1,
                "script": "script/AlreadyDone.s.sol",
                "status": "broadcast"
            },
            {
                "name": "missing",
                "step": 2,
                "script": "script/NonExistent.s.sol",
                "status": "pending"
            }
        ]),
    );

    let output = treb()
        .args([
            "compose",
            "resume.yaml",
            "--resume",
            "--broadcast",
            "--network",
            "localhost",
            "--non-interactive",
        ])
        .current_dir(tmp.path())
        .output()
        .expect("failed to run resumed compose failure");

    assert_eq!(output.status.code(), Some(1));

    let stderr = strip_ansi(&String::from_utf8_lossy(&output.stderr));
    assert!(
        stderr.contains("📂 Resuming compose from step 2 of 2"),
        "missing resume banner: {stderr}"
    );
    // The banner plan should show the resumed component
    assert!(stderr.contains("[2] missing"), "banner should list the resumed component: {stderr}");
    // The done component should be marked as skipped in the plan
    assert!(stderr.contains("(skipped)"), "done component should show as skipped: {stderr}");
}

// ── Compose without init (non-simulation) ────────────────────────────────

#[test]
fn compose_without_init_fails() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("foundry.toml"), MINIMAL_FOUNDRY_TOML).unwrap();
    // Don't run init — but create a valid compose file

    let yaml = tmp.path().join("deploy.yaml");
    fs::write(&yaml, "group: test\ncomponents:\n  a:\n    script: script/A.s.sol\n").unwrap();

    treb()
        .args([
            "compose",
            yaml.to_str().unwrap(),
            "--broadcast",
            "--network",
            "localhost",
            "--non-interactive",
        ])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("treb init").or(predicate::str::contains("foundry.toml")));
}

// ── Self-dependency ───────────────────────────────────────────────────

#[test]
fn compose_self_dependency_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let yaml = tmp.path().join("self-dep.yaml");
    fs::write(
        &yaml,
        "group: test\ncomponents:\n  a:\n    script: script/A.s.sol\n    deps:\n      - a\n",
    )
    .unwrap();

    treb()
        .args(["compose", yaml.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot depend on itself"));
}

// ── Invalid component name ────────────────────────────────────────────

#[test]
fn compose_invalid_component_name_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let yaml = tmp.path().join("bad-name.yaml");
    fs::write(&yaml, "group: test\ncomponents:\n  \"bad name\":\n    script: script/A.s.sol\n")
        .unwrap();

    treb()
        .args(["compose", yaml.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid name"));
}

// ── Empty script ──────────────────────────────────────────────────────

#[test]
fn compose_empty_script_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let yaml = tmp.path().join("empty-script.yaml");
    fs::write(&yaml, "group: test\ncomponents:\n  bad:\n    script: \"\"\n").unwrap();

    treb()
        .args(["compose", yaml.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("must specify a script"));
}
