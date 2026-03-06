//! `treb register` command implementation.
//!
//! Fetches a historical transaction, traces it for contract creations,
//! and records deployments in the registry. Falls back to receipt-only
//! mode when `debug_traceTransaction` is unsupported by the RPC.

use std::{collections::HashMap, env, time::Duration};

use anyhow::{Context, bail};
use chrono::Utc;
use serde::Serialize;
use treb_core::types::{
    ArtifactInfo, Deployment, DeploymentMethod, DeploymentStrategy, DeploymentType, Operation,
    Transaction, TransactionStatus, VerificationInfo, VerificationStatus, generate_deployment_id,
};
use treb_registry::Registry;

use crate::output;

const FOUNDRY_TOML: &str = "foundry.toml";
const TREB_DIR: &str = ".treb";

// ── JSON-RPC helpers ────────────────────────────────────────────────────

/// Build a reqwest client with timeouts for RPC calls.
fn rpc_client() -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("failed to build HTTP client")
}

/// Make a JSON-RPC call and return the "result" field.
async fn rpc_call(
    client: &reqwest::Client,
    url: &str,
    method: &str,
    params: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1
    });

    let resp = client
        .post(url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("RPC request failed for {method}"))?;

    let json: serde_json::Value =
        resp.json().await.with_context(|| format!("invalid JSON response for {method}"))?;

    if let Some(error) = json.get("error") {
        bail!("RPC error for {method}: {error}");
    }

    json.get("result")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("no result field in {method} response"))
}

/// Parse a hex string (with or without 0x prefix) to u64.
fn parse_hex_u64(hex: &str) -> u64 {
    let stripped = hex.strip_prefix("0x").or_else(|| hex.strip_prefix("0X")).unwrap_or(hex);
    u64::from_str_radix(stripped, 16).unwrap_or(0)
}

// ── RPC URL resolution ──────────────────────────────────────────────────

/// Resolve the RPC URL from --rpc-url or --network (via foundry.toml endpoints).
fn resolve_rpc_url(
    rpc_url: Option<String>,
    network: Option<String>,
    cwd: &std::path::Path,
) -> anyhow::Result<String> {
    if let Some(url) = rpc_url {
        return Ok(url);
    }

    let network_name = network.context("either --rpc-url or --network must be specified")?;

    // If it already looks like a URL, use it directly.
    if network_name.starts_with("http://") || network_name.starts_with("https://") {
        return Ok(network_name);
    }

    // Look up in foundry.toml [rpc_endpoints].
    let config = treb_config::load_foundry_config(cwd).map_err(|e| anyhow::anyhow!("{e}"))?;
    let endpoints = treb_config::rpc_endpoints(&config);

    let url = endpoints.get(&network_name).ok_or_else(|| {
        anyhow::anyhow!("network '{}' not found in foundry.toml [rpc_endpoints]", network_name)
    })?;

    // Bail if the URL has unresolved env vars.
    if url.contains("${") {
        bail!(
            "RPC URL for network '{}' contains unresolved environment variables: {}\n\n\
             Set the required environment variables or use --rpc-url directly.",
            network_name,
            url
        );
    }

    Ok(url.clone())
}

// ── Trace parsing ───────────────────────────────────────────────────────

/// A contract creation found in a transaction trace or receipt.
struct TracedCreation {
    address: String,
    from: String,
    create_type: String,
}

/// Parse debug_traceTransaction callTracer output for contract creations.
fn extract_creations_from_trace(trace: &serde_json::Value) -> Vec<TracedCreation> {
    let mut creations = Vec::new();
    walk_trace_calls(trace, &mut creations);
    creations
}

