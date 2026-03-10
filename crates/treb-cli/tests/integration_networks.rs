//! Golden-file integration tests for `treb networks`.

mod framework;

use framework::{
    context::TestContext,
    integration_test::{IntegrationTest, run_integration_test},
};

/// foundry.toml with unresolved bare env var endpoints (no real HTTP calls needed).
const FOUNDRY_TOML_UNRESOLVED: &str = r#"[profile.default]
src = "src"

[rpc_endpoints]
mainnet = "$MAINNET_RPC_URL"
sepolia = "$SEPOLIA_RPC_URL"
"#;

/// foundry.toml with no [rpc_endpoints] section.
const FOUNDRY_TOML_NO_ENDPOINTS: &str = r#"[profile.default]
src = "src"
"#;

/// Networks table with unresolved env vars shows Status column.
#[test]
fn networks_unresolved_env_vars() {
    let ctx = TestContext::new("project");

    let test = IntegrationTest::new("networks_unresolved_env_vars")
        .pre_setup_hook(|ctx| {
            std::fs::write(ctx.path().join("foundry.toml"), FOUNDRY_TOML_UNRESOLVED).unwrap();
        })
        .test(&["networks"]);

    run_integration_test(&test, &ctx);
}

/// Networks JSON output with unresolved env vars has chain_id as null.
#[test]
fn networks_unresolved_json() {
    let ctx = TestContext::new("project");

    let test = IntegrationTest::new("networks_unresolved_json")
        .pre_setup_hook(|ctx| {
            std::fs::write(ctx.path().join("foundry.toml"), FOUNDRY_TOML_UNRESOLVED).unwrap();
        })
        .test(&["networks", "--json"]);

    run_integration_test(&test, &ctx);
}

/// Networks without foundry.toml fails with error.
#[test]
fn networks_no_foundry_toml() {
    let ctx = TestContext::new("project");

    let test = IntegrationTest::new("networks_no_foundry_toml")
        .pre_setup_hook(|ctx| {
            std::fs::remove_file(ctx.path().join("foundry.toml")).ok();
        })
        .test(&["networks"])
        .expect_err(true);

    run_integration_test(&test, &ctx);
}

/// Networks with no endpoints configured shows helpful message.
#[test]
fn networks_no_endpoints() {
    let ctx = TestContext::new("project");

    let test = IntegrationTest::new("networks_no_endpoints")
        .pre_setup_hook(|ctx| {
            std::fs::write(ctx.path().join("foundry.toml"), FOUNDRY_TOML_NO_ENDPOINTS).unwrap();
        })
        .test(&["networks"]);

    run_integration_test(&test, &ctx);
}

/// Networks loads `.env` before parsing foundry endpoints, so `${VAR}` URLs
/// resolve to a concrete RPC URL and no longer report as unresolved.
#[test]
fn networks_resolves_dotenv_rpc_urls() {
    let ctx = TestContext::new("project");
    let test = IntegrationTest::new("networks_resolves_dotenv_rpc_urls")
        .pre_setup_hook(move |ctx| {
            std::fs::write(
                ctx.path().join("foundry.toml"),
                r#"[profile.default]
src = "src"

[rpc_endpoints]
test = "${TEST_RPC_URL}"
"#,
            )
            .unwrap();
            std::fs::write(ctx.path().join(".env"), "TEST_RPC_URL=http://127.0.0.1:8545\n")
                .unwrap();
        })
        .test(&["networks", "--json"]);

    run_integration_test(&test, &ctx);
}
