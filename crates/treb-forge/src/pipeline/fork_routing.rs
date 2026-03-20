//! Production-faithful fork-mode routing for Safe and Governor senders.
//!
//! Instead of simply impersonating contract addresses, this module executes
//! transactions through the real on-chain contracts on an Anvil fork:
//!
//! - **Safe**: builds MultiSend bundles, queries owners/threshold, pre-approves
//!   hashes, and calls `execTransaction` — exercising the actual Safe contract.
//! - **Governor + Timelock**: schedules on the timelock, fast-forwards time,
//!   and executes — exercising access control and atomicity.
//!
//! This gives fork-mode the same `msg.sender` semantics as production.

use alloy_primitives::{Address, B256, Bytes, U256, keccak256};
use alloy_provider::Provider;
use alloy_rpc_types::{TransactionInput, TransactionRequest};
use alloy_sol_types::{SolCall, SolValue, sol};
use treb_core::error::TrebError;

use super::routing::TransactionRun;
use crate::script::BroadcastReceipt;

// ---------------------------------------------------------------------------
// ABI definitions (sol! macro)
// ---------------------------------------------------------------------------

sol! {
    // Safe
    function getOwners() external view returns (address[]);
    function getThreshold() external view returns (uint256);
    function approveHash(bytes32 hashToApprove) external;
    function execTransaction(
        address to,
        uint256 value,
        bytes data,
        uint8 operation,
        uint256 safeTxGas,
        uint256 baseGas,
        uint256 gasPrice,
        address gasToken,
        address refundReceiver,
        bytes signatures
    ) external returns (bool);
    function nonce() external view returns (uint256);

    // CreateCall — used to deploy contracts via Safe DelegateCall
    function performCreate(uint256 value, bytes deploymentData) external returns (address);

    // TimelockController
    function getMinDelay() external view returns (uint256);
    function scheduleBatch(
        address[] targets,
        uint256[] values,
        bytes[] payloads,
        bytes32 predecessor,
        bytes32 salt,
        uint256 delay
    ) external;
    function executeBatch(
        address[] targets,
        uint256[] values,
        bytes[] payloads,
        bytes32 predecessor,
        bytes32 salt
    ) external payable;

    // AccessControl (OZ)
    function grantRole(bytes32 role, address account) external;
}

/// `EXECUTOR_ROLE = keccak256("EXECUTOR_ROLE")`
/// Used by OZ TimelockController for access control on `executeBatch`.
const EXECUTOR_ROLE: B256 = B256::new([
    0xd8, 0xaa, 0x0f, 0x31, 0x94, 0x97, 0x1a, 0x2a,
    0x11, 0x66, 0x79, 0xf7, 0xc2, 0x09, 0x0f, 0x69,
    0x39, 0xc8, 0xd4, 0xe0, 0x1a, 0x2a, 0x8d, 0x7e,
    0x41, 0xd5, 0x5e, 0x53, 0x51, 0x46, 0x9e, 0x63,
]);

// ---------------------------------------------------------------------------
// Alloy provider helpers for Anvil-specific operations
// ---------------------------------------------------------------------------

/// Read-only `eth_call` returning raw bytes.
async fn eth_call_bytes(
    provider: &impl Provider,
    to: Address,
    data: &[u8],
    from: Option<Address>,
) -> Result<Vec<u8>, TrebError> {
    let mut tx = TransactionRequest::default().to(to);
    tx.input = TransactionInput::new(Bytes::from(data.to_vec()));
    if let Some(f) = from {
        tx = tx.from(f);
    }
    let result = provider
        .call(tx)
        .await
        .map_err(|e| TrebError::Forge(format!("eth_call failed: {e}")))?;
    Ok(result.to_vec())
}

/// Impersonate an account, send a transaction, wait for receipt, stop impersonating.
pub async fn impersonate_send_tx(
    provider: &impl Provider,
    from: Address,
    to: Address,
    data: &[u8],
    value: U256,
) -> Result<BroadcastReceipt, TrebError> {
    // Ensure the impersonated account has ETH for gas. Contract addresses
    // (Safe, Governor, Timelock, owners) typically hold zero ETH on forks.
    // 100 ETH = 100 * 10^18 = 0x56BC75E2D63100000
    anvil_set_balance(provider, from, U256::from(100_000_000_000_000_000_000u128)).await?;
    anvil_impersonate(provider, from).await?;

    let mut tx = TransactionRequest::default()
        .from(from)
        .to(to);
    tx.gas = Some(30_000_000);
    if !data.is_empty() {
        tx.input = TransactionInput::new(Bytes::from(data.to_vec()));
    }
    if !value.is_zero() {
        tx = tx.value(value);
    }

    let receipt = provider
        .send_transaction(tx)
        .await
        .map_err(|e| TrebError::Forge(format!("impersonated send tx failed: {e}")))?
        .get_receipt()
        .await
        .map_err(|e| TrebError::Forge(format!("get receipt failed: {e}")))?;

    anvil_stop_impersonating(provider, from).await;

    Ok(receipt_to_broadcast_receipt(&receipt))
}

