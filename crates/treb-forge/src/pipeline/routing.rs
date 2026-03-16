//! Transaction routing — partitions broadcastable transactions by sender type
//! and dispatches each group through the appropriate broadcast path.
//!
//! After script execution, forge captures `BroadcastableTransactions` with a
//! `from` address on each tx. This module partitions them into consecutive
//! "runs" grouped by sender, then routes each run:
//!
//! - **Wallet**: sign and broadcast directly (or impersonate on fork)
//! - **Safe**: batch via MultiSend → execTransaction (1/1) or propose (multi-sig)
//! - **Governor**: build Governor.propose() → broadcast via proposer

use std::collections::HashMap;

use alloy_primitives::{Address, B256};
use treb_core::error::TrebError;

use crate::sender::{ResolvedSender, SenderCategory};
use crate::script::BroadcastReceipt;

/// A consecutive group of transactions from the same sender.
#[derive(Debug)]
pub struct TransactionRun {
    /// Sender role name (e.g. "deployer", "admin").
    pub sender_role: String,
    /// Sender category (Wallet, Safe, Governor).
    pub category: SenderCategory,
    /// The sender's on-chain address.
    pub sender_address: Address,
    /// Indices into the original BroadcastableTransactions vec.
    pub tx_indices: Vec<usize>,
}

/// Partition `BroadcastableTransactions` into consecutive runs by sender.
///
/// Adjacent transactions with the same `from` address are grouped together.
/// When `from` changes, a new run starts. This preserves execution ordering
/// while enabling per-sender routing (wallet broadcast vs Safe proposal vs
/// Governor proposal).
pub fn partition_into_runs(
    btxs: &foundry_cheatcodes::BroadcastableTransactions,
    resolved_senders: &HashMap<String, ResolvedSender>,
    sender_labels: &HashMap<Address, String>,
) -> Vec<TransactionRun> {
    let addr_to_role: HashMap<Address, (String, SenderCategory)> = resolved_senders
        .iter()
        .map(|(role, sender)| {
            (sender.sender_address(), (role.clone(), sender.category()))
        })
        .collect();

    let mut runs: Vec<TransactionRun> = Vec::new();

    for (idx, btx) in btxs.iter().enumerate() {
        let from = btx.transaction.from().unwrap_or_default();

        // Check if this tx extends the current run (same sender)
        if let Some(current) = runs.last_mut() {
            if current.sender_address == from {
                current.tx_indices.push(idx);
                continue;
            }
        }

        // New run — look up sender info
        let (role, category) = addr_to_role
            .get(&from)
            .cloned()
            .unwrap_or_else(|| {
                let label = sender_labels
                    .get(&from)
                    .cloned()
                    .unwrap_or_else(|| format!("{:#x}", from));
                (label, SenderCategory::Wallet)
            });

        runs.push(TransactionRun {
            sender_role: role,
            category,
            sender_address: from,
            tx_indices: vec![idx],
        });
    }

    runs
}

/// Returns true if all runs are wallet senders (no Safe/Governor routing needed).
pub fn all_wallet_runs(runs: &[TransactionRun]) -> bool {
    runs.iter().all(|r| matches!(r.category, SenderCategory::Wallet))
}

// ---------------------------------------------------------------------------
// Unified routing
// ---------------------------------------------------------------------------

/// Result of routing a single transaction run.
#[derive(Debug)]
pub enum RunResult {
    /// All txs were broadcast on-chain and confirmed.
    Broadcast(Vec<BroadcastReceipt>),
    /// Txs were proposed to the Safe Transaction Service (live mode).
    SafeProposed {
        safe_tx_hash: B256,
        safe_address: Address,
        nonce: u64,
        tx_count: usize,
    },
    /// Txs were submitted as a Governor proposal (live mode).
    GovernorProposed {
        proposal_id: String,
        governor_address: Address,
        tx_count: usize,
    },
}

/// Context needed for transaction routing.
pub struct RouteContext<'a> {
    pub rpc_url: &'a str,
    pub chain_id: u64,
    pub is_fork: bool,
    pub resolved_senders: &'a HashMap<String, ResolvedSender>,
    pub sender_labels: &'a HashMap<Address, String>,
    pub sender_configs: &'a HashMap<String, treb_config::SenderConfig>,
}

