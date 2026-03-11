//! Golden-file integration tests for `treb register`.
//!
//! Happy-path tests deploy a contract directly on Anvil via `eth_sendTransaction`,
//! then run `treb register --tx-hash` with the real on-chain tx hash.
//! Error-path tests use fixed args via `IntegrationTest`.

mod framework;
mod helpers;

use std::{
    io::{Read, Write},
    path::PathBuf,
};

use framework::{
    context::TestContext,
    golden::GoldenFile,
    integration_test::{IntegrationTest, run_integration_test},
    normalizer::{
        BlockNumberNormalizer, CompilerOutputNormalizer, DurationNormalizer, GasNormalizer,
        Normalizer, NormalizerChain, PathNormalizer,
    },
};

/// Returns the golden file directory for this crate's tests.
fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("golden")
}

const TREB_TOML_WITH_STAGING_NAMESPACE: &str = r#"[accounts.deployer]
type = "private_key"
private_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

[namespace.default]
profile = "default"

[namespace.default.senders]
deployer = "deployer"

[namespace.staging]
profile = "default"

[namespace.staging.senders]
deployer = "deployer"
"#;

const DOTENV_REGISTER_FOUNDRY_TOML: &str = r#"[profile.default]
src = "src"
out = "out"
libs = ["lib"]
script = "script"

[rpc_endpoints]
dotenv-anvil = "${TREB_P10_REGISTER_RPC_URL}"
"#;
const DOTENV_REGISTER_TX_HASH: &str =
    "0x1111111111111111111111111111111111111111111111111111111111111111";
const DOTENV_REGISTER_DEPLOYER: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const DOTENV_REGISTER_CONTRACT: &str = "0x1234567890abcdef1234567890abcdef12345678";

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

fn spawn_register_rpc_server() -> Option<String> {
    let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return None,
        Err(err) => panic!("failed to bind local register RPC: {err}"),
    };
    listener.set_nonblocking(true).expect("failed to set nonblocking");
    let port = listener.local_addr().expect("failed to read local addr").port();

    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let trace = serde_json::json!({
            "type": "CREATE",
            "from": DOTENV_REGISTER_DEPLOYER,
            "to": DOTENV_REGISTER_CONTRACT,
        });

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
                        "eth_chainId" => serde_json::json!("0x7a69"),
                        "eth_getTransactionByHash" => serde_json::json!({
                            "from": DOTENV_REGISTER_DEPLOYER,
                            "nonce": "0x0",
                        }),
                        "eth_getTransactionReceipt" => serde_json::json!({
                            "blockNumber": "0x1",
                            "status": "0x1",
                            "contractAddress": DOTENV_REGISTER_CONTRACT,
                        }),
                        "debug_traceTransaction" => trace.clone(),
                        other => panic!("unexpected register RPC method: {other}"),
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
                Err(err) => panic!("register RPC accept failed: {err}"),
            }
        }
    });

    Some(format!("http://127.0.0.1:{port}"))
}

/// Deploy a minimal contract directly on Anvil via `eth_sendTransaction` and
/// return the on-chain transaction hash.
///
/// Uses Anvil's default funded account (0xf39F...92266) to send a contract
/// creation transaction with minimal bytecode.  Anvil auto-mines, so the
/// transaction is included in a block immediately.
async fn deploy_contract_on_anvil(rpc_url: &str) -> String {
    let client = reqwest::Client::new();

    // Minimal creation bytecode: deploys a contract that returns 0x00 (STOP).
    // PUSH1 0x01  PUSH1 0x00  RETURN  =  0x600160_00_f3  (wrong)
    // Actually: code that copies 1 byte of runtime code (0x00 = STOP):
    //   PUSH1 0x01   // runtime code size
    //   PUSH1 0x0a   // offset of runtime code in initcode
    //   PUSH1 0x00   // dest in memory
    //   CODECOPY     // copy runtime to memory
    //   PUSH1 0x01   // runtime size
    //   PUSH1 0x00   // memory offset
    //   RETURN       // return runtime code
    //   STOP         // runtime code (1 byte: 0x00)
    // = 6001600a600039600160_00_f300
    let creation_bytecode = "0x6001600a600039600160006000f300";

    let resp: serde_json::Value = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_sendTransaction",
            "params": [{
                "from": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
                "data": creation_bytecode,
                "gas": "0x100000"
            }],
            "id": 1
        }))
        .send()
        .await
        .expect("failed to send deploy tx to Anvil")
        .json()
        .await
        .expect("invalid JSON response from Anvil");

    resp["result"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("eth_sendTransaction failed: {}", serde_json::to_string_pretty(&resp).unwrap())
        })
        .to_string()
}