/// Convert an alloy `TransactionReceipt` to a `BroadcastReceipt`.
///
/// Consistent with the live broadcast mapping in routing.rs.
pub(crate) fn receipt_to_broadcast_receipt(
    receipt: &alloy_rpc_types::TransactionReceipt,
) -> BroadcastReceipt {
    BroadcastReceipt {
        hash: receipt.transaction_hash,
        block_number: receipt.block_number.unwrap_or(0),
        gas_used: receipt.gas_used,
        status: receipt.inner.status(),
        contract_name: None,
        contract_address: receipt.contract_address,
        raw_receipt: serde_json::to_value(receipt).ok(),
    }
}

async fn anvil_set_balance(
    provider: &impl Provider,
    addr: Address,
    balance: U256,
) -> Result<(), TrebError> {
    provider
        .raw_request::<_, serde_json::Value>("anvil_setBalance".into(), (addr, balance))
        .await
        .map_err(|e| TrebError::Forge(format!("anvil_setBalance failed: {e}")))?;
    Ok(())
}

pub(crate) async fn anvil_impersonate(
    provider: &impl Provider,
    addr: Address,
) -> Result<(), TrebError> {
    provider
        .raw_request::<_, serde_json::Value>("anvil_impersonateAccount".into(), (addr,))
        .await
        .map_err(|e| TrebError::Forge(format!("anvil_impersonateAccount failed: {e}")))?;
    Ok(())
}

pub(crate) async fn anvil_stop_impersonating(provider: &impl Provider, addr: Address) {
    let _ = provider
        .raw_request::<_, serde_json::Value>(
            "anvil_stopImpersonatingAccount".into(),
            (addr,),
        )
        .await;
}

async fn anvil_set_code(
    provider: &impl Provider,
    addr: Address,
    code: &[u8],
) -> Result<(), TrebError> {
    let code_bytes = Bytes::from(code.to_vec());
    provider
        .raw_request::<_, serde_json::Value>("anvil_setCode".into(), (addr, code_bytes))
        .await
        .map_err(|e| TrebError::Forge(format!("anvil_setCode failed: {e}")))?;
    Ok(())
}

async fn evm_increase_time(
    provider: &impl Provider,
    seconds: u64,
) -> Result<(), TrebError> {
    provider
        .raw_request::<_, serde_json::Value>(
            "evm_increaseTime".into(),
            (U256::from(seconds),),
        )
        .await
        .map_err(|e| TrebError::Forge(format!("evm_increaseTime failed: {e}")))?;
    Ok(())
}

async fn anvil_mine(provider: &impl Provider, count: u64) -> Result<(), TrebError> {
    provider
        .raw_request::<_, serde_json::Value>("anvil_mine".into(), (U256::from(count),))
        .await
        .map_err(|e| TrebError::Forge(format!("anvil_mine failed: {e}")))?;
    Ok(())
}

async fn estimate_gas_for_call(
    provider: &impl Provider,
    from: Address,
    to: Address,
    data: &[u8],
) -> Result<u64, TrebError> {
    let mut tx = TransactionRequest::default().from(from).to(to);
    if !data.is_empty() {
        tx.input = TransactionInput::new(Bytes::from(data.to_vec()));
    }
    let estimate = provider
        .estimate_gas(tx)
        .await
        .map_err(|e| TrebError::Forge(format!("estimate_gas failed: {e}")))?;
    Ok(estimate as u64)
}

async fn block_gas_limit(provider: &impl Provider) -> Result<u64, TrebError> {
    // Use raw_request to avoid Block type complexities across alloy versions.
    let block: serde_json::Value = provider
        .raw_request("eth_getBlockByNumber".into(), ("latest", false))
        .await
        .map_err(|e| TrebError::Forge(format!("get block failed: {e}")))?;
    let hex = block
        .get("gasLimit")
        .and_then(|v| v.as_str())
        .unwrap_or("0x1c9c380");
    let stripped = hex.strip_prefix("0x").unwrap_or(hex);
    u64::from_str_radix(stripped, 16)
        .map_err(|e| TrebError::Forge(format!("hex parse gas limit: {e}")))
}