/// Route all broadcastable transactions through the appropriate paths.
///
/// Partitions transactions into runs by sender, then dispatches each run:
/// - Wallet → impersonate (fork) or sign+send (live)
/// - Safe → impersonate (fork) or propose to Safe Service (live)
/// - Governor → impersonate (fork) or propose (live)
///
/// Returns the runs paired with their results, preserving order.
pub async fn route_all(
    btxs: &foundry_cheatcodes::BroadcastableTransactions,
    ctx: &RouteContext<'_>,
) -> Result<Vec<(TransactionRun, RunResult)>, TrebError> {
    let runs = partition_into_runs(btxs, ctx.resolved_senders, ctx.sender_labels);
    let mut results = Vec::with_capacity(runs.len());

    for run in runs {
        let result = match run.category {
            SenderCategory::Wallet => {
                let receipts = broadcast_wallet_run(
                    ctx.rpc_url, &run, btxs, ctx.is_fork,
                ).await?;
                RunResult::Broadcast(receipts)
            }
            SenderCategory::Safe => {
                let resolved_sender = ctx.resolved_senders.get(&run.sender_role)
                    .ok_or_else(|| TrebError::Forge(format!(
                        "sender '{}' not found", run.sender_role
                    )))?;
                let safe_result = broadcast_safe_run(
                    ctx.rpc_url, &run, btxs, resolved_sender,
                    ctx.chain_id, ctx.sender_configs, ctx.is_fork,
                ).await?;
                match safe_result {
                    SafeRunResult::Executed(receipts) => RunResult::Broadcast(receipts),
                    SafeRunResult::Proposed { safe_tx_hash, safe_address, nonce, tx_count } => {
                        RunResult::SafeProposed { safe_tx_hash, safe_address, nonce, tx_count }
                    }
                }
            }
            SenderCategory::Governor => {
                let resolved_sender = ctx.resolved_senders.get(&run.sender_role)
                    .ok_or_else(|| TrebError::Forge(format!(
                        "sender '{}' not found", run.sender_role
                    )))?;
                let receipts = broadcast_governor_run(
                    ctx.rpc_url, &run, btxs, resolved_sender, ctx.is_fork,
                ).await?;
                RunResult::Broadcast(receipts)
            }
        };
        results.push((run, result));
    }

    Ok(results)
}

/// Flatten run results into a single ordered receipt list.
///
/// For `Broadcast` results, receipts are included directly.
/// For `Proposed` results, placeholder receipts with zero hash are inserted
/// (one per inner transaction) so the list stays aligned with the original
/// BroadcastableTransactions indices.
pub fn flatten_receipts(results: &[(TransactionRun, RunResult)]) -> Vec<BroadcastReceipt> {
    let mut receipts = Vec::new();
    for (run, result) in results {
        match result {
            RunResult::Broadcast(r) => receipts.extend(r.clone()),
            RunResult::SafeProposed { tx_count, .. }
            | RunResult::GovernorProposed { tx_count, .. } => {
                for _ in 0..*tx_count {
                    receipts.push(BroadcastReceipt {
                        hash: B256::ZERO,
                        block_number: 0,
                        gas_used: 0,
                        status: true,
                        contract_name: None,
                        contract_address: None,
                    });
                }
            }
        }
    }
    receipts
}

