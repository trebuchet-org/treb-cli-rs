use std::collections::HashMap;
use std::time::Duration;

use alloy_chains::Chain;
use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use treb_core::types::enums::TransactionStatus;
use treb_core::types::safe_transaction::Confirmation;
use treb_registry::Registry;

use crate::output;

// ── Safe Transaction Service response types ─────────────────────────────

/// Top-level response from Safe Transaction Service multisig-transactions endpoint.
#[derive(Debug, Deserialize)]
struct SafeServiceResponse {
    results: Vec<SafeServiceTx>,
}

/// A single multisig transaction as returned by the Safe Transaction Service.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct SafeServiceTx {
    safe_tx_hash: String,
    #[serde(default)]
    nonce: u64,
    #[serde(default)]
    is_executed: bool,
    /// On-chain transaction hash (present when executed).
    #[serde(default)]
    transaction_hash: Option<String>,
    /// When the transaction was executed on-chain.
    #[serde(default)]
    execution_date: Option<DateTime<Utc>>,
    #[serde(default)]
    confirmations: Vec<SafeServiceConfirmation>,
}

/// A signer confirmation from the Safe Transaction Service.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SafeServiceConfirmation {
    owner: String,
    signature: String,
    submission_date: DateTime<Utc>,
}

// ── JSON output types ───────────────────────────────────────────────────

#[derive(Serialize)]
struct SyncOutputJson {
    synced: usize,
    updated: usize,
    newly_executed: usize,
    removed: usize,
    errors: Vec<String>,
}

// ── Chain name mapping ──────────────────────────────────────────────────

/// Map a chain ID to the Safe Transaction Service URL chain name segment.
///
/// The Safe Transaction Service uses specific chain name segments that may
/// differ from alloy-chains naming. This function handles known overrides
/// and falls back to alloy-chains for the rest.
fn safe_service_chain_name(chain_id: u64) -> Option<String> {
    // Known Safe Transaction Service chain name segments
    match chain_id {
        1 => Some("mainnet".into()),
        10 => Some("optimism".into()),
        56 => Some("bsc".into()),
        100 => Some("gnosis-chain".into()),
        137 => Some("polygon".into()),
        324 => Some("zksync".into()),
        8453 => Some("base".into()),
        42161 => Some("arbitrum".into()),
        42220 => Some("celo".into()),
        43114 => Some("avalanche".into()),
        59144 => Some("linea".into()),
        534352 => Some("scroll".into()),
        11155111 => Some("sepolia".into()),
        84532 => Some("base-sepolia".into()),
        _ => {
            // Fall back to alloy-chains named chain
            let chain = Chain::from_id(chain_id);
            chain.named().map(|n| n.as_str().to_lowercase())
        }
    }
}

/// Build the Safe Transaction Service API URL for a given safe address and chain.
fn safe_service_url(chain_id: u64, safe_address: &str) -> Option<String> {
    safe_service_chain_name(chain_id).map(|chain_name| {
        format!(
            "https://safe-transaction-{chain_name}.safe.global/api/v1/safes/{safe_address}/multisig-transactions/"
        )
    })
}

/// Resolve a network name or numeric chain ID to a u64 chain ID.
fn resolve_chain_id(network: &str) -> anyhow::Result<u64> {
    // Try parsing as a numeric chain ID first
    if let Ok(id) = network.parse::<u64>() {
        return Ok(id);
    }

    // Try resolving as a named chain via alloy-chains
    let chain: Chain = network
        .parse()
        .map_err(|_| anyhow::anyhow!("unknown network: {network}"))?;
    Ok(chain.id())
}

// ── Main implementation ─────────────────────────────────────────────────

