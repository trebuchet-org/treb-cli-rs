mod framework;

use std::time::Duration;

use framework::anvil_node::AnvilNode;
use treb_forge::anvil::AnvilConfig;

#[tokio::test(flavor = "multi_thread")]
async fn spawn_returns_valid_port_and_rpc_url() {
    let node = AnvilNode::spawn().await.expect("spawn failed");

    assert!(node.port() > 0, "port should be non-zero");
    assert!(
        node.rpc_url().starts_with("http://127.0.0.1:"),
        "rpc_url should be a localhost HTTP URL"
    );
    assert_eq!(node.chain_id(), 31337, "default chain ID should be 31337");
}

#[tokio::test(flavor = "multi_thread")]
async fn spawn_with_config_custom_chain_id() {
    let config = AnvilConfig::new().chain_id(42161).port(0);
    let node = AnvilNode::spawn_with_config(config).await.expect("spawn_with_config failed");

    assert_eq!(node.chain_id(), 42161, "chain ID should match config");
    assert!(node.port() > 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn port_is_reachable() {
    let node = AnvilNode::spawn().await.expect("spawn failed");

    let addr = format!("127.0.0.1:{}", node.port());
    tokio::net::TcpStream::connect(&addr)
        .await
        .expect("RPC port should be reachable");
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_node_frees_port() {
    let port = {
        let node = AnvilNode::spawn().await.expect("spawn failed");
        let p = node.port();

        // Verify reachable while alive
        let addr = format!("127.0.0.1:{p}");
        tokio::net::TcpStream::connect(&addr)
            .await
            .expect("port should be in use");
        p
        // node dropped here
    };

    // Poll until the port is free (up to 2 seconds)
    let addr = format!("127.0.0.1:{port}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if tokio::net::TcpStream::connect(&addr).await.is_err() {
            break; // Port is free
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "port {port} still in use 2 seconds after dropping AnvilNode"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn instance_accessor_provides_api_access() {
    let node = AnvilNode::spawn().await.expect("spawn failed");

    // Use the instance accessor to call snapshot (verifies API access works)
    let _snap = node.instance().snapshot().await.expect("snapshot should succeed");
}
