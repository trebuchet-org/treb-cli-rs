//! Golden-file integration tests for `treb sync`.
//!
//! Tests exercise empty-state paths: no safe transactions (plain/JSON),
//! network filter with no matches, uninitialized project, and no foundry project.

mod framework;
mod helpers;

use std::io::{Read, Write};

use alloy_primitives::{Address, Bytes};
use framework::{
    context::TestContext,
    integration_test::{IntegrationTest, run_integration_test},
    normalizer::PathNormalizer,
};
use treb_forge::anvil::AnvilConfig;

// ── Tests ────────────────────────────────────────────────────────────────

/// Empty registry with no pending entries uses the Go-style summary output.
#[test]
fn sync_no_safe_txs() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("sync_no_safe_txs")
        .setup(&["init"])
        .test(&["registry", "sync"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Empty registry with --network filter still uses the shared summary output.
#[test]
fn sync_no_safe_txs_network_filter() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("sync_no_safe_txs_network_filter")
        .setup(&["init"])
        .test(&["registry", "sync", "--network", "1"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Empty registry with --json outputs zero-count JSON.
#[test]
fn sync_json_empty() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("sync_json_empty")
        .setup(&["init"])
        .test(&["registry", "sync", "--json"])
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Uninitialized project (no .treb/) produces an error.
#[test]
fn sync_uninitialized() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("sync_uninitialized")
        .pre_setup_hook(|ctx| {
            std::fs::remove_dir_all(ctx.path().join(".treb")).unwrap();
        })
        .test(&["registry", "sync"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── Governor proposal sync tests ────────────────────────────────────────

const GOVERNOR_ADDRESS: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DOTENV_SYNC_FOUNDRY_TOML: &str = r#"
[profile.default]
src = "src"
out = "out"
libs = ["lib"]

[rpc_endpoints]
mainnet = "${TREB_P10_SYNC_RPC_URL}"
"#;

fn governor_address() -> Address {
    GOVERNOR_ADDRESS.parse().unwrap()
}

fn governor_state_runtime(state: u8) -> Bytes {
    Bytes::from(vec![0x60, state, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3])
}

fn governor_revert_runtime() -> Bytes {
    Bytes::from(vec![0x60, 0x00, 0x60, 0x00, 0xfd])
}

async fn governor_sync_context(runtime_code: Bytes) -> Option<TestContext> {
    let ctx = match TestContext::new("minimal-project")
        .with_anvil_mapped("mainnet", AnvilConfig::new().chain_id(1).port(0), 8545)
        .await
    {
        Ok(ctx) => ctx,
        Err(err) if err.to_string().contains("Operation not permitted") => return None,
        Err(err) => panic!("failed to spawn anvil: {err}"),
    };

    ctx.run(["init"]).success();
    ctx.anvil("mainnet")
        .unwrap()
        .instance()
        .set_code(governor_address(), runtime_code)
        .await
        .expect("failed to set governor runtime code");
    seed_governor_proposal(&ctx);

    Some(ctx)
}

async fn governor_sync_dotenv_context(runtime_code: Bytes) -> Option<TestContext> {
    let ctx = TestContext::new("minimal-project");
    let rpc_url = spawn_governor_rpc_server(1, runtime_code[1])?;
    std::fs::write(ctx.path().join("foundry.toml"), DOTENV_SYNC_FOUNDRY_TOML)
        .expect("write env-backed foundry.toml");
    std::fs::write(ctx.path().join(".env"), format!("TREB_P10_SYNC_RPC_URL={rpc_url}\n"))
        .expect("write .env with sync rpc url");

    ctx.run(["init"]).success();
    seed_governor_proposal(&ctx);

    Some(ctx)
}

fn read_http_request(stream: &mut std::net::TcpStream) -> std::io::Result<String> {
    stream.set_read_timeout(Some(std::time::Duration::from_millis(250)))?;

    let mut buf = Vec::new();
    let mut chunk = [0_u8; 2048];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);

                let Some(headers_end) = buf.windows(4).position(|window| window == b"\r\n\r\n")
                else {
                    continue;
                };
                let body_start = headers_end + 4;
                let headers = String::from_utf8_lossy(&buf[..headers_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.split_once(':').and_then(|(name, value)| {
                            if name.eq_ignore_ascii_case("content-length") {
                                value.trim().parse::<usize>().ok()
                            } else {
                                None
                            }
                        })
                    })
                    .unwrap_or(0);

                if buf.len() >= body_start + content_length {
                    break;
                }
            }
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                break;
            }
            Err(err) => return Err(err),
        }
    }

    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn spawn_governor_rpc_server(chain_id: u64, proposal_state: u8) -> Option<String> {
    let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return None,
        Err(err) => panic!("failed to bind local sync RPC: {err}"),
    };
    listener.set_nonblocking(true).expect("failed to set nonblocking");
    let port = listener.local_addr().expect("failed to read local addr").port();

    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request =
                        read_http_request(&mut stream).expect("failed to read RPC request");
                    if request.is_empty() {
                        continue;
                    }

                    let body = request.split("\r\n\r\n").nth(1).unwrap_or("");
                    let json: serde_json::Value =
                        serde_json::from_str(body).expect("invalid RPC request body");
                    let method = json["method"].as_str().expect("RPC method should be present");
                    let result = match method {
                        "eth_chainId" => serde_json::json!(format!("0x{chain_id:x}")),
                        "eth_call" => serde_json::json!(format!("0x{:064x}", proposal_state)),
                        other => panic!("unexpected sync RPC method: {other}"),
                    };
                    let response_body = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": json.get("id").cloned().unwrap_or_else(|| serde_json::json!(1)),
                        "result": result,
                    })
                    .to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response_body.len(),
                        response_body
                    );
                    stream.write_all(response.as_bytes()).expect("failed to write RPC response");
                    stream.flush().expect("failed to flush RPC response");
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(err) => panic!("sync RPC accept failed: {err}"),
            }
        }
    });

    Some(format!("http://127.0.0.1:{port}"))
}