/// Collect output from a register command, normalize, and compare against golden files.
///
/// Mimics `run_integration_test` output format for consistency:
/// command output → `commands.golden`, each artifact → `{stem}.golden`.
fn run_register_golden_test(
    ctx: &TestContext,
    test_name: &str,
    args: &[&str],
    output_artifacts: &[&str],
    extra_normalizers: Vec<Box<dyn Normalizer>>,
) {
    let assertion = ctx.run(args);
    let stdout = String::from_utf8_lossy(&assertion.get_output().stdout).to_string();
    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assertion.success();

    let mut output = format!("=== cmd 0: [{}] ===\n", args.join(" "));
    if !stdout.is_empty() {
        output.push_str(&stdout);
    }
    if !stderr.is_empty() {
        output.push_str(&stderr);
    }
    output.push('\n');

    // Build normalizer closure
    let default_chain = NormalizerChain::default_chain();
    let normalize = |text: &str| -> String {
        let mut normalized = default_chain.normalize(text);
        for n in &extra_normalizers {
            normalized = n.normalize(&normalized);
        }
        normalized
    };

    let golden = GoldenFile::new(golden_dir());

    // Command output → commands.golden
    golden.compare_with_normalizer(test_name, "commands", &output, normalize);

    // Each artifact → {stem}.golden (missing files silently skipped)
    for artifact_path in output_artifacts {
        let full_path = ctx.path().join(artifact_path);
        if full_path.exists() {
            let content = std::fs::read_to_string(&full_path)
                .unwrap_or_else(|e| panic!("failed to read artifact {artifact_path}: {e}"));
            let stem = std::path::Path::new(artifact_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(artifact_path);
            golden.compare_with_normalizer(test_name, stem, &content, normalize);
        }
    }
}

async fn register_test_context() -> Option<TestContext> {
    match TestContext::new("project").with_anvil("anvil-31337").await {
        Ok(ctx) => Some(ctx),
        Err(err) if err.to_string().contains("Operation not permitted") => None,
        Err(err) => panic!("failed to spawn anvil: {err}"),
    }
}

// ── Happy-path tests ────────────────────────────────────────────────────

/// Register a contract from a prior deployment transaction.
///
/// Deploys a minimal contract directly on Anvil, then registers it via
/// `treb register --tx-hash`.  Verifies table output and `deployments.json`
/// artifact.
#[tokio::test(flavor = "multi_thread")]
async fn register_basic() {
    let Some(ctx) = register_test_context().await else {
        return;
    };

    // Setup: init project
    ctx.run(["init"]).success();

    // Deploy directly on Anvil and get the real tx hash
    let rpc_url = ctx.anvil("anvil-31337").unwrap().rpc_url().to_string();
    let tx_hash = deploy_contract_on_anvil(&rpc_url).await;

    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    run_register_golden_test(
        &ctx,
        "register_basic",
        &["register", "--tx-hash", &tx_hash, "--network", "anvil-31337"],
        &[".treb/deployments.json"],
        vec![
            Box::new(path_normalizer),
            Box::new(CompilerOutputNormalizer),
            Box::new(GasNormalizer),
            Box::new(BlockNumberNormalizer),
            Box::new(DurationNormalizer),
        ],
    );
}

/// Register without `--network` falls back to the active config network.
#[tokio::test(flavor = "multi_thread")]
async fn register_network_from_config() {
    let Some(ctx) = register_test_context().await else {
        return;
    };

    ctx.run(["init"]).success();
    ctx.run(["config", "set", "network", "anvil-31337"]).success();

    let rpc_url = ctx.anvil("anvil-31337").unwrap().rpc_url().to_string();
    let tx_hash = deploy_contract_on_anvil(&rpc_url).await;

    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    run_register_golden_test(
        &ctx,
        "register_network_from_config",
        &["register", "--tx-hash", &tx_hash],
        &[".treb/deployments.json"],
        vec![
            Box::new(path_normalizer),
            Box::new(CompilerOutputNormalizer),
            Box::new(GasNormalizer),
            Box::new(BlockNumberNormalizer),
            Box::new(DurationNormalizer),
        ],
    );
}

/// Register without `--namespace` falls back to the active config namespace.
#[tokio::test(flavor = "multi_thread")]
async fn register_namespace_from_config() {
    let Some(ctx) = register_test_context().await else {
        return;
    };

    std::fs::write(ctx.path().join("treb.toml"), TREB_TOML_WITH_STAGING_NAMESPACE)
        .expect("write treb.toml with staging namespace");

    ctx.run(["init"]).success();
    ctx.run(["config", "set", "namespace", "staging"]).success();

    let rpc_url = ctx.anvil("anvil-31337").unwrap().rpc_url().to_string();
    let tx_hash = deploy_contract_on_anvil(&rpc_url).await;

    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    run_register_golden_test(
        &ctx,
        "register_namespace_from_config",
        &["register", "--tx-hash", &tx_hash, "--network", "anvil-31337"],
        &[".treb/deployments.json"],
        vec![
            Box::new(path_normalizer),
            Box::new(CompilerOutputNormalizer),
            Box::new(GasNormalizer),
            Box::new(BlockNumberNormalizer),
            Box::new(DurationNormalizer),
        ],
    );
}

/// Register resolves a config-backed network through a `.env` RPC endpoint.
#[test]
fn register_network_from_dotenv_config() {
    let ctx = TestContext::new("project");
    ctx.run(["init"]).success();

    let Some(rpc_url) = spawn_register_rpc_server() else {
        return;
    };
    std::fs::write(ctx.path().join("foundry.toml"), DOTENV_REGISTER_FOUNDRY_TOML)
        .expect("write env-backed foundry.toml");
    std::fs::write(ctx.path().join(".env"), format!("TREB_P10_REGISTER_RPC_URL={rpc_url}\n"))
        .expect("write .env with register rpc url");
    ctx.run(["config", "set", "network", "dotenv-anvil"]).success();

    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    run_register_golden_test(
        &ctx,
        "register_network_from_dotenv_config",
        &["register", "--tx-hash", DOTENV_REGISTER_TX_HASH],
        &[".treb/deployments.json"],
        vec![
            Box::new(path_normalizer),
            Box::new(CompilerOutputNormalizer),
            Box::new(GasNormalizer),
            Box::new(BlockNumberNormalizer),
            Box::new(DurationNormalizer),
        ],
    );
}

/// Register with JSON output.
///
/// Same setup as `register_basic`, verifies JSON structure with `deployments`
/// array containing entries with `address`, `contractName`, `deploymentId`, `label`.
#[tokio::test(flavor = "multi_thread")]
async fn register_json() {
    let Some(ctx) = register_test_context().await else {
        return;
    };

    // Setup: init project
    ctx.run(["init"]).success();

    // Deploy directly on Anvil and get the real tx hash
    let rpc_url = ctx.anvil("anvil-31337").unwrap().rpc_url().to_string();
    let tx_hash = deploy_contract_on_anvil(&rpc_url).await;

    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    run_register_golden_test(
        &ctx,
        "register_json",
        &["register", "--tx-hash", &tx_hash, "--network", "anvil-31337", "--json"],
        &[],
        vec![
            Box::new(path_normalizer),
            Box::new(CompilerOutputNormalizer),
            Box::new(GasNormalizer),
            Box::new(BlockNumberNormalizer),
            Box::new(DurationNormalizer),
        ],
    );
}

// ── Error-path tests ────────────────────────────────────────────────────

/// Error: tx hash without 0x prefix.
///
/// Verifies the error mentions the `0x` prefix requirement.
#[test]
fn register_error_bad_prefix() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("register_error_bad_prefix")
        .setup(&["init"])
        .test(&["register", "--tx-hash", "abc123", "--rpc-url", "http://localhost:8545"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Error: uninitialized project (no foundry.toml or .treb/).
///
/// Verifies the error mentions `foundry.toml` or `treb init`.
#[test]
fn register_error_no_init() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("register_error_no_init")
        .pre_setup_hook(|ctx| {
            std::fs::remove_dir_all(ctx.treb_dir()).ok();
            std::fs::remove_file(ctx.path().join("foundry.toml")).ok();
        })
        .test(&["register", "--tx-hash", "0xabc"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Error: config-backed network fallback requires an active network.
#[test]
fn register_error_no_active_network() {
    let ctx = TestContext::new("project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("register_error_no_active_network")
        .setup(&["init"])
        .test(&["register", "--tx-hash", "0xabc"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}

/// Error: nonexistent transaction hash.
///
/// Uses a fabricated hash that doesn't exist on the Anvil node.
/// Verifies the error mentions "not found".
#[tokio::test(flavor = "multi_thread")]
async fn register_error_tx_not_found() {
    let Some(ctx) = register_test_context().await else {
        return;
    };

    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("register_error_tx_not_found")
        .setup(&["init"])
        .test(&[
            "register",
            "--tx-hash",
            "0x0000000000000000000000000000000000000000000000000000000000000001",
            "--network",
            "anvil-31337",
        ])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer));

    run_integration_test(&test, &ctx);
}
