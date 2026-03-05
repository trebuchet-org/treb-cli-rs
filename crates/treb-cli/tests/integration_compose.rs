//! Golden-file integration tests for `treb compose`.
//!
//! Tests exercise dry-run plans (human-readable and JSON output) for various
//! dependency topologies (single, simple, chain, diamond) and error paths
//! (file not found, invalid YAML, empty components, cycle detection, unknown
//! dependency, self-dependency).
//!
//! Compose dry-run does NOT require `treb init` or a Foundry project — it only
//! parses the compose YAML and displays the execution plan.

mod framework;

use std::{
    fs,
    path::{Path, PathBuf},
};

use framework::{
    context::TestContext,
    integration_test::{IntegrationTest, run_integration_test},
    normalizer::PathNormalizer,
};

/// Path to the compose YAML fixture files.
fn compose_fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("compose")
}

/// Copy a compose YAML fixture file into the test's working directory.
fn copy_compose_fixture(name: &str, ctx: &TestContext) {
    let src = compose_fixtures_dir().join(name);
    let dst = ctx.path().join(name);
    std::fs::copy(&src, &dst).unwrap_or_else(|e| panic!("copy compose fixture {name}: {e}"));
}

/// Write inline YAML content as a compose file in the test's working directory.
fn write_compose_fixture(name: &str, content: &str, ctx: &TestContext) {
    let dst = ctx.path().join(name);
    std::fs::write(&dst, content).unwrap_or_else(|e| panic!("write compose fixture {name}: {e}"));
}

// ── Dry-run tests (human-readable) ──────────────────────────────────────

/// Dry-run with a single component (no dependencies).
///
/// Verifies the execution plan header, step numbering, and "(no dependencies)"
/// annotation for a lone component.
#[test]
fn compose_dry_run_single() {
    let ctx = TestContext::new("compose-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("compose_dry_run_single")
        .pre_setup_hook(|ctx| copy_compose_fixture("single.yaml", ctx))
        .test(&["compose", "single.yaml", "--dry-run"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Dry-run with two independent components (no dependencies).
///
/// Verifies both components appear in the plan with alphabetical ordering
/// and "(no dependencies)" annotations.
#[test]
fn compose_dry_run_simple() {
    let ctx = TestContext::new("compose-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("compose_dry_run_simple")
        .pre_setup_hook(|ctx| copy_compose_fixture("simple.yaml", ctx))
        .test(&["compose", "simple.yaml", "--dry-run"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Dry-run with a linear dependency chain (libs → core → periphery).
///
/// Verifies components appear in dependency order: libs first, periphery last.
#[test]
fn compose_dry_run_chain() {
    let ctx = TestContext::new("compose-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("compose_dry_run_chain")
        .pre_setup_hook(|ctx| copy_compose_fixture("chain.yaml", ctx))
        .test(&["compose", "chain.yaml", "--dry-run"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Dry-run with a diamond dependency pattern (base → left/right → top).
///
/// Verifies base is step 1 and top is step 4, with left and right in
/// alphabetical order as steps 2 and 3.
#[test]
fn compose_dry_run_diamond() {
    let ctx = TestContext::new("compose-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("compose_dry_run_diamond")
        .pre_setup_hook(|ctx| copy_compose_fixture("diamond.yaml", ctx))
        .test(&["compose", "diamond.yaml", "--dry-run"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── Dry-run tests (JSON output) ─────────────────────────────────────────

/// JSON dry-run with two independent components.
///
/// Verifies output is a valid JSON array with step, component, script, deps
/// fields for each entry.
#[test]
fn compose_dry_run_json_simple() {
    let ctx = TestContext::new("compose-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("compose_dry_run_json_simple")
        .pre_setup_hook(|ctx| copy_compose_fixture("simple.yaml", ctx))
        .test(&["compose", "simple.yaml", "--dry-run", "--json"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// JSON dry-run with a linear dependency chain.
///
/// Verifies JSON array shows correct step ordering and deps arrays
/// matching the chain topology.
#[test]
fn compose_dry_run_json_chain() {
    let ctx = TestContext::new("compose-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("compose_dry_run_json_chain")
        .pre_setup_hook(|ctx| copy_compose_fixture("chain.yaml", ctx))
        .test(&["compose", "chain.yaml", "--dry-run", "--json"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Dry-run resume + verbose shows hash/skip context and marks completed step.
#[test]
fn compose_dry_run_resume_verbose() {
    let ctx = TestContext::new("compose-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("compose_dry_run_resume_verbose")
        .pre_setup_hook(|ctx| {
            copy_compose_fixture("simple.yaml", ctx);
            fs::create_dir_all(ctx.treb_dir()).unwrap();
            let state = serde_json::json!({
                "compose_hash": "deadbeef",
                "completed": ["registry"]
            });
            fs::write(
                ctx.treb_dir().join("compose-state.json"),
                serde_json::to_string_pretty(&state).unwrap(),
            )
            .unwrap();
        })
        .test(&["compose", "simple.yaml", "--dry-run", "--resume", "--verbose"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── Error path tests ────────────────────────────────────────────────────

/// Error: compose file does not exist.
///
/// Verifies error message mentions "compose file not found".
#[test]
fn compose_error_file_not_found() {
    let ctx = TestContext::new("compose-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("compose_error_file_not_found")
        .test(&["compose", "nonexistent.yaml", "--dry-run"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Error: malformed YAML content.
///
/// Verifies error message mentions "failed to parse compose file".
#[test]
fn compose_error_invalid_yaml() {
    let ctx = TestContext::new("compose-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("compose_error_invalid_yaml")
        .pre_setup_hook(|ctx| {
            write_compose_fixture("invalid.yaml", "not: [valid: yaml: {{", ctx);
        })
        .test(&["compose", "invalid.yaml", "--dry-run"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Error: compose file has empty components map.
///
/// Verifies error message mentions "'components' must not be empty".
#[test]
fn compose_error_empty_components() {
    let ctx = TestContext::new("compose-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("compose_error_empty_components")
        .pre_setup_hook(|ctx| {
            write_compose_fixture("empty-components.yaml", "group: test\ncomponents: {}\n", ctx);
        })
        .test(&["compose", "empty-components.yaml", "--dry-run"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Error: dependency cycle among components (alpha → beta → gamma → alpha).
///
/// Verifies error message mentions "dependency cycle detected".
#[test]
fn compose_error_cycle() {
    let ctx = TestContext::new("compose-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("compose_error_cycle")
        .pre_setup_hook(|ctx| copy_compose_fixture("cycle.yaml", ctx))
        .test(&["compose", "cycle.yaml", "--dry-run"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Error: component depends on a non-existent component.
///
/// Verifies error message mentions "depends on unknown component".
#[test]
fn compose_error_unknown_dep() {
    let ctx = TestContext::new("compose-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("compose_error_unknown_dep")
        .pre_setup_hook(|ctx| copy_compose_fixture("bad-dep.yaml", ctx))
        .test(&["compose", "bad-dep.yaml", "--dry-run"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Error: component depends on itself.
///
/// Verifies error message mentions "cannot depend on itself".
#[test]
fn compose_error_self_dep() {
    let ctx = TestContext::new("compose-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("compose_error_self_dep")
        .pre_setup_hook(|ctx| {
            write_compose_fixture(
                "self-dep.yaml",
                "group: test\ncomponents:\n  a:\n    script: script/A.s.sol\n    deps:\n      - a\n",
                ctx,
            );
        })
        .test(&["compose", "self-dep.yaml", "--dry-run"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}
