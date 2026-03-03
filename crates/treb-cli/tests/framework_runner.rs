//! Integration tests for `framework::runner::TrebRunner`.

mod framework;

use framework::runner::TrebRunner;
use framework::workdir::TestWorkdir;

fn minimal_fixture() -> std::path::PathBuf {
    TestWorkdir::fixture_dir("minimal-project")
}

#[test]
fn treb_version_succeeds() {
    let fixture = minimal_fixture();
    let w = TestWorkdir::new(&fixture);
    let runner = TrebRunner::new(w.path());

    runner.run(["version"]).success();
}

#[test]
fn run_with_env_passes_env_vars() {
    let fixture = minimal_fixture();
    let w = TestWorkdir::new(&fixture);
    let runner = TrebRunner::new(w.path());

    // Env vars shouldn't break the command — just verify it still works.
    runner
        .run_with_env(["version"], [("MY_TEST_VAR", "hello")])
        .success();
}

#[test]
fn workdir_accessor() {
    let fixture = minimal_fixture();
    let w = TestWorkdir::new(&fixture);
    let runner = TrebRunner::new(w.path());

    assert_eq!(runner.workdir(), w.path());
}

#[test]
fn invalid_subcommand_fails() {
    let fixture = minimal_fixture();
    let w = TestWorkdir::new(&fixture);
    let runner = TrebRunner::new(w.path());

    runner.run(["this-is-not-a-real-command"]).failure();
}
