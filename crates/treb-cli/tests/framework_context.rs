//! Smoke tests for `framework::context::TestContext`.

mod framework;

use framework::context::TestContext;

#[test]
fn treb_version_via_context() {
    let ctx = TestContext::new("minimal-project");
    ctx.run(["version"]).success();
}

#[test]
fn treb_init_creates_treb_directory_structure() {
    let ctx = TestContext::new("minimal-project");

    // Remove the auto-created .treb/ so init has something to do.
    std::fs::remove_dir_all(ctx.treb_dir()).expect("remove .treb");

    ctx.run(["init"]).success();

    // init should recreate .treb/ with registry files.
    assert!(ctx.treb_dir().exists(), ".treb/ should exist after init");
    assert!(ctx.treb_dir().join("registry.json").exists(), "registry.json should exist after init");
    assert!(
        ctx.treb_dir().join("config.local.json").exists(),
        "config.local.json should exist after init"
    );
}

#[test]
fn run_with_env_delegates_correctly() {
    let ctx = TestContext::new("minimal-project");

    // Passing env vars shouldn't break the command.
    ctx.run_with_env(["version"], [("MY_TEST_VAR", "hello")]).success();
}

#[test]
fn path_and_treb_dir_accessors() {
    let ctx = TestContext::new("minimal-project");

    assert!(ctx.path().exists(), "workdir path should exist");
    assert!(ctx.treb_dir().exists(), ".treb/ should exist");
    assert_eq!(ctx.treb_dir(), ctx.path().join(".treb"));
}

#[tokio::test(flavor = "multi_thread")]
async fn anvil_node_reachable_via_context() {
    let ctx =
        TestContext::new("minimal-project").with_anvil("local").await.expect("with_anvil failed");

    let node = ctx.anvil("local").expect("node should be registered");
    assert!(node.port() > 0, "port should be non-zero");

    // Verify the node is reachable.
    let addr = format!("127.0.0.1:{}", node.port());
    tokio::net::TcpStream::connect(&addr).await.expect("anvil node should be reachable");
}

#[tokio::test(flavor = "multi_thread")]
async fn anvil_port_rewrite_applied() {
    let ctx =
        TestContext::new("minimal-project").with_anvil("local").await.expect("with_anvil failed");

    let node = ctx.anvil("local").unwrap();
    let port_str = node.port().to_string();

    // Read foundry.toml and verify port was rewritten.
    let content =
        std::fs::read_to_string(ctx.path().join("foundry.toml")).expect("read foundry.toml");

    // The fixture uses port 8545 — it should now contain the Anvil port.
    assert!(
        content.contains(&port_str),
        "foundry.toml should contain the anvil port {port_str}, got:\n{content}"
    );
    assert!(!content.contains("8545"), "foundry.toml should no longer contain port 8545");
}
