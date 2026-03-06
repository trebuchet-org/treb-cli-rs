use std::collections::HashMap;

use alloy_chains::Chain;
use anyhow::Context;
use owo_colors::OwoColorize;
use serde::Serialize;
use treb_core::types::{enums::TransactionStatus, safe_transaction::Confirmation};
use treb_registry::Registry;
use treb_safe::{
    SafeServiceClient,
    types::{SafeServiceMultisigResponse, SafeServiceTx},
};

use crate::{output, ui::color};

// ── JSON output types ───────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncOutputJson {
    synced: usize,
    updated: usize,
    newly_executed: usize,
    removed: usize,
    errors: Vec<String>,
}

// ── Chain ID resolution ─────────────────────────────────────────────────

/// Resolve a network name or numeric chain ID to a u64 chain ID.
fn resolve_chain_id(network: &str) -> anyhow::Result<u64> {
    // Try parsing as a numeric chain ID first
    if let Ok(id) = network.parse::<u64>() {
        return Ok(id);
    }

    // Try resolving as a named chain via alloy-chains
    let chain: Chain =
        network.parse().map_err(|_| anyhow::anyhow!("unknown network: {network}"))?;
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
        anyhow::bail!("no .treb/ registry found. Run `treb init` to initialize.");
    }

    let mut registry = Registry::open(&cwd).map_err(|e| anyhow::anyhow!("{e}"))?;

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

    if !json {
        output::print_stage("\u{1f50d}", "Syncing safe transactions...");
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

    // Cache SafeServiceClient instances per chain_id to avoid redundant construction.
    let mut clients: HashMap<u64, SafeServiceClient> = HashMap::new();

    let mut updated_count = 0usize;
    let mut newly_executed_count = 0usize;
    let mut removed_count = 0usize;
    let mut errors: Vec<String> = Vec::new();
    let synced_count = filtered.len();

    for ((safe_address, chain_id), local_hashes) in &groups {
        let client = match clients.get(chain_id) {
            Some(c) => c,
            None => match SafeServiceClient::new(*chain_id) {
                Some(c) => {
                    clients.insert(*chain_id, c);
                    clients.get(chain_id).unwrap()
                }
                None => {
                    let msg = format!(
                        "unsupported chain {chain_id} for Safe Transaction Service (safe {safe_address})"
                    );
                    errors.push(msg.clone());
                    if !json {
                        eprintln!("{}", output::format_warning_banner("\u{26a0}\u{fe0f}", &msg));
                    }
                    continue;
                }
            },
        };

        if debug {
            eprintln!(
                "[debug] GET {}/safes/{}/multisig-transactions/",
                client.base_url(),
                safe_address
            );
        }

        // Fetch multisig transactions from the Safe Transaction Service
        let service_resp: SafeServiceMultisigResponse =
            match client.get_multisig_transactions(safe_address).await {
                Ok(resp) => {
                    if debug {
                        eprintln!("[debug] received {} results", resp.results.len());
                    }
                    resp
                }
                Err(e) => {
                    let msg = format!(
                        "Safe service error for {} (chain {chain_id}): {e}",
                        output::truncate_address(safe_address)
                    );
                    errors.push(msg.clone());
                    if !json {
                        eprintln!("{}", output::format_warning_banner("\u{26a0}\u{fe0f}", &msg));
                    }
                    continue;
                }
            };

        // Index service results by safeTxHash for fast lookup
        let service_map: HashMap<&str, &SafeServiceTx> =
            service_resp.results.iter().map(|tx| (tx.safe_tx_hash.as_str(), tx)).collect();

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
                    updated_stx.execution_tx_hash =
                        service_tx.transaction_hash.clone().unwrap_or_default();
                    newly_executed_count += 1;
                }

                // Persist updated safe transaction
                registry
                    .update_safe_transaction(updated_stx.clone())
                    .with_context(|| format!("failed to update safe transaction {local_hash}"))?;
                updated_count += 1;

                // Update linked Transaction records when safe tx becomes Executed
                if !was_executed && updated_stx.status == TransactionStatus::Executed {
                    for tx_id in &updated_stx.transaction_ids {
                        if let Some(tx) = registry.get_transaction(tx_id) {
                            let mut tx = tx.clone();
                            if tx.status != TransactionStatus::Executed {
                                tx.status = TransactionStatus::Executed;
                                tx.hash = updated_stx.execution_tx_hash.clone();
                                registry.update_transaction(tx).with_context(|| {
                                    format!("failed to update transaction {tx_id}")
                                })?;
                            }
                        }
                    }
                }
            } else if clean {
                // Safe transaction not found on the service — remove it
                registry.remove_safe_transaction(local_hash).with_context(|| {
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
        println!("{}", output::format_stage("\u{2705}", "Sync complete."));
        println!("  Safe transactions synced: {synced_count}");
        if color::is_color_enabled() {
            println!("  Updated:                  {}", updated_count.style(color::WARNING));
            println!("  Newly executed:           {}", newly_executed_count.style(color::SUCCESS));
        } else {
            println!("  Updated:                  {updated_count}");
            println!("  Newly executed:           {newly_executed_count}");
        }
        if clean {
            println!("  Removed (stale):          {removed_count}");
        }
        if !errors.is_empty() {
            if color::is_color_enabled() {
                println!("  Errors:                   {}", errors.len().style(color::ERROR));
            } else {
                println!("  Errors:                   {}", errors.len());
            }
        }
    }

    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use treb_safe::types::{SafeServiceConfirmation, SafeServiceMultisigResponse};

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

    // ── SafeServiceMultisigResponse deserialization ─────────────────────
    // These tests verify that the treb_safe types work correctly for sync's
    // deserialization needs (confirmations, execution status, etc.)

    #[test]
    fn deserialize_safe_service_response_executed() {
        let json = r#"{
            "count": 1,
            "next": null,
            "previous": null,
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

        let resp: SafeServiceMultisigResponse = serde_json::from_str(json).unwrap();
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
            "count": 1,
            "next": null,
            "previous": null,
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

        let resp: SafeServiceMultisigResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.results.len(), 1);
        let tx = &resp.results[0];
        assert!(!tx.is_executed);
        assert!(tx.transaction_hash.is_none());
        assert!(tx.execution_date.is_none());
        assert_eq!(tx.confirmations.len(), 1);
    }

    #[test]
    fn deserialize_safe_service_response_empty() {
        let json = r#"{ "count": 0, "next": null, "previous": null, "results": [] }"#;
        let resp: SafeServiceMultisigResponse = serde_json::from_str(json).unwrap();
        assert!(resp.results.is_empty());
    }

    // ── Confirmation field mapping ──────────────────────────────────────

    #[test]
    fn confirmation_field_mapping_from_service() {
        let json = r#"{
            "owner": "0xOwnerAddr",
            "signature": "0xdeadbeef",
            "submissionDate": "2025-06-15T14:30:00Z"
        }"#;

        let conf: SafeServiceConfirmation = serde_json::from_str(json).unwrap();
        // Verify the fields sync.rs uses to build Confirmation records
        let mapped = Confirmation {
            signer: conf.owner.clone(),
            signature: conf.signature.clone(),
            confirmed_at: conf.submission_date,
        };
        assert_eq!(mapped.signer, "0xOwnerAddr");
        assert_eq!(mapped.signature, "0xdeadbeef");
        assert!(mapped.confirmed_at.timestamp() > 0);
    }

    // ── Client construction via treb_safe ───────────────────────────────

    #[test]
    fn safe_service_client_supported_chains() {
        // Verify SafeServiceClient can be constructed for all chains sync needs
        let chains =
            [1, 10, 56, 100, 137, 324, 8453, 42161, 42220, 43114, 59144, 534352, 11155111, 84532];
        for chain_id in chains {
            assert!(
                SafeServiceClient::new(chain_id).is_some(),
                "chain {chain_id} should be supported"
            );
        }
    }

    #[test]
    fn safe_service_client_unsupported_chain() {
        assert!(SafeServiceClient::new(999999).is_none());
    }

    #[test]
    fn safe_service_client_base_url_format() {
        let client = SafeServiceClient::new(1).unwrap();
        assert_eq!(client.base_url(), "https://safe-transaction-mainnet.safe.global/api/v1");
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
        assert!(json.contains("\"newlyExecuted\":1"));
        assert!(json.contains("\"removed\":0"));
        assert!(json.contains("some error"));
    }
}