/// Truncate a checksummed or lowercased address to `0x1234...5678` form.
fn short_addr(addr: Address) -> String {
    let s = format!("{:#x}", addr);
    if s.len() >= 10 {
        format!("{}...{}", &s[..6], &s[s.len() - 4..])
    } else {
        s
    }
}

/// Print a progress line to stderr, clearing any active spinner first.
///
/// Writes `\x1b[2K\r` (erase line + carriage return) before the message
/// so a background spinner doesn't collide. The combined write is a single
/// `eprintln!` call, which locks stderr for the whole write.
macro_rules! progress {
    ($fmt:literal $(, $($arg:tt)*)?) => {
        eprintln!(concat!("\x1b[2K\r", $fmt) $(, $($arg)*)?)
    };
}

/// Deterministic address for the CreateCall helper deployed on fork.
const CREATE_CALL_ADDRESS: Address = Address::new([
    0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC,
    0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0x01,
]);

/// Runtime bytecode for CreateCall.sol compiled with solc 0.8.30.
/// Source: crates/treb-cli/tests/fixtures/project/src/safe/CreateCall.sol
const CREATE_CALL_BYTECODE: &str = "608060405234801561000f575f5ffd5b5060043610610029575f3560e01c80634c8c9ea11461002d575b5f5ffd5b6100476004803603810190610042919061029f565b61005d565b6040516100549190610338565b60405180910390f35b5f81516020830184f090505f73ffffffffffffffffffffffffffffffffffffffff168173ffffffffffffffffffffffffffffffffffffffff16036100d6576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016100cd906103ab565b60405180910390fd5b8073ffffffffffffffffffffffffffffffffffffffff167f4db17dd5e4732fb6da34a148104a592783ca119a1e7bb8829eba6cbadef0b51160405160405180910390a292915050565b5f604051905090565b5f5ffd5b5f5ffd5b5f819050919050565b61014281610130565b811461014c575f5ffd5b50565b5f8135905061015d81610139565b92915050565b5f5ffd5b5f5ffd5b5f601f19601f8301169050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52604160045260245ffd5b6101b18261016b565b810181811067ffffffffffffffff821117156101d0576101cf61017b565b5b80604052505050565b5f6101e261011f565b90506101ee82826101a8565b919050565b5f67ffffffffffffffff82111561020d5761020c61017b565b5b6102168261016b565b9050602081019050919050565b828183375f83830152505050565b5f61024361023e846101f3565b6101d9565b90508281526020810184848401111561025f5761025e610167565b5b61026a848285610223565b509392505050565b5f82601f83011261028657610285610163565b5b8135610296848260208601610231565b91505092915050565b5f5f604083850312156102b5576102b4610128565b5b5f6102c28582860161014f565b925050602083013567ffffffffffffffff8111156102e3576102e261012c565b5b6102ef85828601610272565b9150509250929050565b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f610322826102f9565b9050919050565b61033281610318565b82525050565b5f60208201905061034b5f830184610329565b92915050565b5f82825260208201905092915050565b7f436f756c64206e6f74206465706c6f7920636f6e7472616374000000000000005f82015250565b5f610395601983610351565b91506103a082610361565b602082019050919050565b5f6020820190508181035f8301526103c281610389565b905091905056";

// ---------------------------------------------------------------------------
// Safe fork execution
// ---------------------------------------------------------------------------