/// Broadcast a wallet run's transactions to an RPC endpoint.
///
/// For fork mode (Anvil): uses `anvil_impersonateAccount` + `eth_sendTransaction`.
/// For live mode: signs each transaction with the sender's private key and
/// uses `eth_sendRawTransaction`.
///
/// Returns one `BroadcastReceipt` per transaction.
pub async fn broadcast_wallet_run(
    rpc_url: &str,
    run: &TransactionRun,
    btxs: &foundry_cheatcodes::BroadcastableTransactions,
    is_fork: bool,
) -> Result<Vec<BroadcastReceipt>, TrebError> {
    let client = reqwest::Client::new();
    let mut receipts = Vec::new();

    for &tx_idx in &run.tx_indices {
        let btx = btxs.iter().nth(tx_idx).ok_or_else(|| {
            TrebError::Forge(format!("transaction index {tx_idx} out of range"))
        })?;

        let from = btx.transaction.from().unwrap_or_default();

        // Build the transaction object
        let mut tx_obj = serde_json::Map::new();
        tx_obj.insert("from".into(), serde_json::json!(format!("{:#x}", from)));

        if let Some(to) = btx.transaction.to() {
            match to {
                alloy_primitives::TxKind::Call(addr) => {
                    tx_obj.insert("to".into(), serde_json::json!(format!("{:#x}", addr)));
                }
                alloy_primitives::TxKind::Create => {}
            }
        }

        if let Some(input) = btx.transaction.input() {
            if !input.is_empty() {
                tx_obj.insert(
                    "data".into(),
                    serde_json::json!(format!("0x{}", alloy_primitives::hex::encode(input))),
                );
            }
        }

        let value = btx.transaction.value().unwrap_or_default();
        if !value.is_zero() {
            tx_obj.insert("value".into(), serde_json::json!(format!("{:#x}", value)));
        }

        // High gas limit — let the node estimate or cap
        tx_obj.insert("gas".into(), serde_json::json!("0x1c9c380")); // 30M

        if is_fork {
            // Fork mode: impersonate + sendTransaction (no signing needed)
            let impersonate = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "anvil_impersonateAccount",
                "params": [format!("{:#x}", from)],
                "id": 1,
            });
            client
                .post(rpc_url)
                .json(&impersonate)
                .send()
                .await
                .map_err(|e| TrebError::Forge(format!("impersonate failed: {e}")))?;
        }

        let send_method = if is_fork { "eth_sendTransaction" } else { "eth_sendRawTransaction" };

        // For live mode, we'd need to sign here — currently only fork mode is supported.
        // TODO: implement signing with WalletSigner for live broadcast
        if !is_fork {
            return Err(TrebError::Forge(
                "live network broadcast through routing is not yet supported; \
                 use fork mode or wallet-only scripts for live broadcast"
                    .into(),
            ));
        }

        let send_tx = serde_json::json!({
            "jsonrpc": "2.0",
            "method": send_method,
            "params": [tx_obj],
            "id": 2,
        });

        let resp: serde_json::Value = client
            .post(rpc_url)
            .json(&send_tx)
            .send()
            .await
            .map_err(|e| TrebError::Forge(format!("send tx failed: {e}")))?
            .json()
            .await
            .map_err(|e| TrebError::Forge(format!("parse send response failed: {e}")))?;

        if let Some(err) = resp.get("error") {
            return Err(TrebError::Forge(format!(
                "tx {} from {:#x} failed: {}",
                tx_idx, from, err
            )));
        }

        let tx_hash_hex = resp
            .get("result")
            .and_then(|r| r.as_str())
            .unwrap_or("0x0");

        // Fetch receipt
        let receipt = fetch_receipt(&client, rpc_url, tx_hash_hex).await?;
        receipts.push(receipt);

        if is_fork {
            let stop = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "anvil_stopImpersonatingAccount",
                "params": [format!("{:#x}", from)],
                "id": 3,
            });
            let _ = client.post(rpc_url).json(&stop).send().await;
        }
    }

    Ok(receipts)
}

/// Fetch a transaction receipt by hash.
async fn fetch_receipt(
    client: &reqwest::Client,
    rpc_url: &str,
    tx_hash: &str,
) -> Result<BroadcastReceipt, TrebError> {
    let req = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "eth_getTransactionReceipt",
        "params": [tx_hash],
        "id": 1,
    });

    let resp: serde_json::Value = client
        .post(rpc_url)
        .json(&req)
        .send()
        .await
        .map_err(|e| TrebError::Forge(format!("fetch receipt failed: {e}")))?
        .json()
        .await
        .map_err(|e| TrebError::Forge(format!("parse receipt failed: {e}")))?;

    let result = resp.get("result").ok_or_else(|| {
        TrebError::Forge(format!("no receipt for tx {tx_hash}"))
    })?;

    let hash = result
        .get("transactionHash")
        .and_then(|v| v.as_str())
        .unwrap_or(tx_hash);
    let hash = hash.parse::<B256>().unwrap_or_default();

    let block_hex = result
        .get("blockNumber")
        .and_then(|v| v.as_str())
        .unwrap_or("0x0");
    let block_number = u64::from_str_radix(
        block_hex.strip_prefix("0x").unwrap_or(block_hex),
        16,
    )
    .unwrap_or(0);

    let gas_hex = result
        .get("gasUsed")
        .and_then(|v| v.as_str())
        .unwrap_or("0x0");
    let gas_used = u64::from_str_radix(
        gas_hex.strip_prefix("0x").unwrap_or(gas_hex),
        16,
    )
    .unwrap_or(0);

    let status_hex = result
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("0x1");
    let status = status_hex != "0x0";

    let contract_address = result
        .get("contractAddress")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<Address>().ok());

    Ok(BroadcastReceipt {
        hash,
        block_number,
        gas_used,
        status,
        contract_name: None,
        contract_address,
    })
}

