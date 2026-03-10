//! Golden-file integration tests for CLI help output.

mod framework;

use framework::{
    context::TestContext,
    integration_test::{IntegrationTest, run_integration_test},
};

/// Root help keeps grouped command listings aligned with CLI compatibility aliases.
#[test]
fn help_root() {
    let ctx = TestContext::new("minimal-project");
    let test = IntegrationTest::new("help_root").test(&["--help"]);

    run_integration_test(&test, &ctx);
}

/// `gen --help` should continue to show the nested `deploy` structure.
#[test]
fn help_gen() {
    let ctx = TestContext::new("minimal-project");
    let test = IntegrationTest::new("help_gen").test(&["gen", "--help"]);

    run_integration_test(&test, &ctx);
}

/// `completion --help` should stay singular in help output.
#[test]
fn help_completion() {
    let ctx = TestContext::new("minimal-project");
    let test = IntegrationTest::new("help_completion").test(&["completion", "--help"]);

    run_integration_test(&test, &ctx);
}

/// `config --help` should document the default-to-show behavior.
#[test]
fn help_config() {
    let ctx = TestContext::new("minimal-project");
    let test = IntegrationTest::new("help_config").test(&["config", "--help"]);

    run_integration_test(&test, &ctx);
}

/// `list --help` should surface the Go-compatible short filter flags.
#[test]
fn help_list() {
    let ctx = TestContext::new("minimal-project");
    let test = IntegrationTest::new("help_list").test(&["list", "--help"]);

    run_integration_test(&test, &ctx);
}

/// `show --help` should document deployment query scoping flags.
#[test]
fn help_show() {
    let ctx = TestContext::new("minimal-project");
    let test = IntegrationTest::new("help_show").test(&["show", "--help"]);

    run_integration_test(&test, &ctx);
}

/// `verify --help` should snapshot the full flag-parity surface.
#[test]
fn help_verify() {
    let ctx = TestContext::new("minimal-project");
    let test = IntegrationTest::new("help_verify").test(&["verify", "--help"]);

    run_integration_test(&test, &ctx);
}

/// `fork enter --help` should show the positional network form plus legacy aliases.
#[test]
fn help_fork_enter() {
    let ctx = TestContext::new("minimal-project");
    let test = IntegrationTest::new("help_fork_enter").test(&["fork", "enter", "--help"]);

    run_integration_test(&test, &ctx);
}
