//! Integration tests for `treb completions` and long-form `--help` output.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;

fn treb() -> assert_cmd::Command {
    cargo_bin_cmd!("treb-cli")
}

#[test]
fn completions_bash_exits_zero_and_contains_treb() {
    treb()
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("treb"));
}

#[test]
fn completions_zsh_exits_zero_and_contains_treb() {
    treb()
        .args(["completions", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("treb"));
}

#[test]
fn completions_fish_exits_zero_and_contains_complete_c_treb() {
    treb()
        .args(["completions", "fish"])
        .assert()
        .success()
        .stdout(predicate::str::contains("complete -c treb"));
}

#[test]
fn completions_elvish_exits_zero_and_contains_treb() {
    treb()
        .args(["completions", "elvish"])
        .assert()
        .success()
        .stdout(predicate::str::contains("treb"));
}

#[test]
fn completions_unsupported_shell_exits_nonzero() {
    treb()
        .args(["completions", "tcsh"])
        .assert()
        .failure();
}

#[test]
fn run_help_contains_dry_run() {
    treb()
        .args(["run", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("dry-run"));
}

#[test]
fn run_help_contains_broadcast() {
    treb()
        .args(["run", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("broadcast"));
}
