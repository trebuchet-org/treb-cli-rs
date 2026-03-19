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

/// Strip ANSI escape codes from a string for plain-text assertions.
fn strip_ansi(s: &str) -> String {
    let re = Regex::new(r"\x1b\[[0-9;]*m").unwrap();
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
    fs::create_dir_all(tmp.path().join(".treb")).unwrap();
    copy_fixture_to("simple.yaml", tmp.path());

    fs::write(
        tmp.path().join(".treb").join("compose-state.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "compose_hash": "0000000000000000",
            "completed": ["registry"]
        }))
        .unwrap(),
    )
    .unwrap();

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
#[ignore] // TODO: compose tests need updating after --dry-run removal
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
        .args(["compose", "broken.yaml", "--network", "localhost", "--json", "--non-interactive"])
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
        error.contains("compose failed: component 'missing' failed (0/1 completed)"),
        "unexpected error: {error}"
    );
    assert!(
        !stderr.contains("Component 'missing' failed"),
        "stderr should not include per-component human-readable lines: {stderr}"
    );
}

#[test]
#[ignore] // TODO: compose tests need updating after --dry-run removal
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
            "--network",
            "localhost",
            "--non-interactive",
            "--env",
            "BADENV",
        ])
        .current_dir(tmp.path())
        .output()
        .expect("failed to run compose setup failure");

    assert_eq!(output.status.code(), Some(1));

    let stderr = strip_ansi(&String::from_utf8_lossy(&output.stderr));
    assert!(stderr.contains("[1/1] Starting missing"), "missing step-start line: {stderr}");
    assert!(
        stderr.contains(
            "❌ Failed: invalid --env value 'BADENV': expected KEY=VALUE format (missing '=')"
        ),
        "missing per-component failure line: {stderr}"
    );
    assert_eq!(
        stderr.matches("1. missing → script/NonExistent.s.sol").count(),
        1,
        "execution plan should be the only place that renders the numbered step header: {stderr}"
    );
    let separator = "═".repeat(70);
    assert!(
        stderr.contains(&format!(
            "❌ Failed: invalid --env value 'BADENV': expected KEY=VALUE format (missing '=')\n{separator}\n❌ Orchestration failed"
        )),
        "summary separator should follow the failed step without a blank line: {stderr}"
    );
    assert!(
        stderr.contains("Error: compose failed: component 'missing' failed (0/1 completed)"),
        "unexpected top-level error: {stderr}"
    );
    // The raw error should not appear as a top-level Error: line — it should only
    // appear inside the component failure renderer (❌ Failed: ...) and summary bullet (• Error:
    // ...).
    assert!(
        !stderr.lines().any(|l| l.starts_with("Error: invalid --env value")),
        "setup error should be rendered through the component failure path, not as a top-level error: {stderr}"
    );
}

#[test]
#[ignore] // TODO: compose tests need updating after --dry-run removal
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

    fs::write(
        tmp.path().join(".treb").join("compose-state.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "compose_hash": compose_hash,
            "completed": ["done"],
            "deployment_total": 2
        }))
        .unwrap(),
    )
    .unwrap();

    let output = treb()
        .args(["compose", "resume.yaml", "--resume", "--network", "localhost", "--non-interactive"])
        .current_dir(tmp.path())
        .output()
        .expect("failed to run resumed compose failure");

    assert_eq!(output.status.code(), Some(1));

    let stderr = strip_ansi(&String::from_utf8_lossy(&output.stderr));
    assert!(
        stderr.contains("📂 Resuming compose from step 2 of 2"),
        "missing resume banner: {stderr}"
    );
    assert!(stderr.contains("[2/2] Starting missing"), "missing resumed step-start line: {stderr}");
    assert_eq!(
        stderr.matches("2. missing → script/NonExistent.s.sol").count(),
        1,
        "execution plan should be the only place that renders the numbered step header: {stderr}"
    );
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