/// Helper: seed the registry with a governor proposal and linked transaction on chain 1.
fn seed_governor_proposal(ctx: &TestContext) {
    use chrono::{TimeZone, Utc};
    use treb_core::types::{
        GovernorProposal, Operation, ProposalStatus, Transaction, enums::TransactionStatus,
    };
    use treb_registry::Registry;

    let mut registry = Registry::open(ctx.path()).unwrap();
    let transaction = Transaction {
        id: "tx-0x0001".to_string(),
        chain_id: 1,
        hash: String::new(),
        status: TransactionStatus::Queued,
        block_number: 0,
        sender: GOVERNOR_ADDRESS.to_string(),
        nonce: 0,
        deployments: Vec::new(),
        operations: vec![Operation {
            operation_type: "CALL".to_string(),
            target: "0x1000000000000000000000000000000000000000".to_string(),
            method: "setValue".to_string(),
            result: std::collections::HashMap::new(),
        }],
        safe_context: None,
        broadcast_file: None,
        environment: "default".to_string(),
        created_at: Utc.with_ymd_and_hms(2026, 3, 8, 9, 55, 0).unwrap(),
    };
    registry.insert_transaction(transaction).unwrap();

    let proposal = GovernorProposal {
        proposal_id: "12345678901234567890".to_string(),
        governor_address: GOVERNOR_ADDRESS.to_string(),
        timelock_address: "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        chain_id: 1,
        status: ProposalStatus::Pending,
        transaction_ids: vec!["tx-0x0001".to_string()],
        proposed_by: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".to_string(),
        proposed_at: Utc.with_ymd_and_hms(2026, 3, 8, 10, 0, 0).unwrap(),
        description: String::new(),
        executed_at: None,
        execution_tx_hash: String::new(),
    };
    registry.insert_governor_proposal(proposal).unwrap();
}

/// Sync with governor proposals in registry — human output.
///
/// Verifies an on-chain state change is persisted and summarized.
#[tokio::test(flavor = "multi_thread")]
async fn sync_governor_human() {
    let Some(ctx) = governor_sync_context(governor_state_runtime(1)).await else {
        return;
    };
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("sync_governor_human")
        .test(&["registry", "sync"])
        .output_artifact(".treb/governor-txs.json")
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Sync resolves Go-style `${VAR}` RPC endpoints from `.env` when probing governors.
#[tokio::test(flavor = "multi_thread")]
async fn sync_governor_dotenv_rpc() {
    let Some(ctx) = governor_sync_dotenv_context(governor_state_runtime(1)).await else {
        return;
    };
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("sync_governor_dotenv_rpc")
        .test(&["registry", "sync"])
        .output_artifact(".treb/governor-txs.json")
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Sync with governor proposals but no matching RPC endpoint emits one warning section.
#[test]
fn sync_governor_missing_rpc_human() {
    let ctx = TestContext::new("minimal-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    ctx.run(["init"]).success();
    seed_governor_proposal(&ctx);

    let test = IntegrationTest::new("sync_governor_missing_rpc_human")
        .test(&["registry", "sync"])
        .output_artifact(".treb/governor-txs.json")
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Sync with governor proposals in registry — JSON output.
///
/// Verifies executed-state propagation updates both proposal and linked transaction records.
#[tokio::test(flavor = "multi_thread")]
async fn sync_governor_json() {
    let Some(ctx) = governor_sync_context(governor_state_runtime(7)).await else {
        return;
    };
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("sync_governor_json")
        .test(&["registry", "sync", "--json"])
        .output_artifact(".treb/governor-txs.json")
        .output_artifact(".treb/transactions.json")
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Sync with --clean removes proposals whose governor call reverts.
#[tokio::test(flavor = "multi_thread")]
async fn sync_governor_clean() {
    let Some(ctx) = governor_sync_context(governor_revert_runtime()).await else {
        return;
    };
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("sync_governor_clean")
        .test(&["registry", "sync", "--clean"])
        .output_artifact(".treb/governor-txs.json")
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

// ── Error tests ─────────────────────────────────────────────────────────

/// No foundry project produces an error.
#[test]
fn sync_no_foundry_project() {
    let ctx = TestContext::new("compose-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("sync_no_foundry_project")
        .test(&["registry", "sync"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}
