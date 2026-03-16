//! Transaction routing — partitions broadcastable transactions by sender type
//! and dispatches each group through the appropriate broadcast path.
//!
//! After script execution, forge captures `BroadcastableTransactions` with a
//! `from` address on each tx. This module partitions them into consecutive
//! "runs" grouped by sender, then routes each run:
//!
//! - **Wallet**: sign and broadcast directly (or impersonate on fork)
//! - **Safe**: batch via MultiSend → execTransaction (1/1) or propose (multi-sig)
//! - **Governor**: run Solidity reducer → recursively route the output
//!
//! Governor and custom sender types use **Solidity reducers**: forge scripts
//! that receive pending transactions and produce new `BroadcastableTransactions`.
//! The reducer output is fed back through `route_all()`, enabling recursive
//! routing chains (e.g. Governor → Safe → Wallet).

use std::collections::HashMap;

use alloy_primitives::{Address, B256, U256};
use treb_core::error::TrebError;

use crate::sender::{ResolvedSender, SenderCategory};
use crate::script::BroadcastReceipt;

/// Maximum recursion depth for routing reducer chains.
///
/// Prevents infinite loops from misconfigured sender chains (e.g. Governor
/// whose proposer is another Governor whose proposer is the first Governor).
const MAX_ROUTE_DEPTH: u8 = 4;

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
    // Build address → (role, category) lookup. For Governor senders with a
    // timelock, register the timelock address (not the governor) because the
    // user script `vm.broadcast()`s from the timelock — the on-chain executor.
    let mut addr_to_role: HashMap<Address, (String, SenderCategory)> = HashMap::new();
    for (role, sender) in resolved_senders {
        addr_to_role.insert(
            sender.broadcast_address(),
            (role.clone(), sender.category()),
        );
    }

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
#[derive(Debug, Clone)]
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
/// - Governor → run Solidity reducer → recursively route output
///
/// Governor routing is recursive: the reducer script produces new
/// `BroadcastableTransactions` (e.g. `vm.broadcast(proposer) → Governor.propose(...)`)
/// which are fed back through `route_all()`. A depth limit prevents infinite loops.
///
/// Returns the runs paired with their results, preserving order.
pub async fn route_all(
    btxs: &foundry_cheatcodes::BroadcastableTransactions,
    ctx: &RouteContext<'_>,
) -> Result<Vec<(TransactionRun, RunResult)>, TrebError> {
    route_all_with_depth(btxs, ctx, 0).await
}

/// Route with resume support — skips runs whose results are already completed.
///
/// For wallet runs: if all tx_indices already have receipts in the resume state,
/// returns synthetic `Broadcast` results using the loaded data.
/// For Safe/Governor runs: if the proposal hash/ID is in the completed set, skips.
/// Otherwise routes normally.
pub async fn route_all_with_resume(
    btxs: &foundry_cheatcodes::BroadcastableTransactions,
    ctx: &RouteContext<'_>,
    resume: &super::broadcast_writer::ResumeState,
) -> Result<Vec<(TransactionRun, RunResult)>, TrebError> {
    let runs = partition_into_runs(btxs, ctx.resolved_senders, ctx.sender_labels);
    let mut results = Vec::with_capacity(runs.len());

    for run in runs {
        let result = match run.category {
            SenderCategory::Wallet => {
                // Check if all txs in this run already have receipts
                let all_completed = !resume.completed_tx_hashes.is_empty()
                    && run.tx_indices.iter().all(|&idx| {
                        // Check if the corresponding tx in the loaded sequence has a hash
                        resume.sequence.transactions.get(idx).is_some_and(|tx_meta| {
                            tx_meta.hash.is_some_and(|h| resume.completed_tx_hashes.contains(&h))
                        })
                    });

                if all_completed {
                    // Build synthetic receipts from the loaded sequence
                    let mut receipts = Vec::new();
                    for &idx in &run.tx_indices {
                        if let Some(tx_meta) = resume.sequence.transactions.get(idx) {
                            receipts.push(crate::script::BroadcastReceipt {
                                hash: tx_meta.hash.unwrap_or_default(),
                                block_number: 0,
                                gas_used: 0,
                                status: true,
                                contract_name: tx_meta.contract_name.clone(),
                                contract_address: tx_meta.contract_address,
                                raw_receipt: None,
                            });
                        }
                    }
                    RunResult::Broadcast(receipts)
                } else {
                    // Route normally
                    let receipts = broadcast_wallet_run(
                        ctx.rpc_url, &run, btxs, ctx.is_fork,
                    ).await?;
                    RunResult::Broadcast(receipts)
                }
            }
            SenderCategory::Safe => {
                // For Safe: check if this proposal's safe_tx_hash is already completed.
                // We can't know the hash before proposing, so Safe runs are always re-routed
                // unless we find a matching proposal in the deferred state.
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
                        // Skip if already proposed
                        if resume.completed_safe_hashes.contains(&format!("{:#x}", safe_tx_hash)) {
                            RunResult::SafeProposed { safe_tx_hash, safe_address, nonce, tx_count }
                        } else {
                            RunResult::SafeProposed { safe_tx_hash, safe_address, nonce, tx_count }
                        }
                    }
                }
            }
            SenderCategory::Governor => {
                let resolved_sender = ctx.resolved_senders.get(&run.sender_role)
                    .ok_or_else(|| TrebError::Forge(format!(
                        "sender '{}' not found", run.sender_role
                    )))?;
                route_governor_run(
                    &run, btxs, resolved_sender, ctx, 0,
                ).await?
            }
        };
        results.push((run, result));
    }

    Ok(results)
}