/// Execute a Safe run on an Anvil fork using production-faithful routing.
///
/// Builds MultiSend bundles, splits into batches by gas limit, then for each
/// batch: queries owners/threshold, pre-approves the hash from enough owners,
/// and calls `Safe.execTransaction`.
pub async fn execute_safe_on_fork(
    provider: &impl Provider,
    run: &TransactionRun,
    btxs: &foundry_cheatcodes::BroadcastableTransactions,
    safe_address: Address,
    chain_id: u64,
    quiet: bool,
) -> Result<Vec<BroadcastReceipt>, TrebError> {
    // Check if any transactions are CREATE (contract deployment).
    // Safe.execTransaction cannot directly deploy contracts — CREATE txs
    // must be routed through a CreateCall helper via DelegateCall.
    let has_creates = run.tx_indices.iter()
        .filter_map(|&idx| btxs.get(idx))
        .any(|btx| matches!(btx.transaction.to(), Some(alloy_primitives::TxKind::Create) | None));

    if has_creates {
        // Deploy CreateCall helper at a deterministic address
        let bytecode = alloy_primitives::hex::decode(CREATE_CALL_BYTECODE)
            .map_err(|e| TrebError::Forge(format!("invalid CreateCall bytecode: {e}")))?;
        anvil_set_code(provider, CREATE_CALL_ADDRESS, &bytecode).await?;
    }

    // Build MultiSend operations from the run's transactions.
    // CREATE transactions are wrapped as DelegateCall to CreateCall.performCreate().
    let operations: Vec<treb_safe::MultiSendOperation> = run.tx_indices.iter()
        .filter_map(|&idx| btxs.get(idx))
        .map(|btx| {
            let is_create = matches!(btx.transaction.to(), Some(alloy_primitives::TxKind::Create) | None);
            let value = btx.transaction.value().unwrap_or_default();
            let input = btx.transaction.input().cloned().unwrap_or_default();

            if is_create {
                // Wrap as DelegateCall to CreateCall.performCreate(value, bytecode)
                let calldata = performCreateCall {
                    value: U256::from(value),
                    deploymentData: input,
                }.abi_encode();
                treb_safe::MultiSendOperation {
                    operation: 1, // DelegateCall
                    to: CREATE_CALL_ADDRESS,
                    value: U256::ZERO,
                    data: calldata.into(),
                }
            } else {
                let to = btx.transaction.to()
                    .and_then(|kind| match kind {
                        alloy_primitives::TxKind::Call(addr) => Some(addr),
                        alloy_primitives::TxKind::Create => None,
                    })
                    .unwrap_or(Address::ZERO);
                treb_safe::MultiSendOperation {
                    operation: 0, // Call
                    to,
                    value: U256::from(value),
                    data: input,
                }
            }
        })
        .collect();

    if operations.is_empty() {
        return Ok(Vec::new());
    }

    // Split into gas-limited batches
    let batches = split_into_batches(provider, &operations, safe_address).await?;
    let batch_count = batches.len();
    let tx_count = operations.len();

    if !quiet {
        progress!(
            "Executing via Safe {} ({} tx{}, {} batch{})",
            short_addr(safe_address),
            tx_count,
            if tx_count == 1 { "" } else { "s" },
            batch_count,
            if batch_count == 1 { "" } else { "es" },
        );

        // Show bundling detail per batch
        for (i, batch) in batches.iter().enumerate() {
            if batch.len() == 1 {
                progress!(
                    "  batch {}/{}: direct Call to {}",
                    i + 1, batch_count, short_addr(batch[0].to),
                );
            } else {
                progress!(
                    "  batch {}/{}: MultiSend DelegateCall ({} ops)",
                    i + 1, batch_count, batch.len(),
                );
                for (j, op) in batch.iter().enumerate() {
                    progress!("    [{}/{}] Call {} ({} bytes)", j + 1, batch.len(), short_addr(op.to), op.data.len());
                }
            }
        }
    }

    let mut all_receipts = Vec::new();

    for (i, batch) in batches.iter().enumerate() {
        if !quiet && batch_count > 1 {
            progress!("  executing batch {}/{}...", i + 1, batch_count);
        }
        let receipt = execute_safe_batch(provider, batch, safe_address, chain_id).await?;
        if !receipt.status {
            return Err(TrebError::Forge(format!(
                "Safe.execTransaction reverted on fork (batch {}/{})", i + 1, batch_count,
            )));
        }
        all_receipts.push(receipt);
    }

    Ok(all_receipts)
}

/// Gas estimation + batch splitting (ported from v1 GnosisSafeSender.sol).
///
/// Estimates gas per sub-tx (impersonating the Safe), adds 20% buffer.
/// When cumulative gas exceeds 50% of block gas limit, starts a new batch.
const BATCH_OVERHEAD: u64 = 100_000;

async fn split_into_batches(
    provider: &impl Provider,
    operations: &[treb_safe::MultiSendOperation],
    safe_address: Address,
) -> Result<Vec<Vec<treb_safe::MultiSendOperation>>, TrebError> {
    let block_limit = block_gas_limit(provider).await.unwrap_or(30_000_000);
    let threshold = block_limit / 2;

    // Estimate gas for each operation
    let mut gas_estimates = Vec::with_capacity(operations.len());
    for op in operations {
        let estimate = estimate_gas_for_call(provider, safe_address, op.to, &op.data).await
            .unwrap_or(200_000); // fallback if estimation fails
        gas_estimates.push(estimate + estimate / 5); // +20% buffer
    }

    let mut batches = Vec::new();
    let mut current_batch = Vec::new();
    let mut current_gas: u64 = BATCH_OVERHEAD;

    for (i, op) in operations.iter().enumerate() {
        let gas = gas_estimates[i];
        if !current_batch.is_empty() && current_gas + gas > threshold {
            batches.push(current_batch);
            current_batch = Vec::new();
            current_gas = BATCH_OVERHEAD;
        }
        current_gas += gas;
        current_batch.push(op.clone());
    }

    if !current_batch.is_empty() {
        batches.push(current_batch);
    }

    Ok(batches)
}