// ---------------------------------------------------------------------------
// Safe transaction routing
// ---------------------------------------------------------------------------

/// Result of routing a Safe run.
pub enum SafeRunResult {
    /// Fork mode: transactions executed directly via impersonation.
    Executed(Vec<BroadcastReceipt>),
    /// Live mode: transaction proposed to Safe Transaction Service.
    Proposed {
        safe_tx_hash: B256,
        safe_address: Address,
        nonce: u64,
        tx_count: usize,
    },
}

/// Route a Safe run's transactions.
///
/// **Fork mode**: impersonate the Safe address on Anvil and send each tx
/// directly — no MultiSend, no execTransaction, no signing needed.
///
/// **Live mode**: batch via MultiSend, sign with sub-signer key, propose
/// to Safe Transaction Service. Returns `Proposed` so the caller can
/// poll for execution or save as queued.
pub async fn broadcast_safe_run(
    rpc_url: &str,
    run: &TransactionRun,
    btxs: &foundry_cheatcodes::BroadcastableTransactions,
    resolved_sender: &ResolvedSender,
    chain_id: u64,
    sender_configs: &std::collections::HashMap<String, treb_config::SenderConfig>,
    is_fork: bool,
) -> Result<SafeRunResult, TrebError> {
    // Fork mode: just impersonate the Safe address and send each tx directly.
    // Anvil doesn't care that it's a contract address — impersonation bypasses
    // all signature checks. This is the simplest and most reliable path.
    if is_fork {
        let receipts = broadcast_wallet_run(rpc_url, run, btxs, true).await?;
        return Ok(SafeRunResult::Executed(receipts));
    }

    // Live mode: propose via Safe Transaction Service
    let safe_address = match resolved_sender {
        ResolvedSender::Safe { safe_address, .. } => *safe_address,
        _ => return Err(TrebError::Safe("expected Safe sender".into())),
    };

    // Build MultiSend batch from the run's transactions
    let operations: Vec<treb_safe::MultiSendOperation> = run.tx_indices.iter()
        .filter_map(|&idx| btxs.iter().nth(idx))
        .map(|btx| {
            let to = btx.transaction.to()
                .and_then(|kind| match kind {
                    alloy_primitives::TxKind::Call(addr) => Some(addr),
                    alloy_primitives::TxKind::Create => None,
                })
                .unwrap_or(Address::ZERO);
            let value = btx.transaction.value().unwrap_or_default();
            let data = btx.transaction.input().cloned().unwrap_or_default();
            treb_safe::MultiSendOperation {
                operation: 0, // Call
                to,
                value: alloy_primitives::U256::from(value),
                data,
            }
        })
        .collect();

    // Single tx → direct call; multiple → MultiSend DelegateCall
    let (to, data, operation) = if operations.len() == 1 {
        let op = &operations[0];
        (op.to, op.data.clone(), 0u8)
    } else {
        let multi_send_data = treb_safe::encode_multi_send_call(&operations);
        (treb_safe::MULTI_SEND_ADDRESS, multi_send_data, 1u8)
    };

    // Query Safe nonce from the Transaction Service
    let safe_client = treb_safe::SafeServiceClient::new(chain_id)
        .ok_or_else(|| TrebError::Safe(format!(
            "Safe Transaction Service not available for chain {chain_id}"
        )))?;
    let safe_info = safe_client
        .get_safe_info(&format!("{:#x}", safe_address))
        .await?;

    // Build SafeTx, compute EIP-712 hash, sign
    let safe_tx = treb_safe::SafeTx {
        to,
        value: alloy_primitives::U256::ZERO,
        data: data.to_vec().into(),
        operation,
        safeTxGas: alloy_primitives::U256::ZERO,
        baseGas: alloy_primitives::U256::ZERO,
        gasPrice: alloy_primitives::U256::ZERO,
        gasToken: Address::ZERO,
        refundReceiver: Address::ZERO,
        nonce: alloy_primitives::U256::from(safe_info.nonce),
    };
    let safe_tx_hash = treb_safe::compute_safe_tx_hash(chain_id, safe_address, &safe_tx);

    let signer_key_hex = crate::sender::extract_signing_key(
        &run.sender_role, resolved_sender, sender_configs,
    ).ok_or_else(|| TrebError::Safe(format!(
        "no signing key for Safe sender '{}'", run.sender_role,
    )))?;
    let key_bytes: B256 = signer_key_hex.parse()
        .map_err(|e| TrebError::Safe(format!("invalid signer key: {e}")))?;
    let wallet_signer = foundry_wallets::WalletSigner::from_private_key(&key_bytes)
        .map_err(|e| TrebError::Safe(format!("failed to create signer: {e}")))?;
    let signature = treb_safe::sign_safe_tx(&wallet_signer, safe_tx_hash).await?;

    // Propose
    let signer_addr = alloy_signer::Signer::address(&wallet_signer);
    let request = treb_safe::types::ProposeRequest {
        to: format!("{:#x}", to),
        value: "0".into(),
        data: Some(format!("0x{}", alloy_primitives::hex::encode(&data))),
        operation,
        safe_tx_gas: "0".into(),
        base_gas: "0".into(),
        gas_price: "0".into(),
        gas_token: format!("{:#x}", Address::ZERO),
        refund_receiver: format!("{:#x}", Address::ZERO),
        nonce: safe_info.nonce,
        contract_transaction_hash: format!("{:#x}", safe_tx_hash),
        sender: format!("{:#x}", signer_addr),
        signature: format!("0x{}", alloy_primitives::hex::encode(&signature)),
        origin: Some("treb".into()),
    };

    safe_client
        .propose_transaction(&format!("{:#x}", safe_address), &request)
        .await?;

    Ok(SafeRunResult::Proposed {
        safe_tx_hash,
        safe_address,
        nonce: safe_info.nonce,
        tx_count: run.tx_indices.len(),
    })
}