fn walk_trace_calls(call: &serde_json::Value, creations: &mut Vec<TracedCreation>) {
    let call_type = call.get("type").and_then(|v| v.as_str()).unwrap_or("");

    if call_type.eq_ignore_ascii_case("CREATE") || call_type.eq_ignore_ascii_case("CREATE2") {
        if let Some(to) = call.get("to").and_then(|v| v.as_str()) {
            let from = call.get("from").and_then(|v| v.as_str()).unwrap_or("").to_string();
            creations.push(TracedCreation {
                address: to.to_string(),
                from,
                create_type: call_type.to_uppercase(),
            });
        }
    }

    // Recurse into sub-calls.
    if let Some(calls) = call.get("calls").and_then(|v| v.as_array()) {
        for subcall in calls {
            walk_trace_calls(subcall, creations);
        }
    }
}

// ── Contract name resolution ────────────────────────────────────────────

/// Resolve the effective contract name from --contract-name or --contract.
///
/// `--contract-name` takes precedence. If only `--contract` is provided,
/// the name is extracted from the artifact path (e.g., "src/Counter.sol:Counter"
/// → "Counter").
fn resolve_contract_name(
    contract_name: Option<String>,
    contract: Option<String>,
) -> Option<String> {
    if contract_name.is_some() {
        return contract_name;
    }

    if let Some(ref artifact) = contract {
        if let Some(pos) = artifact.rfind(':') {
            return Some(artifact[pos + 1..].to_string());
        }
        if let Some(pos) = artifact.rfind('/') {
            let name = &artifact[pos + 1..];
            return Some(name.strip_suffix(".sol").unwrap_or(name).to_string());
        }
        return Some(artifact.clone());
    }

    None
}

// ── Output types ────────────────────────────────────────────────────────

#[derive(Serialize)]
struct RegisterOutputJson {
    success: bool,
    tx_hash: String,
    chain_id: u64,
    mode: String,
    deployments: Vec<RegisteredDeploymentJson>,
    transaction_id: String,
}

#[derive(Serialize)]
struct RegisteredDeploymentJson {
    id: String,
    contract_name: String,
    address: String,
    namespace: String,
    chain_id: u64,
}