/// Execute a single Safe batch via `execTransaction` with pre-approved hash signatures.
async fn execute_safe_batch(
    provider: &impl Provider,
    batch: &[treb_safe::MultiSendOperation],
    safe_address: Address,
    chain_id: u64,
) -> Result<BroadcastReceipt, TrebError> {
    // Build the execTransaction target + data + operation.
    // For single-op batches, use the operation's own type (Call or DelegateCall).
    let (to, data, operation) = if batch.len() == 1 {
        let op = &batch[0];
        (op.to, op.data.clone(), op.operation)
    } else {
        let multi_send_data = treb_safe::encode_multi_send_call(batch);
        (treb_safe::MULTI_SEND_ADDRESS, multi_send_data, 1u8)
    };

    // Query Safe nonce, owners, threshold
    let nonce_bytes = eth_call_bytes(provider, safe_address, &nonceCall {}.abi_encode(), None).await?;
    let nonce = U256::abi_decode(&nonce_bytes)
        .map_err(|e| TrebError::Forge(format!("decode nonce: {e}")))?;

    let owners_bytes = eth_call_bytes(provider, safe_address, &getOwnersCall {}.abi_encode(), None).await?;
    let owners = <Vec<Address>>::abi_decode(&owners_bytes)
        .map_err(|e| TrebError::Forge(format!("decode owners: {e}")))?;

    let threshold_bytes = eth_call_bytes(provider, safe_address, &getThresholdCall {}.abi_encode(), None).await?;
    let threshold_val = U256::abi_decode(&threshold_bytes)
        .map_err(|e| TrebError::Forge(format!("decode threshold: {e}")))?;
    let threshold: usize = threshold_val.try_into()
        .map_err(|_| TrebError::Forge("threshold too large".into()))?;

    // Compute safeTxHash (EIP-712)
    let safe_tx = treb_safe::SafeTx {
        to,
        value: U256::ZERO,
        data: data.to_vec().into(),
        operation,
        safeTxGas: U256::ZERO,
        baseGas: U256::ZERO,
        gasPrice: U256::ZERO,
        gasToken: Address::ZERO,
        refundReceiver: Address::ZERO,
        nonce,
    };
    let safe_tx_hash = treb_safe::compute_safe_tx_hash(chain_id, safe_address, &safe_tx);

    // Sort owners for deterministic signature ordering
    let mut sorted_owners = owners.clone();
    sorted_owners.sort();

    // Impersonate `threshold` owners and call approveHash
    let approvers: Vec<Address> = sorted_owners.iter().take(threshold).copied().collect();
    for owner in &approvers {
        let calldata = approveHashCall { hashToApprove: safe_tx_hash }.abi_encode();
        impersonate_send_tx(provider, *owner, safe_address, &calldata, U256::ZERO).await?;
    }

    // Build pre-approved signatures (v=1 means "approved via approveHash")
    let signatures = build_pre_approved_signatures(&approvers);

    // Call execTransaction from any account (the first owner is fine)
    let exec_calldata = execTransactionCall {
        to,
        value: U256::ZERO,
        data,
        operation,
        safeTxGas: U256::ZERO,
        baseGas: U256::ZERO,
        gasPrice: U256::ZERO,
        gasToken: Address::ZERO,
        refundReceiver: Address::ZERO,
        signatures: Bytes::from(signatures),
    }.abi_encode();

    let executor = approvers[0];
    impersonate_send_tx(provider, executor, safe_address, &exec_calldata, U256::ZERO).await
}

/// Build pre-approved signatures for Safe.execTransaction.
///
/// Each signature: r = left-padded owner address (32 bytes), s = 0 (32 bytes), v = 1.
/// Owners must be sorted ascending (Safe contract requirement).
fn build_pre_approved_signatures(owners: &[Address]) -> Vec<u8> {
    let mut sorted = owners.to_vec();
    sorted.sort();
    let mut sigs = Vec::with_capacity(sorted.len() * 65);
    for owner in &sorted {
        let mut r = [0u8; 32];
        r[12..32].copy_from_slice(owner.as_slice());
        sigs.extend_from_slice(&r);       // r = padded address
        sigs.extend_from_slice(&[0u8; 32]); // s = 0
        sigs.push(1);                      // v = 1 (pre-approved)
    }
    sigs
}

// ---------------------------------------------------------------------------
// Governor fork execution
// ---------------------------------------------------------------------------