/// Poll the Safe Transaction Service until a proposed tx is executed.
///
/// Returns the on-chain execution tx hash if executed, or `None` if the
/// caller chose to skip (via the `should_continue` callback returning true).
pub async fn poll_safe_execution(
    chain_id: u64,
    safe_tx_hash: &B256,
    should_continue: impl Fn() -> bool,
) -> Result<Option<String>, TrebError> {
    let safe_client = treb_safe::SafeServiceClient::new(chain_id)
        .ok_or_else(|| TrebError::Safe(format!(
            "Safe Transaction Service not available for chain {chain_id}"
        )))?;
    let hash_hex = format!("{:#x}", safe_tx_hash);

    loop {
        let tx = safe_client.get_transaction(&hash_hex).await?;
        if tx.is_executed {
            return Ok(tx.transaction_hash);
        }
        if should_continue() {
            return Ok(None);
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

// ---------------------------------------------------------------------------
// Governor routing stub
// ---------------------------------------------------------------------------

/// Route a Governor run's transactions.
///
/// **Fork mode**: impersonate and send directly (same as wallet).
/// **Live mode**: not yet implemented.
pub async fn broadcast_governor_run(
    rpc_url: &str,
    run: &TransactionRun,
    btxs: &foundry_cheatcodes::BroadcastableTransactions,
    _resolved_sender: &ResolvedSender,
    is_fork: bool,
) -> Result<Vec<BroadcastReceipt>, TrebError> {
    if is_fork {
        return broadcast_wallet_run(rpc_url, run, btxs, true).await;
    }
    Err(TrebError::Forge(
        "Governor proposal routing on live networks is not yet implemented".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_wallet_runs_returns_true_for_empty() {
        assert!(all_wallet_runs(&[]));
    }
}