pub async fn run(
    network: Option<String>,
    clean: bool,
    debug: bool,
    json: bool,
) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;

    // Validate project structure
    if !cwd.join("foundry.toml").exists() {
        anyhow::bail!(
            "no foundry.toml found in the current directory.\n\
             Run this command from a Foundry project root."
        );
    }
    if !cwd.join(".treb").exists() {
        anyhow::bail!(
            "no .treb/ registry found. Run `treb init` to initialize."
        );
    }

    let mut registry = Registry::open(&cwd)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // Resolve --network filter to chain_id
    let chain_filter: Option<u64> = match &network {
        Some(name) => Some(resolve_chain_id(name)?),
        None => None,
    };

    // List all safe transactions, optionally filtered by chain
    let safe_txs = registry.list_safe_transactions();
    let filtered: Vec<_> = safe_txs
        .into_iter()
        .filter(|stx| match chain_filter {
            Some(cid) => stx.chain_id == cid,
            None => true,
        })
        .collect();

    if filtered.is_empty() {
        if json {
            output::print_json(&SyncOutputJson {
                synced: 0,
                updated: 0,
                newly_executed: 0,
                removed: 0,
                errors: vec![],
            })?;
        } else {
            match &network {
                Some(name) => println!("No safe transactions found for network {name}."),
                None => println!("No safe transactions in the registry."),
            }
        }
        return Ok(());
    }

    // Group safe transactions by (safe_address, chain_id) for batched API calls.
    // Store the safe_tx_hash for each group so we can match responses.
    let mut groups: HashMap<(String, u64), Vec<String>> = HashMap::new();
    for stx in &filtered {
        groups
            .entry((stx.safe_address.clone(), stx.chain_id))
            .or_default()
            .push(stx.safe_tx_hash.clone());
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("failed to build HTTP client")?;

    let mut updated_count = 0usize;
    let mut newly_executed_count = 0usize;
    let mut removed_count = 0usize;
    let mut errors: Vec<String> = Vec::new();
    let synced_count = filtered.len();

    for ((safe_address, chain_id), local_hashes) in &groups {
        let url = match safe_service_url(*chain_id, safe_address) {
            Some(u) => u,
            None => {
                let msg = format!(
                    "unsupported chain {chain_id} for Safe Transaction Service (safe {safe_address})"
                );
                errors.push(msg.clone());
                if !json {
                    eprintln!("warning: {msg}");
                }
                continue;
            }
        };

        // Fetch multisig transactions from the Safe Transaction Service
        let resp = match client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                let msg = format!(
                    "failed to reach Safe service for {} (chain {chain_id}): {e}",
                    output::truncate_address(safe_address)
                );
                errors.push(msg.clone());
                if !json {
                    eprintln!("warning: {msg}");
                }
                continue;
            }
        };

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();

        if debug {
            eprintln!(
                "[debug] GET {url}\n[debug] status: {status}\n[debug] body: {body}"
            );
        }

        if !status.is_success() {
            let msg = format!(
                "Safe service returned {status} for {} (chain {chain_id})",
                output::truncate_address(safe_address)
            );
            errors.push(msg.clone());
            if !json {
                eprintln!("warning: {msg}");
            }
            continue;
        }

        let service_resp: SafeServiceResponse = match serde_json::from_str(&body) {
            Ok(r) => r,
            Err(e) => {
                let msg = format!(
                    "failed to parse Safe service response for {} (chain {chain_id}): {e}",
                    output::truncate_address(safe_address)
                );
                errors.push(msg.clone());
                if !json {
                    eprintln!("warning: {msg}");
                }
                continue;
            }
        };

        // Index service results by safeTxHash for fast lookup
        let service_map: HashMap<&str, &SafeServiceTx> = service_resp
            .results
            .iter()
            .map(|tx| (tx.safe_tx_hash.as_str(), tx))
            .collect();

        for local_hash in local_hashes {
            if let Some(service_tx) = service_map.get(local_hash.as_str()) {
                // Get the current safe transaction from registry, clone, and update
                let local_stx = match registry.get_safe_transaction(local_hash) {
                    Some(stx) => stx.clone(),
                    None => continue,
                };

                let was_executed = local_stx.status == TransactionStatus::Executed;
                let mut updated_stx = local_stx;

                // Update confirmations from the service
                updated_stx.confirmations = service_tx
                    .confirmations
                    .iter()
                    .map(|c| Confirmation {
                        signer: c.owner.clone(),
                        signature: c.signature.clone(),
                        confirmed_at: c.submission_date,
                    })
                    .collect();

                // Update execution status if newly executed
                if service_tx.is_executed && updated_stx.status != TransactionStatus::Executed {
                    updated_stx.status = TransactionStatus::Executed;
                    updated_stx.executed_at = service_tx.execution_date;
                    updated_stx.execution_tx_hash = service_tx
                        .transaction_hash
                        .clone()
                        .unwrap_or_default();
                    newly_executed_count += 1;
                }

                // Persist updated safe transaction
                registry
                    .update_safe_transaction(updated_stx.clone())
                    .with_context(|| {
                        format!("failed to update safe transaction {local_hash}")
                    })?;
                updated_count += 1;

                // Update linked Transaction records when safe tx becomes Executed
                if !was_executed
                    && updated_stx.status == TransactionStatus::Executed
                {
                    for tx_id in &updated_stx.transaction_ids {
                        if let Some(tx) = registry.get_transaction(tx_id) {
                            let mut tx = tx.clone();
                            if tx.status != TransactionStatus::Executed {
                                tx.status = TransactionStatus::Executed;
                                tx.hash = updated_stx.execution_tx_hash.clone();
                                registry
                                    .update_transaction(tx)
                                    .with_context(|| {
                                        format!("failed to update transaction {tx_id}")
                                    })?;
                            }
                        }
                    }
                }
            } else if clean {
                // Safe transaction not found on the service — remove it
                registry
                    .remove_safe_transaction(local_hash)
                    .with_context(|| {
                        format!("failed to remove stale safe transaction {local_hash}")
                    })?;
                removed_count += 1;
            }
        }
    }

    // ── Output ──────────────────────────────────────────────────────────

    if json {
        output::print_json(&SyncOutputJson {
            synced: synced_count,
            updated: updated_count,
            newly_executed: newly_executed_count,
            removed: removed_count,
            errors,
        })?;
    } else {
        println!("Sync complete.");
        println!("  Safe transactions synced: {synced_count}");
        println!("  Updated:                  {updated_count}");
        println!("  Newly executed:           {newly_executed_count}");
        if clean {
            println!("  Removed (stale):          {removed_count}");
        }
        if !errors.is_empty() {
            println!("  Errors:                   {}", errors.len());
        }
    }

    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Chain name resolution ───────────────────────────────────────────

    #[test]
    fn safe_chain_name_mainnet() {
        assert_eq!(safe_service_chain_name(1).unwrap(), "mainnet");
    }

    #[test]
    fn safe_chain_name_optimism() {
        assert_eq!(safe_service_chain_name(10).unwrap(), "optimism");
    }

    #[test]
    fn safe_chain_name_polygon() {
        assert_eq!(safe_service_chain_name(137).unwrap(), "polygon");
    }

    #[test]
    fn safe_chain_name_arbitrum() {
        assert_eq!(safe_service_chain_name(42161).unwrap(), "arbitrum");
    }

    #[test]
    fn safe_chain_name_base() {
        assert_eq!(safe_service_chain_name(8453).unwrap(), "base");
    }

    #[test]
    fn safe_chain_name_celo() {
        assert_eq!(safe_service_chain_name(42220).unwrap(), "celo");
    }

    #[test]
    fn safe_chain_name_gnosis_chain() {
        assert_eq!(safe_service_chain_name(100).unwrap(), "gnosis-chain");
    }

    #[test]
    fn safe_chain_name_sepolia() {
        assert_eq!(safe_service_chain_name(11155111).unwrap(), "sepolia");
    }

    // ── Safe service URL construction ───────────────────────────────────

    #[test]
    fn safe_service_url_mainnet() {
        let url = safe_service_url(1, "0x1234567890abcdef1234567890abcdef12345678").unwrap();
        assert_eq!(
            url,
            "https://safe-transaction-mainnet.safe.global/api/v1/safes/0x1234567890abcdef1234567890abcdef12345678/multisig-transactions/"
        );
    }

    #[test]
    fn safe_service_url_polygon() {
        let url = safe_service_url(137, "0xabcdef").unwrap();
        assert!(url.contains("safe-transaction-polygon.safe.global"));
        assert!(url.contains("0xabcdef"));
    }

    // ── Chain ID resolution ─────────────────────────────────────────────

    #[test]
    fn resolve_chain_id_numeric() {
        assert_eq!(resolve_chain_id("1").unwrap(), 1);
        assert_eq!(resolve_chain_id("137").unwrap(), 137);
        assert_eq!(resolve_chain_id("42161").unwrap(), 42161);
    }

    #[test]
    fn resolve_chain_id_named() {
        assert_eq!(resolve_chain_id("mainnet").unwrap(), 1);
        assert_eq!(resolve_chain_id("optimism").unwrap(), 10);
        assert_eq!(resolve_chain_id("polygon").unwrap(), 137);
    }

    #[test]
    fn resolve_chain_id_unknown() {
        assert!(resolve_chain_id("nonexistent_chain_xyz").is_err());
    }

    // ── SafeServiceResponse deserialization ─────────────────────────────

    #[test]
    fn deserialize_safe_service_response_executed() {
        let json = r#"{
            "results": [
                {
                    "safeTxHash": "0xabc123",
                    "nonce": 42,
                    "isExecuted": true,
                    "transactionHash": "0xdef456",
                    "executionDate": "2025-01-15T10:30:00Z",
                    "confirmations": [
                        {
                            "owner": "0x1111111111111111111111111111111111111111",
                            "signature": "0xsig1",
                            "submissionDate": "2025-01-14T08:00:00Z"
                        },
                        {
                            "owner": "0x2222222222222222222222222222222222222222",
                            "signature": "0xsig2",
                            "submissionDate": "2025-01-14T09:00:00Z"
                        }
                    ]
                }
            ]
        }"#;

        let resp: SafeServiceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.results.len(), 1);
        let tx = &resp.results[0];
        assert_eq!(tx.safe_tx_hash, "0xabc123");
        assert_eq!(tx.nonce, 42);
        assert!(tx.is_executed);
        assert_eq!(tx.transaction_hash.as_deref(), Some("0xdef456"));
        assert!(tx.execution_date.is_some());
        assert_eq!(tx.confirmations.len(), 2);
        assert_eq!(tx.confirmations[0].owner, "0x1111111111111111111111111111111111111111");
    }

    #[test]
    fn deserialize_safe_service_response_pending() {
        let json = r#"{
            "results": [
                {
                    "safeTxHash": "0xpending123",
                    "nonce": 10,
                    "isExecuted": false,
                    "confirmations": [
                        {
                            "owner": "0x3333333333333333333333333333333333333333",
                            "signature": "0xsig3",
                            "submissionDate": "2025-02-01T12:00:00Z"
                        }
                    ]
                }
            ]
        }"#;

        let resp: SafeServiceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.results.len(), 1);
        let tx = &resp.results[0];
        assert!(!tx.is_executed);
        assert!(tx.transaction_hash.is_none());
        assert!(tx.execution_date.is_none());
        assert_eq!(tx.confirmations.len(), 1);
    }

    #[test]
    fn deserialize_safe_service_response_empty() {
        let json = r#"{ "results": [] }"#;
        let resp: SafeServiceResponse = serde_json::from_str(json).unwrap();
        assert!(resp.results.is_empty());
    }

    // ── Registry integration tests ──────────────────────────────────────

    #[test]
    fn sync_output_json_serialization() {
        let output = SyncOutputJson {
            synced: 5,
            updated: 3,
            newly_executed: 1,
            removed: 0,
            errors: vec!["some error".into()],
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"synced\":5"));
        assert!(json.contains("\"updated\":3"));
        assert!(json.contains("\"newly_executed\":1"));
        assert!(json.contains("\"removed\":0"));
        assert!(json.contains("some error"));
    }
}
