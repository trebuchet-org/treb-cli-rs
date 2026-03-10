//! Golden-file integration tests for `treb networks`.

mod framework;

use framework::{
    context::TestContext,
    integration_test::{IntegrationTest, run_integration_test},
    normalizer::Normalizer,
};
use std::io::{Read, Write};

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

struct RpcUrlNormalizer;

impl Normalizer for RpcUrlNormalizer {
    fn normalize(&self, input: &str) -> String {
        let re = regex::Regex::new(r"http://127\.0\.0\.1:\d+").unwrap();
        re.replace_all(input, "http://127.0.0.1:<PORT>").into_owned()
    }
}

fn spawn_chain_id_server(chain_id: u64, max_requests: usize) -> String {
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("failed to bind local test RPC server");
    let port = listener.local_addr().unwrap().port();

    std::thread::spawn(move || {
        for _ in 0..max_requests {
            let (mut stream, _) = listener.accept().expect("failed to accept RPC request");
            let mut buf = [0_u8; 4096];
            let _ = stream.read(&mut buf);

            let body = format!(r#"{{"jsonrpc":"2.0","id":1,"result":"0x{chain_id:x}"}}"#);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).expect("failed to write RPC response");
            stream.flush().expect("failed to flush RPC response");
        }
    });

    format!("http://127.0.0.1:{port}")
}

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
/// resolve to a concrete RPC URL, surface the resolved chain ID in human output,
/// and include `chainId` in `--json`.
#[test]
fn networks_resolves_dotenv_rpc_urls() {
    let ctx = TestContext::new("project");
    let rpc_url = spawn_chain_id_server(31337, 2);
    let test = IntegrationTest::new("networks_resolves_dotenv_rpc_urls")
        .pre_setup_hook({
            let rpc_url = rpc_url.clone();
            move |ctx| {
            std::fs::write(
                ctx.path().join("foundry.toml"),
                r#"[profile.default]
src = "src"

[rpc_endpoints]
test = "${TEST_RPC_URL}"
"#,
            )
            .unwrap();
                std::fs::write(ctx.path().join(".env"), format!("TEST_RPC_URL={rpc_url}\n"))
                    .unwrap();
            }
        })
        .extra_normalizer(Box::new(RpcUrlNormalizer))
        .test(&["networks"])
        .test(&["networks", "--json"]);

    run_integration_test(&test, &ctx);
}
