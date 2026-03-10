//! Golden-file integration tests for `treb config`.

mod framework;

use framework::{
    context::TestContext,
    integration_test::{IntegrationTest, run_integration_test},
    normalizer::PathNormalizer,
};

const TREB_TOML_WITH_ENV_BACKED_SENDER: &str = r#"[accounts.deployer]
type = "private_key"
address = "${TEST_ADDR}"
private_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

[namespace.default]
profile = "default"

[namespace.default.senders]
deployer = "deployer"
"#;

/// Default config show displays Namespace, Network (not set), and inline sender rows.
#[test]
fn config_show_default() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("config_show_default")
        .setup(&["init"])
        .test(&["config", "show"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// JSON config show includes namespace, network, profile, config_source, project_root, senders.
#[test]
fn config_show_json() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("config_show_json")
        .setup(&["init"])
        .test(&["config", "show", "--json"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Config show loads `.env` via config resolution and prints resolved sender addresses.
#[test]
fn config_show_resolves_dotenv_sender_address() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("config_show_resolves_dotenv_sender_address")
        .pre_setup_hook(|ctx| {
            std::fs::write(ctx.path().join("treb.toml"), TREB_TOML_WITH_ENV_BACKED_SENDER)
                .expect("write treb.toml with env-backed sender");
            std::fs::write(
                ctx.path().join(".env"),
                "TEST_ADDR=0x1234567890123456789012345678901234567890\n",
            )
            .expect("write .env with sender address");
        })
        .setup(&["init"])
        .test(&["config", "show"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Set network, then show reflects the updated value; config.local.json artifact confirms
/// persistence.
#[test]
fn config_set_show_round_trip() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("config_set_show_round_trip")
        .setup(&["init"])
        .setup(&["config", "set", "network", "mainnet"])
        .test(&["config", "show"])
        .output_artifact(".treb/config.local.json")
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Remove network key resets to default; show reflects the reset.
#[test]
fn config_remove_show_round_trip() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("config_remove_show_round_trip")
        .setup(&["init"])
        .setup(&["config", "set", "network", "mainnet"])
        .setup(&["config", "remove", "network"])
        .test(&["config", "show"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Config show without init fails with error mentioning `treb init`.
#[test]
fn config_show_uninitialized() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("config_show_uninitialized")
        .pre_setup_hook(|ctx| {
            std::fs::remove_dir_all(ctx.treb_dir()).ok();
        })
        .test(&["config", "show"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// JSON error output: config show --json without init produces structured JSON error on stderr.
#[test]
fn config_show_json_error() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("config_show_json_error")
        .pre_setup_hook(|ctx| {
            std::fs::remove_dir_all(ctx.treb_dir()).ok();
        })
        .test(&["config", "show", "--json"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Config set with an invalid key fails with error listing valid keys.
#[test]
fn config_set_invalid_key() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("config_set_invalid_key")
        .setup(&["init"])
        .test(&["config", "set", "invalid_key", "value"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}
