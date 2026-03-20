//! Shared receipt processing for tx-hash-based operations.
//!
//! Extracts contract creations, proxy patterns, and Upgraded events from
//! a transaction receipt and its debug trace. Used by `register`, `fork exec`,
//! and `sync --tx-hash`.

use std::time::Duration;

use alloy_primitives::{Address, B256, Bytes, Log as PrimitiveLog};
use anyhow::{Context, bail};
use chrono::Utc;
use serde::Serialize;
use treb_core::types::{ProxyUpgrade, deployment::ProxyInfo};
use treb_forge::events::{ProxyEvent, decode_events};
use treb_registry::Registry;

// ── JSON-RPC helpers ────────────────────────────────────────────────────

/// Build a reqwest client with timeouts for RPC calls.
pub fn rpc_client() -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("failed to build HTTP client")
}

/// Make a JSON-RPC call and return the "result" field.
pub async fn rpc_call(
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
pub fn parse_hex_u64(hex: &str) -> u64 {
    let stripped = hex.strip_prefix("0x").or_else(|| hex.strip_prefix("0X")).unwrap_or(hex);
    u64::from_str_radix(stripped, 16).unwrap_or(0)
}

// ── Trace parsing ───────────────────────────────────────────────────────

/// A contract creation found in a transaction trace or receipt.
#[derive(Debug, Clone)]
pub struct TracedCreation {
    pub address: String,
    pub from: String,
    pub create_type: String,
}

/// Parse debug_traceTransaction callTracer output for contract creations.
pub fn extract_creations_from_trace(trace: &serde_json::Value) -> Vec<TracedCreation> {
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

// ── Proxy detection (trace-based) ───────────────────────────────────────

/// A detected proxy+implementation pair from trace analysis.
#[derive(Debug, Clone)]
pub struct DetectedProxy {
    pub proxy_address: String,
    pub implementation_address: String,
}

/// Detect proxy patterns from a callTracer trace.
///
/// A proxy pattern is detected when a CREATE/CREATE2 call contains a
/// DELEGATECALL subcall from the newly created address to another address
/// (the implementation).
pub fn detect_proxy_patterns(trace: &serde_json::Value) -> Vec<DetectedProxy> {
    let mut results = Vec::new();
    find_proxy_creates(trace, &mut results);
    results
}

fn find_proxy_creates(call: &serde_json::Value, results: &mut Vec<DetectedProxy>) {
    let call_type = call.get("type").and_then(|v| v.as_str()).unwrap_or("");

    if call_type.eq_ignore_ascii_case("CREATE") || call_type.eq_ignore_ascii_case("CREATE2") {
        if let Some(created_addr) = call.get("to").and_then(|v| v.as_str()) {
            if let Some(impl_addr) = find_delegatecall_target(call, created_addr) {
                results.push(DetectedProxy {
                    proxy_address: created_addr.to_string(),
                    implementation_address: impl_addr,
                });
            }
        }
    }

    if let Some(calls) = call.get("calls").and_then(|v| v.as_array()) {
        for subcall in calls {
            find_proxy_creates(subcall, results);
        }
    }
}

fn find_delegatecall_target(call: &serde_json::Value, from_addr: &str) -> Option<String> {
    if let Some(calls) = call.get("calls").and_then(|v| v.as_array()) {
        for subcall in calls {
            let sub_type = subcall.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if sub_type.eq_ignore_ascii_case("DELEGATECALL") {
                let sub_from = subcall.get("from").and_then(|v| v.as_str()).unwrap_or("");
                if sub_from.eq_ignore_ascii_case(from_addr) {
                    if let Some(target) = subcall.get("to").and_then(|v| v.as_str()) {
                        return Some(target.to_string());
                    }
                }
            }
            if let Some(found) = find_delegatecall_target(subcall, from_addr) {
                return Some(found);
            }
        }
    }
    None
}

/// Extract a non-zero contractAddress from a transaction receipt.
pub fn receipt_contract_address(receipt: &serde_json::Value) -> Option<String> {
    let addr = receipt.get("contractAddress").and_then(|v| v.as_str())?;
    if addr.is_empty() || addr == "0x0000000000000000000000000000000000000000" {
        return None;
    }
    Some(addr.to_string())
}

// ── Proxy upgrade detection (log-based) ─────────────────────────────────

/// A proxy implementation upgrade detected from an Upgraded event in receipt logs.
#[derive(Debug, Clone)]
pub struct DetectedUpgrade {
    pub proxy_address: Address,
    pub new_implementation: Address,
}

/// Parse receipt JSON logs into alloy primitive Log entries for event decoding.
fn parse_receipt_logs(receipt: &serde_json::Value) -> Vec<PrimitiveLog> {
    let logs = match receipt.get("logs").and_then(|v| v.as_array()) {
        Some(logs) => logs,
        None => return Vec::new(),
    };

    logs.iter()
        .filter_map(|log| {
            let address: Address = log.get("address")?.as_str()?.parse().ok()?;
            let topics: Vec<B256> = log
                .get("topics")?
                .as_array()?
                .iter()
                .filter_map(|t| t.as_str()?.parse().ok())
                .collect();
            let data_hex = log.get("data")?.as_str()?;
            let stripped = data_hex.strip_prefix("0x").unwrap_or(data_hex);
            let data_bytes = alloy_primitives::hex::decode(stripped).ok()?;

            Some(PrimitiveLog::new_unchecked(address, topics, Bytes::from(data_bytes)))
        })
        .collect()
}

/// Extract Upgraded events from receipt logs to detect proxy implementation changes.
fn detect_upgrades_from_logs(receipt: &serde_json::Value) -> Vec<DetectedUpgrade> {
    let logs = parse_receipt_logs(receipt);
    let events = decode_events(&logs);

    events
        .iter()
        .filter_map(|e| {
            if let treb_forge::events::decoder::ParsedEvent::Proxy(ProxyEvent::Upgraded {
                proxy_address,
                implementation,
            }) = e
            {
                Some(DetectedUpgrade {
                    proxy_address: *proxy_address,
                    new_implementation: *implementation,
                })
            } else {
                None
            }
        })
        .collect()
}

// ── ProcessedReceipt ────────────────────────────────────────────────────

/// Results of processing a transaction receipt.
#[derive(Debug, Clone)]
pub struct ProcessedReceipt {
    /// Contract creations found in traces.
    pub creations: Vec<TracedCreation>,
    /// Proxy patterns detected from trace (proxy → implementation for newly created contracts).
    pub proxy_patterns: Vec<DetectedProxy>,
    /// Proxy upgrades detected from Upgraded event logs (on existing deployments).
    pub proxy_upgrades: Vec<DetectedUpgrade>,
    /// Block number of the transaction.
    pub block_number: u64,
    /// Gas used by the transaction.
    pub gas_used: u64,
}

/// Fetch receipt + traces for a tx hash and extract all deployment/upgrade info.
pub async fn process_tx_receipt(
    rpc_url: &str,
    tx_hash: &str,
) -> anyhow::Result<ProcessedReceipt> {
    let client = rpc_client()?;

    // Fetch receipt
    let receipt = rpc_call(
        &client,
        rpc_url,
        "eth_getTransactionReceipt",
        serde_json::json!([tx_hash]),
    )
    .await?;

    if receipt.is_null() {
        bail!("transaction receipt not found: {tx_hash}");
    }

    let block_number =
        parse_hex_u64(receipt.get("blockNumber").and_then(|v| v.as_str()).unwrap_or("0x0"));
    let gas_used =
        parse_hex_u64(receipt.get("gasUsed").and_then(|v| v.as_str()).unwrap_or("0x0"));

    let receipt_status = receipt.get("status").and_then(|v| v.as_str()).unwrap_or("0x1");
    if receipt_status == "0x0" {
        bail!("transaction reverted: {tx_hash}");
    }

    // Detect proxy upgrades from receipt logs
    let proxy_upgrades = detect_upgrades_from_logs(&receipt);

    // Try trace for contract creations
    let trace_result = rpc_call(
        &client,
        rpc_url,
        "debug_traceTransaction",
        serde_json::json!([tx_hash, {"tracer": "callTracer"}]),
    )
    .await;

    let (mut creations, proxy_patterns) = match trace_result {
        Ok(trace) => {
            let creations = extract_creations_from_trace(&trace);
            let proxy_patterns = detect_proxy_patterns(&trace);
            (creations, proxy_patterns)
        }
        Err(_) => {
            // Trace not available — fall back to receipt-only for creations
            (Vec::new(), Vec::new())
        }
    };

    // Also check receipt contractAddress as safety net
    if let Some(addr) = receipt_contract_address(&receipt) {
        if !creations.iter().any(|c| c.address.eq_ignore_ascii_case(&addr)) {
            let sender = receipt
                .get("from")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            creations.push(TracedCreation {
                address: addr,
                from: sender,
                create_type: "CREATE".to_string(),
            });
        }
    }

    Ok(ProcessedReceipt {
        creations,
        proxy_patterns,
        proxy_upgrades,
        block_number,
        gas_used,
    })
}

// ── Registry application ────────────────────────────────────────────────

/// An existing deployment whose proxy implementation was updated.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpgradedDeployment {
    pub deployment_id: String,
    pub contract_name: String,
    pub proxy_address: String,
    pub old_implementation: String,
    pub new_implementation: String,
}

/// Results of applying receipt data to the registry.
#[derive(Debug, Clone)]
pub struct ReceiptApplicationResult {
    /// Existing deployments whose proxy implementation was updated.
    pub upgraded_deployments: Vec<UpgradedDeployment>,
    /// New contract creations that don't match existing deployments.
    pub new_creations: Vec<TracedCreation>,
    /// Existing deployment IDs whose address was confirmed in the trace.
    pub verified_deployments: Vec<String>,
}

/// Apply processed receipt data to the registry.
///
/// For each proxy upgrade: finds the deployment by address and updates `proxy_info`.
/// For each traced creation: checks if a deployment already exists at that address;
/// if yes, marks it as "verified"; if no, reports it as a new creation.
pub fn apply_receipt_to_registry(
    processed: &ProcessedReceipt,
    registry: &mut Registry,
    tx_hash: &str,
) -> anyhow::Result<ReceiptApplicationResult> {
    let mut upgraded_deployments = Vec::new();
    let mut new_creations = Vec::new();
    let mut verified_deployments = Vec::new();

    // Build address → deployment index for lookups
    let all_deployments = registry.list_deployments();
    let addr_to_dep: std::collections::HashMap<String, String> = all_deployments
        .iter()
        .map(|d| (d.address.to_lowercase(), d.id.clone()))
        .collect();

    // Process proxy upgrades from Upgraded events
    for upgrade in &processed.proxy_upgrades {
        let proxy_addr_lower = format!("{:#x}", upgrade.proxy_address).to_lowercase();
        if let Some(dep_id) = addr_to_dep.get(&proxy_addr_lower) {
            if let Some(dep) = registry.get_deployment(dep_id).cloned() {
                let old_impl = dep
                    .proxy_info
                    .as_ref()
                    .map(|p| p.implementation.clone())
                    .unwrap_or_default();

                let new_impl = format!("{:#x}", upgrade.new_implementation);

                // Update proxy_info
                let mut updated = dep.clone();
                let proxy_info = updated.proxy_info.get_or_insert_with(|| ProxyInfo {
                    proxy_type: String::new(),
                    implementation: String::new(),
                    admin: String::new(),
                    history: Vec::new(),
                });

                // Add current implementation to history before updating
                if !proxy_info.implementation.is_empty() {
                    proxy_info.history.push(ProxyUpgrade {
                        implementation_id: proxy_info.implementation.clone(),
                        upgraded_at: Utc::now(),
                        upgrade_tx_id: format!("tx-{tx_hash}"),
                    });
                }
                proxy_info.implementation = new_impl.clone();
                updated.updated_at = Utc::now();

                registry
                    .update_deployment(updated)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;

                upgraded_deployments.push(UpgradedDeployment {
                    deployment_id: dep_id.clone(),
                    contract_name: dep.contract_name.clone(),
                    proxy_address: dep.address.clone(),
                    old_implementation: old_impl,
                    new_implementation: new_impl,
                });
            }
        }
    }

    // Process traced creations
    for creation in &processed.creations {
        let addr_lower = creation.address.to_lowercase();
        if let Some(dep_id) = addr_to_dep.get(&addr_lower) {
            // Deployment already exists at this address — verified
            verified_deployments.push(dep_id.clone());
        } else {
            // New creation not in registry
            new_creations.push(creation.clone());
        }
    }

    Ok(ReceiptApplicationResult {
        upgraded_deployments,
        new_creations,
        verified_deployments,
    })
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_creations_from_simple_trace() {
        let trace = serde_json::json!({
            "type": "CALL",
            "from": "0xaaaa",
            "to": "0xbbbb",
            "calls": [
                {
                    "type": "CREATE",
                    "from": "0xbbbb",
                    "to": "0xcccc"
                },
                {
                    "type": "CREATE2",
                    "from": "0xbbbb",
                    "to": "0xdddd"
                }
            ]
        });

        let creations = extract_creations_from_trace(&trace);
        assert_eq!(creations.len(), 2);
        assert_eq!(creations[0].address, "0xcccc");
        assert_eq!(creations[0].create_type, "CREATE");
        assert_eq!(creations[1].address, "0xdddd");
        assert_eq!(creations[1].create_type, "CREATE2");
    }

    #[test]
    fn detect_proxy_with_delegatecall() {
        let trace = serde_json::json!({
            "type": "CALL",
            "from": "0xaaaa",
            "to": "0xbbbb",
            "calls": [
                {
                    "type": "CREATE",
                    "from": "0xbbbb",
                    "to": "0xproxy",
                    "calls": [
                        {
                            "type": "DELEGATECALL",
                            "from": "0xproxy",
                            "to": "0ximpl"
                        }
                    ]
                }
            ]
        });

        let proxies = detect_proxy_patterns(&trace);
        assert_eq!(proxies.len(), 1);
        assert_eq!(proxies[0].proxy_address, "0xproxy");
        assert_eq!(proxies[0].implementation_address, "0ximpl");
    }

    #[test]
    fn receipt_contract_address_ignores_zero() {
        let receipt = serde_json::json!({
            "contractAddress": "0x0000000000000000000000000000000000000000"
        });
        assert!(receipt_contract_address(&receipt).is_none());

        let receipt2 = serde_json::json!({
            "contractAddress": "0xabcdef1234567890abcdef1234567890abcdef12"
        });
        assert!(receipt_contract_address(&receipt2).is_some());
    }

    #[test]
    fn parse_receipt_logs_extracts_upgraded_event() {
        let impl_addr = alloy_primitives::address!("9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0");
        let proxy_addr = alloy_primitives::address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512");

        // Upgraded(address implementation) event signature:
        // keccak256("Upgraded(address)") = 0xbc7cd75a20ee27fd9adebab32041f755214dbc6bffa90cc0225b39da2e5c2d3b
        let upgraded_sig = "0xbc7cd75a20ee27fd9adebab32041f755214dbc6bffa90cc0225b39da2e5c2d3b";
        // Implementation address is indexed (topic[1]), left-padded to 32 bytes
        let impl_topic = format!("0x000000000000000000000000{}", &format!("{:#x}", impl_addr)[2..]);

        let receipt = serde_json::json!({
            "logs": [{
                "address": format!("{:#x}", proxy_addr),
                "topics": [upgraded_sig, impl_topic],
                "data": "0x",
            }]
        });

        let upgrades = detect_upgrades_from_logs(&receipt);
        assert_eq!(upgrades.len(), 1);
        assert_eq!(upgrades[0].proxy_address, proxy_addr);
        assert_eq!(upgrades[0].new_implementation, impl_addr);
    }

    #[test]
    fn parse_hex_u64_handles_prefix() {
        assert_eq!(parse_hex_u64("0xff"), 255);
        assert_eq!(parse_hex_u64("0Xff"), 255);
        assert_eq!(parse_hex_u64("ff"), 255);
        assert_eq!(parse_hex_u64("invalid"), 0);
    }
}