/// Execute a Governor run on an Anvil fork.
///
/// Skips `Governor.propose()` (the proposer may lack governance tokens on
/// a fork) and goes straight to the timelock:
///
/// - **With timelock**: `scheduleBatch` → warp time → grant `EXECUTOR_ROLE`
///   → `executeBatch`. This exercises timelock access control and atomicity.
/// - **Without timelock**: impersonate the governor and send each tx directly.
pub async fn execute_governor_on_fork(
    provider: &impl Provider,
    run: &TransactionRun,
    btxs: &foundry_cheatcodes::BroadcastableTransactions,
    governor_address: Address,
    timelock_address: Option<Address>,
    quiet: bool,
) -> Result<Vec<BroadcastReceipt>, TrebError> {
    let tx_count = run.tx_indices.len();

    // Extract targets/values/calldatas from the run
    let mut targets = Vec::with_capacity(tx_count);
    let mut values = Vec::with_capacity(tx_count);
    let mut calldatas: Vec<Vec<u8>> = Vec::with_capacity(tx_count);

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
        let value = U256::from(btx.transaction.value().unwrap_or_default());
        let data = btx.transaction.input().map(|b| b.to_vec()).unwrap_or_default();
        targets.push(to);
        values.push(value);
        calldatas.push(data);
    }

    if targets.is_empty() {
        return Ok(Vec::new());
    }

    match timelock_address {
        Some(timelock) => {
            if !quiet {
                progress!(
                    "Executing via Governor {} ({} tx{})",
                    short_addr(governor_address), tx_count, if tx_count == 1 { "" } else { "s" },
                );
            }
            let receipt = schedule_and_execute_timelock(
                provider, &targets, &values, &calldatas,
                governor_address, timelock, quiet,
            ).await?;
            Ok(vec![receipt])
        }
        None => {
            if !quiet {
                progress!(
                    "Executing via Governor {} ({} tx{})",
                    short_addr(governor_address), tx_count, if tx_count == 1 { "" } else { "s" },
                );
                progress!("  Executing from governor (no timelock)...");
            }
            let mut receipts = Vec::new();
            for i in 0..targets.len() {
                let receipt = impersonate_send_tx(
                    provider, governor_address, targets[i], &calldatas[i], values[i],
                ).await?;
                if !receipt.status {
                    return Err(TrebError::Forge(format!(
                        "governor tx {} to {:#x} reverted on fork", i, targets[i],
                    )));
                }
                receipts.push(receipt);
            }
            Ok(receipts)
        }
    }
}

/// Schedule batch on timelock, grant EXECUTOR_ROLE, warp time, execute.
///
/// Impersonates the Governor (which has PROPOSER_ROLE on the Timelock)
/// to schedule. Then impersonates the timelock itself (which has
/// DEFAULT_ADMIN_ROLE) to grant EXECUTOR_ROLE to an executor account.
/// Finally fast-forwards past the delay and executes.
async fn schedule_and_execute_timelock(
    provider: &impl Provider,
    targets: &[Address],
    values: &[U256],
    calldatas: &[Vec<u8>],
    governor_address: Address,
    timelock_address: Address,
    quiet: bool,
) -> Result<BroadcastReceipt, TrebError> {
    let calldatas_bytes: Vec<Bytes> = calldatas.iter()
        .map(|c| Bytes::from(c.clone()))
        .collect();

    // description_hash = keccak256("") — matches Governor.propose("") behavior
    let description_hash = keccak256(b"");

    // 1. Query timelock delay
    let delay_bytes = eth_call_bytes(provider, timelock_address, &getMinDelayCall {}.abi_encode(), None).await?;
    let delay = U256::abi_decode(&delay_bytes)
        .map_err(|e| TrebError::Forge(format!("decode getMinDelay: {e}")))?;
    let delay_secs: u64 = delay.try_into().unwrap_or(0);

    if !quiet {
        progress!("  Scheduling on timelock {} (delay={}s)...", short_addr(timelock_address), delay_secs);
    }

    // 2. Schedule on Timelock (impersonate Governor, which has PROPOSER_ROLE)
    let schedule_calldata = scheduleBatchCall {
        targets: targets.to_vec(),
        values: values.to_vec(),
        payloads: calldatas_bytes.clone(),
        predecessor: B256::ZERO,
        salt: description_hash,
        delay,
    }.abi_encode();

    let schedule_receipt = impersonate_send_tx(
        provider, governor_address, timelock_address, &schedule_calldata, U256::ZERO,
    ).await?;
    if !schedule_receipt.status {
        return Err(TrebError::Forge("scheduleBatch reverted on fork".into()));
    }

    // 3. Grant EXECUTOR_ROLE to the governor so it can call executeBatch.
    //    Impersonate the timelock itself — it has DEFAULT_ADMIN_ROLE in OZ's
    //    default TimelockController setup (the timelock is its own admin).
    let grant_calldata = grantRoleCall {
        role: EXECUTOR_ROLE,
        account: governor_address,
    }.abi_encode();
    impersonate_send_tx(
        provider, timelock_address, timelock_address, &grant_calldata, U256::ZERO,
    ).await?;

    // 4. Warp time past the delay
    if delay_secs > 0 {
        if !quiet {
            progress!("  Fast-forwarding {}s...", delay_secs);
        }
        evm_increase_time(provider, delay_secs + 1).await?;
        anvil_mine(provider, 1).await?;
    }

    // 5. Execute from the governor (which now has EXECUTOR_ROLE)
    if !quiet {
        progress!("  Executing via timelock...");
    }
    let execute_calldata = executeBatchCall {
        targets: targets.to_vec(),
        values: values.to_vec(),
        payloads: calldatas_bytes,
        predecessor: B256::ZERO,
        salt: description_hash,
    }.abi_encode();

    let receipt = impersonate_send_tx(
        provider, governor_address, timelock_address, &execute_calldata, U256::ZERO,
    ).await?;

    if !receipt.status {
        return Err(TrebError::Forge("executeBatch reverted on fork".into()));
    }

    Ok(receipt)
}