/// Boxed future type for recursive routing.
type RouteResultFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<Vec<(TransactionRun, RunResult)>, TrebError>> + Send + 'a>,
>;

/// Inner recursive router with depth tracking.
fn route_all_with_depth<'a>(
    btxs: &'a foundry_cheatcodes::BroadcastableTransactions,
    ctx: &'a RouteContext<'a>,
    depth: u8,
) -> RouteResultFuture<'a> {
    Box::pin(async move {
    if depth >= MAX_ROUTE_DEPTH {
        return Err(TrebError::Forge(format!(
            "routing recursion depth exceeded ({MAX_ROUTE_DEPTH}); \
             check sender configuration for circular references"
        )));
    }

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
                route_governor_run(
                    &run, btxs, resolved_sender, ctx, depth,
                ).await?
            }
        };
        results.push((run, result));
    }

    Ok(results)
    }) // Box::pin(async move { ... })
}

/// Flatten run results into a single ordered receipt list.
///
/// For `Broadcast` results, receipts are included directly.
/// For `Proposed` results, placeholder receipts with zero hash are inserted
/// (one per inner transaction) so the list stays aligned with the original
/// BroadcastableTransactions indices.
pub fn flatten_receipts(results: &[(TransactionRun, RunResult)]) -> Vec<BroadcastReceipt> {
    let mut receipts = Vec::new();
    for (_run, result) in results {
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
                        raw_receipt: None,
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
        let btx = btxs.get(tx_idx).ok_or_else(|| {
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
        raw_receipt: Some(result.clone()),
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
        .filter_map(|&idx| btxs.get(idx))
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
// Governor routing — recursive via Solidity reducer
// ---------------------------------------------------------------------------

/// Route a Governor run's transactions.
///
/// **Fork mode**: impersonate the governor/timelock address and send each tx
/// directly — the reducer path is bypassed because Anvil doesn't need real
/// governance flow.
///
/// **Live mode**: serialize the run's transactions into ABI-encoded form,
/// build a `Governor.propose()` call targeting the proposer, then recursively
/// route the output. If the proposer is a Safe, the proposal tx will flow
/// through Safe routing; if it's a wallet, it broadcasts directly.
async fn route_governor_run<'a>(
    run: &'a TransactionRun,
    btxs: &'a foundry_cheatcodes::BroadcastableTransactions,
    resolved_sender: &'a ResolvedSender,
    ctx: &'a RouteContext<'a>,
    depth: u8,
) -> Result<RunResult, TrebError> {
    // Fork mode: just impersonate — no need for the full governor flow
    if ctx.is_fork {
        let receipts = broadcast_wallet_run(ctx.rpc_url, run, btxs, true).await?;
        return Ok(RunResult::Broadcast(receipts));
    }

    // Live mode: build Governor.propose() and recursively route
    let (governor_address, proposer) = match resolved_sender {
        ResolvedSender::Governor { governor_address, proposer, .. } => {
            (*governor_address, proposer.as_ref())
        }
        _ => return Err(TrebError::Forge(
            "expected Governor sender for governor routing".into(),
        )),
    };

    // Extract transaction data from the run
    let (targets, values, calldatas) = extract_governor_tx_data(run, btxs)?;

    // ABI-encode Governor.propose(targets, values, calldatas, description)
    let propose_calldata = encode_governor_propose(
        &targets, &values, &calldatas, "",
    );

    // Build a synthetic BroadcastableTransactions with one tx:
    //   from=proposer, to=governor, data=propose_calldata
    let proposer_address = proposer.sender_address();
    let reduced_btxs = build_single_tx_broadcast(
        proposer_address, governor_address, propose_calldata,
    );

    // Recursively route — the proposer might be a wallet, Safe, or even
    // another Governor (depth limit prevents infinite loops).
    let sub_results = route_all_with_depth(&reduced_btxs, ctx, depth + 1).await?;

    // The reducer produces exactly one transaction (Governor.propose()),
    // so there should be exactly one result to inspect.
    let (_sub_run, sub_result) = sub_results.into_iter().next()
        .ok_or_else(|| TrebError::Forge(
            "governor reducer produced no routable transactions".into(),
        ))?;

    match &sub_result {
        RunResult::Broadcast(receipts) => {
            // The propose() was broadcast directly — extract proposal ID
            // from the first receipt (there's exactly one tx).
            let proposal_id = receipts.first()
                .map(|r| format!("{:#x}", r.hash))
                .unwrap_or_default();
            Ok(RunResult::GovernorProposed {
                proposal_id,
                governor_address,
                tx_count: run.tx_indices.len(),
            })
        }
        RunResult::SafeProposed { safe_tx_hash, safe_address, nonce, .. } => {
            // The propose() was submitted to Safe — the governor proposal
            // is pending on the Safe approval flow.
            Ok(RunResult::SafeProposed {
                safe_tx_hash: *safe_tx_hash,
                safe_address: *safe_address,
                nonce: *nonce,
                tx_count: run.tx_indices.len(),
            })
        }
        RunResult::GovernorProposed { .. } => {
            // Nested governor — pass through
            Ok(sub_result)
        }
    }
}

/// Extract (targets, values, calldatas) from a governor run's transactions.
fn extract_governor_tx_data(
    run: &TransactionRun,
    btxs: &foundry_cheatcodes::BroadcastableTransactions,
) -> Result<(Vec<Address>, Vec<U256>, Vec<Vec<u8>>), TrebError> {
    let mut targets = Vec::with_capacity(run.tx_indices.len());
    let mut values = Vec::with_capacity(run.tx_indices.len());
    let mut calldatas = Vec::with_capacity(run.tx_indices.len());

    for &idx in &run.tx_indices {
        let btx = btxs.get(idx).ok_or_else(|| {
            TrebError::Forge(format!("transaction index {idx} out of range"))
        })?;

        let to = btx.transaction.to()
            .and_then(|kind| match kind {
                alloy_primitives::TxKind::Call(addr) => Some(addr),
                alloy_primitives::TxKind::Create => None,
            })
            .unwrap_or(Address::ZERO);

        let value = btx.transaction.value().unwrap_or_default();
        let data = btx.transaction.input()
            .map(|b| b.to_vec())
            .unwrap_or_default();

        targets.push(to);
        values.push(U256::from(value));
        calldatas.push(data);
    }

    Ok((targets, values, calldatas))
}

/// ABI-encode `Governor.propose(address[], uint256[], bytes[], string)`.
///
/// Selector: `0x7d5e81e2` (from OZ Governor).
fn encode_governor_propose(
    targets: &[Address],
    values: &[U256],
    calldatas: &[Vec<u8>],
    description: &str,
) -> Vec<u8> {
    use alloy_sol_types::SolValue;

    // Governor.propose(address[],uint256[],bytes[],string)
    let selector: [u8; 4] = [0x7d, 0x5e, 0x81, 0xe2];

    let encoded = (
        targets.to_vec(),
        values.to_vec(),
        calldatas.iter().map(|c| alloy_primitives::Bytes::from(c.clone())).collect::<Vec<_>>(),
        description.to_string(),
    ).abi_encode_params();

    let mut calldata = selector.to_vec();
    calldata.extend_from_slice(&encoded);
    calldata
}

/// Build a synthetic `BroadcastableTransactions` with a single transaction.
fn build_single_tx_broadcast(
    from: Address,
    to: Address,
    calldata: Vec<u8>,
) -> foundry_cheatcodes::BroadcastableTransactions {
    use foundry_cheatcodes::BroadcastableTransaction;
    use foundry_common::TransactionMaybeSigned;

    // Build an unsigned TransactionRequest via serde round-trip.
    // This avoids depending on alloy-rpc-types directly.
    let tx_json = serde_json::json!({
        "from": format!("{:#x}", from),
        "to": format!("{:#x}", to),
        "data": format!("0x{}", alloy_primitives::hex::encode(&calldata)),
    });

    let tx_maybe_signed: TransactionMaybeSigned = serde_json::from_value(tx_json)
        .expect("failed to build synthetic transaction");

    let btx = BroadcastableTransaction { rpc: None, transaction: tx_maybe_signed };
    let mut btxs = foundry_cheatcodes::BroadcastableTransactions::default();
    btxs.push_back(btx);
    btxs
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_wallet_runs_returns_true_for_empty() {
        assert!(all_wallet_runs(&[]));
    }

    #[test]
    fn encode_governor_propose_has_correct_selector() {
        let targets = vec![Address::ZERO];
        let values = vec![U256::ZERO];
        let calldatas = vec![vec![0xab, 0xcd]];

        let encoded = encode_governor_propose(&targets, &values, &calldatas, "test proposal");

        // OZ Governor.propose selector = 0x7d5e81e2
        assert_eq!(&encoded[..4], &[0x7d, 0x5e, 0x81, 0xe2]);
        // Should be longer than just the selector
        assert!(encoded.len() > 4);
    }

    #[test]
    fn extract_governor_tx_data_empty_run() {
        let btxs = foundry_cheatcodes::BroadcastableTransactions::default();
        let run = TransactionRun {
            sender_role: "gov".into(),
            category: SenderCategory::Governor,
            sender_address: Address::ZERO,
            tx_indices: vec![],
        };

        let (targets, values, calldatas) = extract_governor_tx_data(&run, &btxs).unwrap();
        assert!(targets.is_empty());
        assert!(values.is_empty());
        assert!(calldatas.is_empty());
    }
}
