//! Integration tests for `treb completion` and long-form `--help` output.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;

fn treb() -> assert_cmd::Command {
    cargo_bin_cmd!("treb-cli")
}

#[test]
fn completion_bash_exits_zero_and_contains_legacy_completions_subcommand() {
    treb()
        .args(["completion", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("treb").and(predicate::str::contains("completions")));
}

#[test]
fn completion_zsh_exits_zero_and_contains_treb() {
    treb().args(["completion", "zsh"]).assert().success().stdout(predicate::str::contains("treb"));
}

#[test]
fn completion_fish_exits_zero_and_contains_complete_c_treb() {
    treb()
        .args(["completion", "fish"])
        .assert()
        .success()
        .stdout(predicate::str::contains("complete -c treb"));
}

#[test]
fn completion_elvish_exits_zero_and_contains_treb() {
    treb()
        .args(["completion", "elvish"])
        .assert()
        .success()
        .stdout(predicate::str::contains("treb"));
}

#[test]
fn completions_alias_bash_exits_zero_and_contains_treb() {
    treb()
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("treb"));
}

#[test]
fn completion_unsupported_shell_exits_nonzero() {
    treb().args(["completion", "tcsh"]).assert().failure();
}

#[test]
fn run_help_contains_script_argument() {
    treb().args(["run", "--help"]).assert().success().stdout(predicate::str::contains("SCRIPT"));
}

#[test]
fn run_help_contains_broadcast() {
    treb().args(["run", "--help"]).assert().success().stdout(predicate::str::contains("broadcast"));
}