// ── Main entry point ────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub async fn run(
    tx_hash: &str,
    network: Option<String>,
    rpc_url: Option<String>,
    address: Option<String>,
    contract: Option<String>,
    contract_name: Option<String>,
    label: Option<String>,
    namespace: Option<String>,
    _skip_verify: bool,
    json: bool,
) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;

    // ── Validate ────────────────────────────────────────────────────────
    if !cwd.join(FOUNDRY_TOML).exists() {
        bail!(
            "no foundry.toml found in {}\n\n\
             Run `forge init` to create a Foundry project, then `treb init`.",
            cwd.display()
        );
    }
    if !cwd.join(TREB_DIR).exists() {
        bail!(
            "project not initialized — .treb/ directory not found in {}\n\n\
             Run `treb init` first.",
            cwd.display()
        );
    }
    if !tx_hash.starts_with("0x") && !tx_hash.starts_with("0X") {
        bail!("--tx-hash must be a hex string starting with 0x");
    }

    // ── Resolve RPC URL ─────────────────────────────────────────────────
    let effective_rpc_url = resolve_rpc_url(rpc_url, network, &cwd)?;
    let client = rpc_client()?;

    // ── Fetch chain ID ──────────────────────────────────────────────────
    let chain_id_result =
        rpc_call(&client, &effective_rpc_url, "eth_chainId", serde_json::json!([])).await?;
    let chain_id = parse_hex_u64(chain_id_result.as_str().unwrap_or("0x0"));

    // ── Fetch transaction ───────────────────────────────────────────────
    let tx_result = rpc_call(
        &client,
        &effective_rpc_url,
        "eth_getTransactionByHash",
        serde_json::json!([tx_hash]),
    )
    .await?;

    if tx_result.is_null() {
        bail!("transaction not found: {tx_hash}");
    }

    let sender = tx_result.get("from").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let nonce = parse_hex_u64(tx_result.get("nonce").and_then(|v| v.as_str()).unwrap_or("0x0"));

    // ── Fetch receipt ───────────────────────────────────────────────────
    let receipt_result = rpc_call(
        &client,
        &effective_rpc_url,
        "eth_getTransactionReceipt",
        serde_json::json!([tx_hash]),
    )
    .await?;

    if receipt_result.is_null() {
        bail!("transaction receipt not found (transaction may be pending): {tx_hash}");
    }

    let block_number =
        parse_hex_u64(receipt_result.get("blockNumber").and_then(|v| v.as_str()).unwrap_or("0x0"));

    let receipt_status = receipt_result.get("status").and_then(|v| v.as_str()).unwrap_or("0x1");
    if receipt_status == "0x0" {
        bail!("transaction reverted: {tx_hash}");
    }

    // ── Try trace, fall back to receipt-only ─────────────────────────────
    if !json {
        output::print_stage("\u{1f50d}", "Tracing transaction...");
    }

    let mut creations;
    let mode;

    let trace_result = rpc_call(
        &client,
        &effective_rpc_url,
        "debug_traceTransaction",
        serde_json::json!([tx_hash, {"tracer": "callTracer"}]),
    )
    .await;

    match trace_result {
        Ok(trace) => {
            mode = "trace";
            creations = extract_creations_from_trace(&trace);

            // Also check receipt contractAddress (direct CREATE may appear
            // at the top level of the trace, but add it as a safety net).
            if let Some(addr) = receipt_contract_address(&receipt_result) {
                if !creations.iter().any(|c| c.address.eq_ignore_ascii_case(&addr)) {
                    creations.push(TracedCreation {
                        address: addr,
                        from: sender.clone(),
                        create_type: "CREATE".to_string(),
                    });
                }
            }
        }
        Err(_) => {
            mode = "receipt";
            if !json {
                eprintln!(
                    "{}",
                    output::format_warning_banner(
                        "\u{26a0}\u{fe0f}",
                        "debug_traceTransaction not available, using receipt-only mode"
                    )
                );
            }
            creations = Vec::new();

            if let Some(addr) = receipt_contract_address(&receipt_result) {
                creations.push(TracedCreation {
                    address: addr,
                    from: sender.clone(),
                    create_type: "CREATE".to_string(),
                });
            }
        }
    }

    if creations.is_empty() {
        bail!("no contract creations found in transaction {tx_hash}");
    }

    // ── Filter by --address ─────────────────────────────────────────────
    if let Some(ref filter_addr) = address {
        let filter_lower = filter_addr.to_lowercase();
        creations.retain(|c| c.address.to_lowercase() == filter_lower);
        if creations.is_empty() {
            bail!("address {} not found in transaction {tx_hash}", filter_addr);
        }
    }

    // ── Build defaults ──────────────────────────────────────────────────
    let effective_namespace = namespace.unwrap_or_else(|| "default".to_string());
    let effective_label = label.unwrap_or_default();
    let effective_name = resolve_contract_name(contract_name, contract);
    let tx_id = format!("tx-{tx_hash}");

    // ── Open registry ───────────────────────────────────────────────────
    let mut registry = Registry::open(&cwd).context("failed to open registry")?;

    // ── Register each creation ──────────────────────────────────────────
    if !json {
        output::print_stage("\u{1f4dd}", "Registering deployments...");
    }

    let mut registered: Vec<(RegisteredDeploymentJson, String)> = Vec::new();

    for (i, creation) in creations.iter().enumerate() {
        let name = match &effective_name {
            Some(n) if creations.len() == 1 => n.clone(),
            Some(n) => format!("{n}_{i}"),
            None if creations.len() == 1 => "Unknown".to_string(),
            None => format!("Unknown_{i}"),
        };

        let dep_label = if creations.len() == 1 {
            effective_label.clone()
        } else {
            format!("{}_{i}", effective_label)
        };

        let deployment_id =
            generate_deployment_id(&effective_namespace, chain_id, &name, &dep_label);

        // Duplicate detection
        if registry.get_deployment(&deployment_id).is_some() {
            bail!(
                "deployment already exists: {deployment_id}\n\n\
                 Use a different --label or --namespace to avoid conflicts."
            );
        }

        let method = match creation.create_type.as_str() {
            "CREATE2" => DeploymentMethod::Create2,
            "CREATE3" => DeploymentMethod::Create3,
            _ => DeploymentMethod::Create,
        };

        let deployment = Deployment {
            id: deployment_id.clone(),
            namespace: effective_namespace.clone(),
            chain_id,
            contract_name: name.clone(),
            label: dep_label,
            address: creation.address.clone(),
            deployment_type: DeploymentType::Unknown,
            transaction_id: tx_id.clone(),
            deployment_strategy: DeploymentStrategy {
                method,
                salt: String::new(),
                init_code_hash: String::new(),
                factory: if creation.from != sender {
                    creation.from.clone()
                } else {
                    String::new()
                },
                constructor_args: String::new(),
                entropy: String::new(),
            },
            proxy_info: None,
            artifact: ArtifactInfo {
                path: String::new(),
                compiler_version: String::new(),
                bytecode_hash: String::new(),
                script_path: String::new(),
                git_commit: String::new(),
            },
            verification: VerificationInfo {
                status: VerificationStatus::Unverified,
                etherscan_url: String::new(),
                verified_at: None,
                reason: String::new(),
                verifiers: HashMap::new(),
            },
            tags: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        registry
            .insert_deployment(deployment)
            .with_context(|| format!("failed to register deployment {deployment_id}"))?;

        registered.push((
            RegisteredDeploymentJson {
                id: deployment_id,
                contract_name: name,
                address: creation.address.clone(),
                namespace: effective_namespace.clone(),
                chain_id,
            },
            creation.create_type.clone(),
        ));
    }

    // ── Create transaction record ───────────────────────────────────────
    let deployment_ids: Vec<String> = registered.iter().map(|(d, _)| d.id.clone()).collect();

    let operations: Vec<Operation> = registered
        .iter()
        .map(|(d, create_type)| Operation {
            operation_type: "DEPLOY".to_string(),
            target: d.address.clone(),
            method: create_type.clone(),
            result: {
                let mut m = HashMap::new();
                m.insert("address".into(), serde_json::Value::String(d.address.clone()));
                m
            },
        })
        .collect();

    let transaction = Transaction {
        id: tx_id.clone(),
        chain_id,
        hash: tx_hash.to_string(),
        status: TransactionStatus::Executed,
        block_number,
        sender,
        nonce,
        deployments: deployment_ids,
        operations,
        safe_context: None,
        environment: effective_namespace.clone(),
        created_at: Utc::now(),
    };

    registry.insert_transaction(transaction).context("failed to record transaction")?;

    // ── Display ─────────────────────────────────────────────────────────
    let dep_jsons: Vec<RegisteredDeploymentJson> = registered.into_iter().map(|(d, _)| d).collect();

    if json {
        output::print_json(&RegisterOutputJson {
            success: true,
            tx_hash: tx_hash.to_string(),
            chain_id,
            mode: mode.to_string(),
            deployments: dep_jsons,
            transaction_id: tx_id,
        })?;
    } else {
        let mut table = output::build_table(&["Contract", "Address", "Namespace", "Chain"]);

        for dep in &dep_jsons {
            table.add_row(vec![
                &dep.contract_name,
                &dep.address,
                &dep.namespace,
                &dep.chain_id.to_string(),
            ]);
        }

        output::print_table(&table);
        println!();
        let n = dep_jsons.len();
        println!(
            "{n} deployment{} registered from transaction {}",
            if n == 1 { "" } else { "s" },
            output::truncate_address(tx_hash),
        );
    }

    Ok(())
}

