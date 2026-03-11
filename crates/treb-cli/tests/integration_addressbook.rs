//! Integration tests for `treb addressbook`.

mod framework;

use std::process::Output;

use framework::context::TestContext;
use serde_json::json;

const CHAIN_ID: &str = "1";
const ALPHA: &str = "Alpha";
const ZULU: &str = "Zulu";
const ALPHA_ADDRESS: &str = "0x1111111111111111111111111111111111111111";
const ZULU_ADDRESS: &str = "0x9999999999999999999999999999999999999999";

fn init_project(ctx: &TestContext) {
    ctx.run(["init"]).success();
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout should be valid UTF-8")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr should be valid UTF-8")
}

#[test]
fn set_and_list() {
    let ctx = TestContext::new("project");
    init_project(&ctx);

    ctx.run(["addressbook", "--network", CHAIN_ID, "set", ZULU, ZULU_ADDRESS]).success();
    ctx.run(["addressbook", "--network", CHAIN_ID, "set", ALPHA, ALPHA_ADDRESS]).success();

    let output = ctx
        .run_with_env(["addressbook", "--network", CHAIN_ID, "list"], [("NO_COLOR", "1")])
        .success()
        .get_output()
        .clone();

    assert_eq!(
        stdout(&output),
        format!("  {ALPHA:<24}  {ALPHA_ADDRESS}\n  {ZULU:<24}  {ZULU_ADDRESS}\n")
    );
}

#[test]
fn set_and_list_json() {
    let ctx = TestContext::new("project");
    init_project(&ctx);

    ctx.run(["addressbook", "--network", CHAIN_ID, "set", ZULU, ZULU_ADDRESS]).success();
    ctx.run(["addressbook", "--network", CHAIN_ID, "set", ALPHA, ALPHA_ADDRESS]).success();

    let output = ctx
        .run(["addressbook", "--network", CHAIN_ID, "list", "--json"])
        .success()
        .get_output()
        .clone();
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("list --json output should be valid JSON");

    assert_eq!(
        value,
        json!([
            {"name": ALPHA, "address": ALPHA_ADDRESS},
            {"name": ZULU, "address": ZULU_ADDRESS}
        ])
    );
}

#[test]
fn remove() {
    let ctx = TestContext::new("project");
    init_project(&ctx);

    ctx.run(["addressbook", "--network", CHAIN_ID, "set", ALPHA, ALPHA_ADDRESS]).success();

    ctx.run(["addressbook", "--network", CHAIN_ID, "remove", ALPHA])
        .success()
        .stdout(format!("Removed {ALPHA} (chain {CHAIN_ID})\n"));

    ctx.run_with_env(["addressbook", "--network", CHAIN_ID, "list"], [("NO_COLOR", "1")])
        .success()
        .stdout("No addressbook entries found\n");
}

#[test]
fn remove_not_found() {
    let ctx = TestContext::new("project");
    init_project(&ctx);

    let output = ctx
        .run_with_env(["addressbook", "--network", CHAIN_ID, "remove", ALPHA], [("NO_COLOR", "1")])
        .failure()
        .get_output()
        .clone();

    assert!(
        stderr(&output).contains("addressbook entry 'Alpha' not found on chain 1"),
        "unexpected stderr: {}",
        stderr(&output)
    );
}

#[test]
fn invalid_address() {
    let ctx = TestContext::new("project");
    init_project(&ctx);

    let output = ctx
        .run_with_env(
            ["addressbook", "--network", CHAIN_ID, "set", ALPHA, "0x1234"],
            [("NO_COLOR", "1")],
        )
        .failure()
        .get_output()
        .clone();

    assert!(
        stderr(&output)
            .contains("invalid address \"0x1234\": must be a 0x-prefixed 40-character hex string"),
        "unexpected stderr: {}",
        stderr(&output)
    );
}

#[test]
fn no_network() {
    let ctx = TestContext::new("project");
    init_project(&ctx);

    let output = ctx
        .run_with_env(["addressbook", "list"], [("NO_COLOR", "1")])
        .failure()
        .get_output()
        .clone();

    assert!(
        stderr(&output).contains(
            "no network configured; set one with --network or 'treb config set network <name>'"
        ),
        "unexpected stderr: {}",
        stderr(&output)
    );
}

#[test]
fn default_to_list() {
    let ctx = TestContext::new("project");
    init_project(&ctx);

    ctx.run(["addressbook", "--network", CHAIN_ID, "set", ALPHA, ALPHA_ADDRESS]).success();

    let output = ctx
        .run_with_env(["addressbook", "--network", CHAIN_ID], [("NO_COLOR", "1")])
        .success()
        .get_output()
        .clone();

    assert_eq!(stdout(&output), format!("  {ALPHA:<24}  {ALPHA_ADDRESS}\n"));
}

#[test]
fn empty_list() {
    let ctx = TestContext::new("project");
    init_project(&ctx);

    ctx.run_with_env(["addressbook", "--network", CHAIN_ID, "list"], [("NO_COLOR", "1")])
        .success()
        .stdout("No addressbook entries found\n");
}