// ---------------------------------------------------------------------------
// High-level fork exec helpers (for `treb fork exec`)
// ---------------------------------------------------------------------------

/// Execute a queued Safe transaction on a fork from registry data.
///
/// Takes the RPC URL, safe address string, chain ID, and the stored
/// `SafeTxData` list. Builds MultiSend operations and executes through
/// the real Safe contract.
pub async fn exec_safe_from_registry(
    rpc_url: &str,
    safe_address_str: &str,
    chain_id: u64,
    transactions: &[treb_core::types::safe_transaction::SafeTxData],
) -> Result<Vec<BroadcastReceipt>, TrebError> {
    let provider = crate::provider::build_http_provider(rpc_url)?;
    let safe_address: Address = safe_address_str.parse()
        .map_err(|e| TrebError::Forge(format!("invalid safe address: {e}")))?;

    let operations: Vec<treb_safe::MultiSendOperation> = transactions.iter()
        .map(|txd| {
            let data_hex = txd.data.strip_prefix("0x").unwrap_or(&txd.data);
            treb_safe::MultiSendOperation {
                operation: txd.operation,
                to: txd.to.parse().unwrap_or_default(),
                value: txd.value.parse().unwrap_or_default(),
                data: alloy_primitives::hex::decode(data_hex).unwrap_or_default().into(),
            }
        })
        .collect();

    if operations.is_empty() {
        return Ok(Vec::new());
    }

    // Build a synthetic run
    let run = super::routing::TransactionRun {
        sender_role: String::new(),
        category: crate::sender::SenderCategory::Safe,
        sender_address: safe_address,
        tx_indices: (0..operations.len()).collect(),
    };

    // Build synthetic broadcastable transactions
    let mut btxs = foundry_cheatcodes::BroadcastableTransactions::default();
    for op in &operations {
        let tx_json = serde_json::json!({
            "from": format!("{:#x}", safe_address),
            "to": format!("{:#x}", op.to),
            "data": format!("0x{}", alloy_primitives::hex::encode(&op.data)),
        });
        let tx: foundry_common::TransactionMaybeSigned =
            serde_json::from_value(tx_json)
                .map_err(|e| TrebError::Forge(format!("build synthetic tx: {e}")))?;
        btxs.push_back(foundry_cheatcodes::BroadcastableTransaction {
            rpc: None,
            transaction: tx,
        });
    }

    execute_safe_on_fork(&provider, &run, &btxs, safe_address, chain_id, true).await
}

/// Execute a queued governance proposal on a fork from registry data.
///
/// Uses simplified simulation: impersonates the governance address
/// and sends each action directly.
pub async fn exec_governance_from_registry(
    rpc_url: &str,
    governor_address_str: &str,
    timelock_address_str: &str,
    actions: &[treb_core::types::GovernorAction],
) -> Result<Vec<BroadcastReceipt>, TrebError> {
    let provider = crate::provider::build_http_provider(rpc_url)?;

    let governance_addr: Address = if !timelock_address_str.is_empty() {
        timelock_address_str.parse()
    } else {
        governor_address_str.parse()
    }
    .map_err(|e| TrebError::Forge(format!("invalid governance address: {e}")))?;

    let targets: Vec<Address> = actions.iter()
        .map(|a| a.target.parse().unwrap_or_default())
        .collect();
    let values: Vec<U256> = actions.iter()
        .map(|a| a.value.parse().unwrap_or_default())
        .collect();
    let calldatas: Vec<Vec<u8>> = actions.iter()
        .map(|a| {
            let hex = a.calldata.strip_prefix("0x").unwrap_or(&a.calldata);
            alloy_primitives::hex::decode(hex).unwrap_or_default()
        })
        .collect();

    if targets.is_empty() {
        return Ok(Vec::new());
    }

    simulate_governance_on_fork(&provider, &targets, &values, &calldatas, governance_addr).await
}