/// Extract a non-zero contractAddress from a transaction receipt.
fn receipt_contract_address(receipt: &serde_json::Value) -> Option<String> {
    let addr = receipt.get("contractAddress").and_then(|v| v.as_str())?;
    if addr.is_empty() || addr == "0x0000000000000000000000000000000000000000" {
        return None;
    }
    Some(addr.to_string())
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    // ── Helper ──────────────────────────────────────────────────────────

    fn setup_project(dir: &std::path::Path) {
        std::fs::write(
            dir.join("foundry.toml"),
            r#"
[profile.default]
src = "src"

[rpc_endpoints]
mainnet = "https://eth-mainnet.example.com"
sepolia = "https://sepolia.example.com"
needs_env = "https://rpc.example.com/${API_KEY}"
"#,
        )
        .unwrap();
    }

    // ── resolve_rpc_url ─────────────────────────────────────────────────

    #[test]
    fn rpc_url_direct_overrides_network() {
        let tmp = TempDir::new().unwrap();
        let url = resolve_rpc_url(
            Some("https://my-rpc.example.com".to_string()),
            Some("mainnet".to_string()),
            tmp.path(),
        )
        .unwrap();
        assert_eq!(url, "https://my-rpc.example.com");
    }

    #[test]
    fn rpc_url_resolves_from_foundry_toml() {
        let tmp = TempDir::new().unwrap();
        setup_project(tmp.path());
        let url = resolve_rpc_url(None, Some("mainnet".to_string()), tmp.path()).unwrap();
        assert_eq!(url, "https://eth-mainnet.example.com");
    }

    #[test]
    fn rpc_url_network_as_url_passthrough() {
        let tmp = TempDir::new().unwrap();
        let url =
            resolve_rpc_url(None, Some("https://direct-url.example.com".to_string()), tmp.path())
                .unwrap();
        assert_eq!(url, "https://direct-url.example.com");
    }

    #[test]
    fn rpc_url_missing_both_errors() {
        let tmp = TempDir::new().unwrap();
        let err = resolve_rpc_url(None, None, tmp.path()).unwrap_err();
        assert!(err.to_string().contains("--rpc-url or --network"));
    }

    #[test]
    fn rpc_url_unknown_network_errors() {
        let tmp = TempDir::new().unwrap();
        setup_project(tmp.path());
        let err = resolve_rpc_url(None, Some("goerli".to_string()), tmp.path()).unwrap_err();
        assert!(err.to_string().contains("goerli"));
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn rpc_url_unresolved_env_var_errors() {
        let tmp = TempDir::new().unwrap();
        setup_project(tmp.path());
        let err = resolve_rpc_url(None, Some("needs_env".to_string()), tmp.path()).unwrap_err();
        assert!(err.to_string().contains("unresolved environment variables"));
    }

    // ── extract_creations_from_trace ────────────────────────────────────

    #[test]
    fn trace_single_create() {
        let trace = serde_json::json!({
            "type": "CREATE",
            "from": "0xSender",
            "to": "0xCreated",
            "value": "0x0",
            "input": "0x6080...",
            "output": "0x6080..."
        });
        let creations = extract_creations_from_trace(&trace);
        assert_eq!(creations.len(), 1);
        assert_eq!(creations[0].address, "0xCreated");
        assert_eq!(creations[0].from, "0xSender");
        assert_eq!(creations[0].create_type, "CREATE");
    }

    #[test]
    fn trace_nested_creates() {
        let trace = serde_json::json!({
            "type": "CALL",
            "from": "0xSender",
            "to": "0xFactory",
            "calls": [
                {
                    "type": "CREATE",
                    "from": "0xFactory",
                    "to": "0xChild1"
                },
                {
                    "type": "CREATE2",
                    "from": "0xFactory",
                    "to": "0xChild2"
                }
            ]
        });
        let creations = extract_creations_from_trace(&trace);
        assert_eq!(creations.len(), 2);
        assert_eq!(creations[0].address, "0xChild1");
        assert_eq!(creations[0].create_type, "CREATE");
        assert_eq!(creations[1].address, "0xChild2");
        assert_eq!(creations[1].create_type, "CREATE2");
    }

    #[test]
    fn trace_deeply_nested_create() {
        let trace = serde_json::json!({
            "type": "CALL",
            "from": "0xSender",
            "to": "0xRouter",
            "calls": [{
                "type": "DELEGATECALL",
                "from": "0xRouter",
                "to": "0xImpl",
                "calls": [{
                    "type": "CREATE",
                    "from": "0xImpl",
                    "to": "0xDeepChild"
                }]
            }]
        });
        let creations = extract_creations_from_trace(&trace);
        assert_eq!(creations.len(), 1);
        assert_eq!(creations[0].address, "0xDeepChild");
        assert_eq!(creations[0].from, "0xImpl");
    }

    #[test]
    fn trace_no_creates() {
        let trace = serde_json::json!({
            "type": "CALL",
            "from": "0xSender",
            "to": "0xContract",
            "calls": [{
                "type": "CALL",
                "from": "0xContract",
                "to": "0xOther"
            }]
        });
        let creations = extract_creations_from_trace(&trace);
        assert!(creations.is_empty());
    }

    #[test]
    fn trace_create_without_to_is_skipped() {
        let trace = serde_json::json!({
            "type": "CREATE",
            "from": "0xSender"
            // no "to" field — creation failed before address assigned
        });
        let creations = extract_creations_from_trace(&trace);
        assert!(creations.is_empty());
    }

    // ── resolve_contract_name ───────────────────────────────────────────

    #[test]
    fn contract_name_takes_precedence() {
        let result = resolve_contract_name(
            Some("Counter".to_string()),
            Some("src/Counter.sol:Counter".to_string()),
        );
        assert_eq!(result, Some("Counter".to_string()));
    }

    #[test]
    fn contract_name_extracted_from_artifact_colon() {
        let result = resolve_contract_name(None, Some("src/Counter.sol:Counter".to_string()));
        assert_eq!(result, Some("Counter".to_string()));
    }

    #[test]
    fn contract_name_extracted_from_artifact_path() {
        let result = resolve_contract_name(None, Some("src/Counter.sol".to_string()));
        assert_eq!(result, Some("Counter".to_string()));
    }

    #[test]
    fn contract_name_bare_name() {
        let result = resolve_contract_name(None, Some("Counter".to_string()));
        assert_eq!(result, Some("Counter".to_string()));
    }

    #[test]
    fn contract_name_none_when_both_empty() {
        let result = resolve_contract_name(None, None);
        assert_eq!(result, None);
    }

    // ── parse_hex_u64 ───────────────────────────────────────────────────

    #[test]
    fn parse_hex_with_prefix() {
        assert_eq!(parse_hex_u64("0x1"), 1);
        assert_eq!(parse_hex_u64("0xff"), 255);
        assert_eq!(parse_hex_u64("0x12d687"), 1234567);
    }

    #[test]
    fn parse_hex_without_prefix() {
        assert_eq!(parse_hex_u64("ff"), 255);
    }

    #[test]
    fn parse_hex_invalid_returns_zero() {
        assert_eq!(parse_hex_u64("not_hex"), 0);
    }

    // ── receipt_contract_address ─────────────────────────────────────────

    #[test]
    fn receipt_has_contract_address() {
        let receipt = serde_json::json!({
            "contractAddress": "0xNewContract"
        });
        assert_eq!(receipt_contract_address(&receipt), Some("0xNewContract".to_string()));
    }

    #[test]
    fn receipt_null_contract_address() {
        let receipt = serde_json::json!({
            "contractAddress": null
        });
        assert_eq!(receipt_contract_address(&receipt), None);
    }

    #[test]
    fn receipt_zero_contract_address() {
        let receipt = serde_json::json!({
            "contractAddress": "0x0000000000000000000000000000000000000000"
        });
        assert_eq!(receipt_contract_address(&receipt), None);
    }

    #[test]
    fn receipt_empty_contract_address() {
        let receipt = serde_json::json!({
            "contractAddress": ""
        });
        assert_eq!(receipt_contract_address(&receipt), None);
    }

    #[test]
    fn receipt_missing_contract_address() {
        let receipt = serde_json::json!({
            "status": "0x1"
        });
        assert_eq!(receipt_contract_address(&receipt), None);
    }
}