// ---------------------------------------------------------------------------
// Threshold / nonce queries
// ---------------------------------------------------------------------------

/// Query the Safe contract's threshold on a fork.
pub async fn query_safe_threshold(provider: &impl Provider, safe: Address) -> Result<u64, TrebError> {
    let threshold_bytes = eth_call_bytes(provider, safe, &getThresholdCall {}.abi_encode(), None).await?;
    let val = U256::abi_decode(&threshold_bytes)
        .map_err(|e| TrebError::Forge(format!("decode threshold: {e}")))?;
    val.try_into().map_err(|_| TrebError::Forge("threshold too large".into()))
}

/// Query the Safe contract's nonce on a fork.
pub async fn query_safe_nonce(provider: &impl Provider, safe: Address) -> Result<u64, TrebError> {
    let nonce_bytes = eth_call_bytes(provider, safe, &nonceCall {}.abi_encode(), None).await?;
    let val = U256::abi_decode(&nonce_bytes)
        .map_err(|e| TrebError::Forge(format!("decode nonce: {e}")))?;
    val.try_into().map_err(|_| TrebError::Forge("nonce too large".into()))
}

// ---------------------------------------------------------------------------
// Simplified governance simulation
// ---------------------------------------------------------------------------

/// Simulate governance execution on a fork by impersonating the governance address.
///
/// Instead of the full timelock schedule+warp+execute dance, this simply
/// impersonates the governance address (timelock if present, else governor)
/// and sends each transaction directly.
pub async fn simulate_governance_on_fork(
    provider: &impl Provider,
    targets: &[Address],
    values: &[U256],
    calldatas: &[Vec<u8>],
    governance_address: Address,
) -> Result<Vec<BroadcastReceipt>, TrebError> {
    let mut receipts = Vec::new();
    for i in 0..targets.len() {
        let receipt = impersonate_send_tx(
            provider, governance_address, targets[i], &calldatas[i], values[i],
        ).await?;
        if !receipt.status {
            return Err(TrebError::Forge(format!(
                "governance simulation tx {} to {:#x} reverted on fork", i, targets[i],
            )));
        }
        receipts.push(receipt);
    }
    Ok(receipts)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_approved_signatures_correct_format() {
        let owners = vec![
            "0x0000000000000000000000000000000000000002".parse::<Address>().unwrap(),
            "0x0000000000000000000000000000000000000001".parse::<Address>().unwrap(),
        ];

        let sigs = build_pre_approved_signatures(&owners);

        // Two owners → 2 * 65 = 130 bytes
        assert_eq!(sigs.len(), 130);

        // Should be sorted: 0x01 before 0x02
        // First sig: r = left-padded 0x01, s = 0, v = 1
        assert_eq!(&sigs[12..32], owners[1].as_slice()); // 0x01 (sorted first)
        assert_eq!(sigs[64], 1); // v = 1

        // Second sig: r = left-padded 0x02
        assert_eq!(&sigs[65 + 12..65 + 32], owners[0].as_slice()); // 0x02 (sorted second)
        assert_eq!(sigs[65 + 64], 1); // v = 1
    }

    #[test]
    fn pre_approved_signatures_single_owner() {
        let addr: Address = "0x1234567890abcdef1234567890abcdef12345678".parse().unwrap();
        let sigs = build_pre_approved_signatures(&[addr]);

        assert_eq!(sigs.len(), 65);
        // r: first 12 bytes zero, then 20 bytes of address
        assert_eq!(&sigs[0..12], &[0u8; 12]);
        assert_eq!(&sigs[12..32], addr.as_slice());
        // s: 32 zero bytes
        assert_eq!(&sigs[32..64], &[0u8; 32]);
        // v: 1
        assert_eq!(sigs[64], 1);
    }

    #[test]
    fn pre_approved_signatures_empty() {
        let sigs = build_pre_approved_signatures(&[]);
        assert!(sigs.is_empty());
    }

    #[test]
    fn executor_role_matches_keccak256() {
        let expected = keccak256(b"EXECUTOR_ROLE");
        assert_eq!(EXECUTOR_ROLE, expected);
    }
}
