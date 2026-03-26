//! Transaction routing — queue-reduction model.
//!
//! After script execution, forge captures `BroadcastableTransactions` with a
//! `from` address on each tx. This module partitions them into consecutive
//! "runs" grouped by sender, then **reduces** each run into a flat list of
//! simple actions:
//!
//! - **`RoutingAction::Exec`** — send tx(s) on-chain (wallet broadcast, Safe `execTransaction` for
//!   1/1, or Governor `propose()` call)
//! - **`RoutingAction::Propose`** — propose to Safe Transaction Service (multi-sig only, noop on
//!   fork)
//!
//! Both action types may emit **`QueuedExecution`** items that represent
//! queued operations (Safe multi-sig approval, governance execution).
//!
//! The reduction is iterative (no recursion, no `Box::pin` futures). Governor
//! routing pushes the `propose()` tx back onto the work queue, resolved through
//! the proposer, with a depth limit to prevent infinite loops.

use std::collections::HashMap;

use alloy_primitives::{Address, B256, U256};
use alloy_provider::Provider;
use treb_core::error::TrebError;

use forge_script_sequence::ScriptSequence;

use alloy_network::Ethereum;

use crate::{
    script::BroadcastReceipt,
    sender::{ResolvedSender, SenderCategory},
};

use super::types::RecordedTransaction;

/// Maximum queue depth for routing reducer chains.
///
/// Prevents infinite loops from misconfigured sender chains (e.g. Governor
/// whose proposer is another Governor whose proposer is the first Governor).
const MAX_ROUTE_DEPTH: u8 = 4;

// ---------------------------------------------------------------------------
// RoutingAction / QueuedExecution / RoutingPlan
// ---------------------------------------------------------------------------

/// A routable transaction — calldata targeting a specific address.
#[derive(Debug, Clone)]
pub struct RoutableTx {
    pub to: Address,
    pub value: U256,
    pub data: Vec<u8>,
}

/// Metadata for an `Exec` that wraps a Safe `execTransaction`.
#[derive(Debug, Clone)]
pub struct SafeContext {
    pub safe_address: Address,
    pub nonce: u64,
    pub safe_tx_hash: B256,
    pub threshold: u64,
}

/// Metadata for an `Exec` or `Propose` that relates to governance.
#[derive(Debug, Clone)]
pub struct GovernanceContext {
    pub governor_address: Address,
    pub timelock_address: Option<Address>,
    pub proposal_description: String,
}

/// A reduced routing action — either execute directly or propose off-chain.
#[derive(Debug)]
pub enum RoutingAction {
    /// Send transaction(s) on-chain: wallet broadcast, Safe execTx (1/1),
    /// or a Governor propose() call.
    Exec {
        /// The `from` address (impersonated on fork, signed on live).
        from: Address,
        /// The transactions to execute in sequence.
        transactions: Vec<RoutableTx>,
        /// If this exec wraps a Safe execTransaction.
        safe: Option<SafeContext>,
        /// If this exec relates to a governance proposal.
        governance: Option<GovernanceContext>,
    },
    /// Propose to the Safe Transaction Service (multi-sig, threshold > 1).
    /// On fork this is a noop (just record). On live, sign + POST.
    Propose {
        safe_address: Address,
        chain_id: u64,
        /// The MultiSend operations for the proposal.
        operations: Vec<treb_safe::MultiSendOperation>,
        /// The original user transactions covered by this proposal.
        inner_transactions: Vec<RoutableTx>,
        sender_role: String,
        nonce: u64,
        /// If the proposal wraps a governance propose() call.
        governance: Option<GovernanceContext>,
    },
}

/// A queued execution that results from a routing action.
///
/// Processed inline by the executor after the corresponding action.
#[derive(Debug, Clone)]
pub enum QueuedExecution {
    /// A Safe multi-sig proposal awaiting approval/execution.
    SafeProposal {
        safe_address: Address,
        safe_tx_hash: B256,
        nonce: u64,
        inner_txs: Vec<RoutableTx>,
    },
    /// A governance proposal awaiting on-chain execution.
    GovernanceProposal {
        governor_address: Address,
        timelock_address: Option<Address>,
        actions: Vec<GovernorAction>,
        proposal_description: String,
    },
}

/// A single governance action (target + value + calldata).
#[derive(Debug, Clone)]
pub struct GovernorAction {
    pub target: Address,
    pub value: U256,
    pub calldata: Vec<u8>,
}

/// The complete routing plan — a flat list of (action, optional queued) pairs
/// paired with the original run metadata.
pub struct RoutingPlan {
    pub actions: Vec<PlannedAction>,
}

/// A single entry in the routing plan.
pub struct PlannedAction {
    /// The original transaction run this action derives from.
    pub run: TransactionRun,
    /// The routing action to execute.
    pub action: RoutingAction,
    /// Optional queued execution to handle after the action.
    pub queued: Option<QueuedExecution>,
}

// ---------------------------------------------------------------------------
// TransactionRun — partitioning
// ---------------------------------------------------------------------------

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
    btxs: &foundry_cheatcodes::BroadcastableTransactions<Ethereum>,
    resolved_senders: &HashMap<String, ResolvedSender>,
    sender_labels: &HashMap<Address, String>,
) -> Vec<TransactionRun> {
    // Build address → (role, category) lookup. For Governor senders with a
    // timelock, register the timelock address (not the governor) because the
    // user script `vm.broadcast()`s from the timelock — the on-chain executor.
    let mut addr_to_role: HashMap<Address, (String, SenderCategory)> = HashMap::new();
    for (role, sender) in resolved_senders {
        addr_to_role.insert(sender.broadcast_address(), (role.clone(), sender.category()));
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
        let (role, category) = addr_to_role.get(&from).cloned().unwrap_or_else(|| {
            let label = sender_labels.get(&from).cloned().unwrap_or_else(|| format!("{:#x}", from));
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
// Result types (kept for backward compat with orchestrator / CLI)
// ---------------------------------------------------------------------------

/// Result of routing a single transaction run.
#[derive(Debug, Clone)]
pub enum RunResult {
    /// All txs were broadcast on-chain and confirmed.
    Broadcast(Vec<BroadcastReceipt>),
    /// Txs were proposed to the Safe Transaction Service (live mode).
    SafeProposed { safe_tx_hash: B256, safe_address: Address, nonce: u64, tx_count: usize },
    /// Txs were submitted as a Governor proposal (live mode).
    GovernorProposed { proposal_id: String, governor_address: Address, tx_count: usize },
}

/// Final routing execution output.
pub struct ExecutionBundle {
    /// Logical per-run results in original script order.
    pub run_results: Vec<(TransactionRun, RunResult)>,
    /// Queueable items that ended up queued for follow-up or fork simulation.
    pub queued_executions: Vec<QueuedExecution>,
}

/// Callback invoked after each top-level routing run completes.
pub type OnRunComplete = dyn Fn(&TransactionRun, &RunResult) + Send + Sync;

/// Context needed for transaction routing.
pub struct RouteContext<'a> {
    pub rpc_url: &'a str,
    pub chain_id: u64,
    pub is_fork: bool,
    /// Suppress progress output (e.g. when `--json` is active).
    pub quiet: bool,
    /// Optional callback fired after each action completes.
    pub on_run_complete: Option<&'a OnRunComplete>,
    pub resolved_senders: &'a HashMap<String, ResolvedSender>,
    pub sender_labels: &'a HashMap<Address, String>,
    pub sender_configs: &'a HashMap<String, treb_config::SenderConfig>,
    /// Optional mutable reference to a pre-built ScriptSequence for in-place
    /// checkpoint updates during broadcast.
    pub sequence: Option<&'a mut ScriptSequence<Ethereum>>,
    /// Tracks nonce offsets per Safe address across multiple `reduce_queue` calls.
    /// Each proposal increments the offset so sequential proposals get unique nonces.
    pub safe_nonce_offsets: HashMap<Address, u64>,
    /// When true, `execute_single_action` returns `SafeProposed` without submitting
    /// to the Safe Transaction Service (live mode acts like fork mode for proposals).
    /// Used by compose's merge flow to defer submission until proposals are merged.
    pub defer_safe_proposals: bool,
    /// Cache of Safe thresholds to avoid re-querying the same Safe across components.
    pub safe_threshold_cache: HashMap<Address, u64>,
}

// ---------------------------------------------------------------------------
// reduce_queue — iterative classification/reduction
// ---------------------------------------------------------------------------

/// Item in the reduction work queue.
struct ReductionItem {
    run: TransactionRun,
    /// The broadcastable transactions (either original or synthetic for governor propose).
    btxs: foundry_cheatcodes::BroadcastableTransactions<Ethereum>,
    /// Governance context inherited from a parent governor reduction.
    governance: Option<GovernanceContext>,
    depth: u8,
}

/// Reduce all transaction runs into a flat `RoutingPlan`.
///
/// This is the classification step — no RPC calls for wallet/governor runs,
/// but Safe runs query threshold/nonce to determine 1/1 vs multi-sig.
pub async fn reduce_queue(
    btxs: &foundry_cheatcodes::BroadcastableTransactions<Ethereum>,
    ctx: &mut RouteContext<'_>,
) -> Result<RoutingPlan, TrebError> {
    let runs = partition_into_runs(btxs, ctx.resolved_senders, ctx.sender_labels);
    let mut queue = std::collections::VecDeque::new();

    for run in runs {
        queue.push_back(ReductionItem { run, btxs: btxs.clone(), governance: None, depth: 0 });
    }

    let mut actions = Vec::new();

    while let Some(item) = queue.pop_front() {
        if item.depth >= MAX_ROUTE_DEPTH {
            return Err(TrebError::Forge(format!(
                "routing queue depth exceeded ({MAX_ROUTE_DEPTH}); \
                 check sender configuration for circular references"
            )));
        }

        match item.run.category {
            SenderCategory::Wallet => {
                let txs = extract_routable_txs(&item.run, &item.btxs)?;
                let from = item.run.sender_address;
                actions.push(PlannedAction {
                    run: item.run,
                    action: RoutingAction::Exec {
                        from,
                        transactions: txs,
                        safe: None,
                        governance: item.governance,
                    },
                    queued: None,
                });
            }
            SenderCategory::Safe => {
                let resolved =
                    ctx.resolved_senders.get(&item.run.sender_role).ok_or_else(|| {
                        TrebError::Forge(format!("sender '{}' not found", item.run.sender_role))
                    })?;
                let safe_address = match resolved {
                    ResolvedSender::Safe { safe_address, .. } => *safe_address,
                    _ => return Err(TrebError::Safe("expected Safe sender".into())),
                };

                let threshold = if let Some(&cached) = ctx.safe_threshold_cache.get(&safe_address) {
                    cached
                } else if ctx.is_fork {
                    let provider = crate::provider::build_http_provider(ctx.rpc_url)?;
                    let t =
                        super::fork_routing::query_safe_threshold(&provider, safe_address).await?;
                    ctx.safe_threshold_cache.insert(safe_address, t);
                    t
                } else {
                    let safe_client =
                        treb_safe::SafeServiceClient::new(ctx.chain_id).ok_or_else(|| {
                            TrebError::Safe(format!(
                                "Safe Transaction Service not available for chain {}",
                                ctx.chain_id
                            ))
                        })?;
                    let info = safe_client.get_safe_info(&format!("{}", safe_address)).await?;
                    ctx.safe_threshold_cache.insert(safe_address, info.threshold);
                    info.threshold
                };

                let inner_txs = extract_routable_txs(&item.run, &item.btxs)?;

                if threshold <= 1 {
                    // Safe(1/1) — reduce to direct execution
                    let planned = reduce_safe_1of1(
                        &item.run,
                        &item.btxs,
                        resolved,
                        safe_address,
                        ctx,
                        item.governance.clone(),
                    )
                    .await?;
                    actions.push(planned);
                } else {
                    // Safe(n/m) — reduce to proposal
                    let operations = build_multisend_operations(&item.run, &item.btxs)?;
                    // When deferring proposals (compose merge), skip the API call —
                    // Phase C assigns real nonces. Use placeholder 0 + offset.
                    let base_nonce = if ctx.defer_safe_proposals {
                        0
                    } else if ctx.is_fork {
                        let provider = crate::provider::build_http_provider(ctx.rpc_url)?;
                        super::fork_routing::query_safe_nonce(&provider, safe_address).await?
                    } else {
                        let safe_client = treb_safe::SafeServiceClient::new(ctx.chain_id)
                            .ok_or_else(|| {
                                TrebError::Safe(format!(
                                    "Safe Transaction Service not available for chain {}",
                                    ctx.chain_id
                                ))
                            })?;
                        safe_client.get_next_nonce(&format!("{}", safe_address)).await?
                    };
                    let offset = ctx.safe_nonce_offsets.entry(safe_address).or_insert(0);
                    let nonce = base_nonce + *offset;
                    *offset += 1;

                    let safe_tx_hash = compute_safe_tx_hash_for_ops(
                        &operations,
                        safe_address,
                        nonce,
                        ctx.chain_id,
                    );

                    let sender_role = item.run.sender_role.clone();
                    actions.push(PlannedAction {
                        run: item.run,
                        action: RoutingAction::Propose {
                            safe_address,
                            chain_id: ctx.chain_id,
                            operations,
                            inner_transactions: inner_txs.clone(),
                            sender_role,
                            nonce,
                            governance: item.governance.clone(),
                        },
                        queued: Some(QueuedExecution::SafeProposal {
                            safe_address,
                            safe_tx_hash,
                            nonce,
                            inner_txs,
                        }),
                    });
                }
            }
            SenderCategory::Governor => {
                let resolved =
                    ctx.resolved_senders.get(&item.run.sender_role).ok_or_else(|| {
                        TrebError::Forge(format!("sender '{}' not found", item.run.sender_role))
                    })?;

                let (governor_address, timelock_address, proposer, reducer_field) = match resolved {
                    ResolvedSender::Governor {
                        governor_address,
                        timelock_address,
                        proposer,
                        reducer,
                    } => (*governor_address, *timelock_address, proposer.as_ref(), reducer.clone()),
                    _ => {
                        return Err(TrebError::Forge(
                            "expected Governor sender for governor routing".into(),
                        ));
                    }
                };

                // Extract transaction data for the proposal
                let (targets, values, calldatas) = extract_governor_tx_data(&item.run, &item.btxs)?;

                let gov_actions: Vec<GovernorAction> = targets
                    .iter()
                    .zip(values.iter())
                    .zip(calldatas.iter())
                    .map(|((t, v), c)| GovernorAction {
                        target: *t,
                        value: *v,
                        calldata: c.clone(),
                    })
                    .collect();

                let gov_ctx = GovernanceContext {
                    governor_address,
                    timelock_address,
                    proposal_description: String::new(),
                };

                let queued = QueuedExecution::GovernanceProposal {
                    governor_address,
                    timelock_address,
                    actions: gov_actions,
                    proposal_description: String::new(),
                };

                let proposer_address = proposer.sender_address();

                // Try reducer script, fall back to inline Rust encoding
                let project_root = std::env::current_dir().unwrap_or_default();
                let reducer_script =
                    super::reducer::resolve_reducer_path(reducer_field.as_deref(), &project_root);

                let reduced_btxs = if let Some(reducer_path) = reducer_script {
                    let env_vars = super::reducer::build_reducer_env_vars(
                        governor_address,
                        proposer_address,
                        &targets,
                        &values,
                        &calldatas,
                        "", // description
                        timelock_address,
                        ctx.chain_id,
                    );
                    super::reducer::invoke_reducer(
                        &reducer_path,
                        env_vars,
                        ctx.rpc_url,
                        ctx.chain_id,
                    )
                    .await?
                } else {
                    // Fallback: inline Rust encoding (backward compat)
                    let propose_calldata =
                        encode_governor_propose(&targets, &values, &calldatas, "");
                    build_single_tx_broadcast(proposer_address, governor_address, propose_calldata)
                };

                // Determine the proposer's category and push back onto queue
                let proposer_category = proposer.category();
                let proposer_role = find_proposer_role(
                    &item.run.sender_role,
                    ctx.resolved_senders,
                    ctx.sender_configs,
                );

                let proposer_run = TransactionRun {
                    sender_role: proposer_role,
                    category: proposer_category,
                    sender_address: proposer_address,
                    tx_indices: vec![0],
                };

                // Push the propose() tx as a new item on the queue
                queue.push_back(ReductionItem {
                    run: TransactionRun {
                        // Preserve the original run metadata for the governor
                        sender_role: item.run.sender_role.clone(),
                        category: item.run.category,
                        sender_address: item.run.sender_address,
                        tx_indices: item.run.tx_indices.clone(),
                    },
                    btxs: item.btxs.clone(),
                    governance: Some(gov_ctx),
                    depth: item.depth, // depth of the _original_ governor run
                });

                // But actually, we need to reduce the propose() tx through
                // the proposer — not re-reduce the original run.
                // Remove the item we just pushed and instead do it inline.
                queue.pop_back();

                // Instead: reduce the proposer run inline with the propose() btxs
                let proposer_item = ReductionItem {
                    run: proposer_run,
                    btxs: reduced_btxs,
                    governance: Some(GovernanceContext {
                        governor_address,
                        timelock_address,
                        proposal_description: String::new(),
                    }),
                    depth: item.depth + 1,
                };

                // We need to reduce this item. Since the proposer could be
                // Wallet, Safe(1/1), Safe(n/m), or another Governor, push it
                // back onto the front of the queue. The result will carry the
                // governance context and the queued GovernanceProposal.
                //
                // However, we need to attach the QueuedExecution::GovernanceProposal
                // to whatever action the proposer produces. We do this by
                // letting the item reduce naturally — the governance context
                // propagates. Then after reduction, we attach the queued item
                // to the last action that was produced.
                let before_len = actions.len();
                queue.push_front(proposer_item);

                // Process just this one item (it's at the front)
                let front = queue.pop_front().unwrap();

                if front.depth >= MAX_ROUTE_DEPTH {
                    return Err(TrebError::Forge(format!(
                        "routing queue depth exceeded ({MAX_ROUTE_DEPTH}); \
                         check sender configuration for circular references"
                    )));
                }

                match front.run.category {
                    SenderCategory::Wallet => {
                        let txs = extract_routable_txs(&front.run, &front.btxs)?;
                        actions.push(PlannedAction {
                            run: item.run,
                            action: RoutingAction::Exec {
                                from: front.run.sender_address,
                                transactions: txs,
                                safe: None,
                                governance: front.governance,
                            },
                            queued: Some(queued),
                        });
                    }
                    SenderCategory::Safe => {
                        let proposer_resolved =
                            ctx.resolved_senders.get(&front.run.sender_role).ok_or_else(|| {
                                TrebError::Forge(format!(
                                    "sender '{}' not found",
                                    front.run.sender_role
                                ))
                            })?;
                        let proposer_safe = match proposer_resolved {
                            ResolvedSender::Safe { safe_address, .. } => *safe_address,
                            _ => {
                                return Err(TrebError::Safe(
                                    "expected Safe sender for proposer".into(),
                                ));
                            }
                        };

                        let proposer_threshold = if let Some(&cached) =
                            ctx.safe_threshold_cache.get(&proposer_safe)
                        {
                            cached
                        } else if ctx.is_fork {
                            let provider = crate::provider::build_http_provider(ctx.rpc_url)?;
                            let t =
                                super::fork_routing::query_safe_threshold(&provider, proposer_safe)
                                    .await?;
                            ctx.safe_threshold_cache.insert(proposer_safe, t);
                            t
                        } else {
                            let safe_client = treb_safe::SafeServiceClient::new(ctx.chain_id)
                                .ok_or_else(|| {
                                    TrebError::Safe(format!(
                                        "Safe Transaction Service not available for chain {}",
                                        ctx.chain_id
                                    ))
                                })?;
                            let info =
                                safe_client.get_safe_info(&format!("{}", proposer_safe)).await?;
                            ctx.safe_threshold_cache.insert(proposer_safe, info.threshold);
                            info.threshold
                        };

                        if proposer_threshold <= 1 {
                            let planned = reduce_safe_1of1(
                                &front.run,
                                &front.btxs,
                                proposer_resolved,
                                proposer_safe,
                                ctx,
                                front.governance.clone(),
                            )
                            .await?;
                            // Attach governance queued to the safe exec
                            actions.push(PlannedAction {
                                run: item.run,
                                action: planned.action,
                                queued: Some(queued),
                            });
                        } else {
                            let ops = build_multisend_operations(&front.run, &front.btxs)?;
                            let inner = extract_routable_txs(&front.run, &front.btxs)?;
                            let base_nonce = if ctx.defer_safe_proposals {
                                0
                            } else if ctx.is_fork {
                                let provider = crate::provider::build_http_provider(ctx.rpc_url)?;
                                super::fork_routing::query_safe_nonce(&provider, proposer_safe)
                                    .await?
                            } else {
                                let safe_client = treb_safe::SafeServiceClient::new(ctx.chain_id)
                                    .ok_or_else(|| {
                                    TrebError::Safe(format!(
                                        "Safe Transaction Service not available for chain {}",
                                        ctx.chain_id
                                    ))
                                })?;
                                safe_client.get_next_nonce(&format!("{}", proposer_safe)).await?
                            };
                            let offset = ctx.safe_nonce_offsets.entry(proposer_safe).or_insert(0);
                            let nonce = base_nonce + *offset;
                            *offset += 1;
                            // Propose wrapping the governance context
                            actions.push(PlannedAction {
                                run: item.run,
                                action: RoutingAction::Propose {
                                    safe_address: proposer_safe,
                                    chain_id: ctx.chain_id,
                                    operations: ops,
                                    inner_transactions: inner,
                                    sender_role: front.run.sender_role.clone(),
                                    nonce,
                                    governance: front.governance,
                                },
                                queued: Some(queued),
                            });
                        }
                    }
                    SenderCategory::Governor => {
                        // Governor → Governor: push back with increased depth
                        queue.push_front(ReductionItem {
                            run: front.run,
                            btxs: front.btxs,
                            governance: front.governance,
                            depth: front.depth + 1,
                        });
                        // We still need to store the original run's governance queued
                        // item. It will be attached when the inner governor reduces
                        // to a terminal action (Wallet or Safe).
                        // For now, continue the loop — the recursive governor will
                        // eventually reduce to a wallet or safe.
                        continue;
                    }
                }

                let _ = before_len; // suppress unused warning
            }
        }
    }

    Ok(RoutingPlan { actions })
}

/// Find the proposer's role name from sender configs.
fn find_proposer_role(
    governor_role: &str,
    resolved_senders: &HashMap<String, ResolvedSender>,
    sender_configs: &HashMap<String, treb_config::SenderConfig>,
) -> String {
    // First try the config's proposer field
    if let Some(config) = sender_configs.get(governor_role) {
        if let Some(proposer_name) = &config.proposer {
            return proposer_name.clone();
        }
    }
    // Fall back to finding the proposer in resolved senders
    if let Some(ResolvedSender::Governor { proposer, .. }) = resolved_senders.get(governor_role) {
        let proposer_addr = proposer.sender_address();
        for (role, sender) in resolved_senders {
            if sender.sender_address() == proposer_addr && role != governor_role {
                return role.clone();
            }
        }
    }
    governor_role.to_string()
}

/// Reduce a Safe(1/1) run into an Exec action with full execTransaction calldata.
async fn reduce_safe_1of1(
    run: &TransactionRun,
    btxs: &foundry_cheatcodes::BroadcastableTransactions<Ethereum>,
    resolved_sender: &ResolvedSender,
    safe_address: Address,
    ctx: &RouteContext<'_>,
    governance: Option<GovernanceContext>,
) -> Result<PlannedAction, TrebError> {
    let operations = build_multisend_operations(run, btxs)?;

    if operations.is_empty() {
        return Ok(PlannedAction {
            run: TransactionRun {
                sender_role: run.sender_role.clone(),
                category: run.category,
                sender_address: run.sender_address,
                tx_indices: run.tx_indices.clone(),
            },
            action: RoutingAction::Exec {
                from: safe_address,
                transactions: Vec::new(),
                safe: None,
                governance,
            },
            queued: None,
        });
    }

    if ctx.is_fork {
        // Fork mode: the executor will call execute_safe_on_fork,
        // which handles approveHash + execTransaction internally.
        // We encode the action as an Exec targeting the safe address.
        let provider = crate::provider::build_http_provider(ctx.rpc_url)?;
        let nonce = super::fork_routing::query_safe_nonce(&provider, safe_address).await?;

        let (to, data, operation) = if operations.len() == 1 {
            let op = &operations[0];
            (op.to, op.data.to_vec(), 0u8)
        } else {
            let multi_send_data = treb_safe::encode_multi_send_call(&operations);
            (treb_safe::MULTI_SEND_ADDRESS, multi_send_data.to_vec(), 1u8)
        };

        let safe_tx = treb_safe::SafeTx {
            to,
            value: U256::ZERO,
            data: data.clone().into(),
            operation,
            safeTxGas: U256::ZERO,
            baseGas: U256::ZERO,
            gasPrice: U256::ZERO,
            gasToken: Address::ZERO,
            refundReceiver: Address::ZERO,
            nonce: U256::from(nonce),
        };
        let safe_tx_hash = treb_safe::compute_safe_tx_hash(ctx.chain_id, safe_address, &safe_tx);

        // For fork mode, we pack the operations into the exec action.
        // The executor will use execute_safe_on_fork which handles
        // the full approveHash + execTransaction flow.
        let exec_tx = RoutableTx {
            to: safe_address,
            value: U256::ZERO,
            data, // the inner MultiSend/direct data — executor unpacks
        };

        Ok(PlannedAction {
            run: TransactionRun {
                sender_role: run.sender_role.clone(),
                category: run.category,
                sender_address: run.sender_address,
                tx_indices: run.tx_indices.clone(),
            },
            action: RoutingAction::Exec {
                from: safe_address,
                transactions: vec![exec_tx],
                safe: Some(SafeContext { safe_address, nonce, safe_tx_hash, threshold: 1 }),
                governance,
            },
            queued: None,
        })
    } else {
        // Live mode: build execTransaction calldata with real ECDSA signature.
        // Query nonce from Safe TX Service.
        let safe_client = treb_safe::SafeServiceClient::new(ctx.chain_id).ok_or_else(|| {
            TrebError::Safe(format!(
                "Safe Transaction Service not available for chain {}",
                ctx.chain_id
            ))
        })?;
        let safe_info = safe_client.get_safe_info(&format!("{}", safe_address)).await?;
        let nonce = safe_info.nonce;

        let (to, data, operation) = if operations.len() == 1 {
            let op = &operations[0];
            (op.to, op.data.to_vec(), 0u8)
        } else {
            let multi_send_data = treb_safe::encode_multi_send_call(&operations);
            (treb_safe::MULTI_SEND_ADDRESS, multi_send_data.to_vec(), 1u8)
        };

        let safe_tx = treb_safe::SafeTx {
            to,
            value: U256::ZERO,
            data: data.clone().into(),
            operation,
            safeTxGas: U256::ZERO,
            baseGas: U256::ZERO,
            gasPrice: U256::ZERO,
            gasToken: Address::ZERO,
            refundReceiver: Address::ZERO,
            nonce: U256::from(nonce),
        };
        let safe_tx_hash = treb_safe::compute_safe_tx_hash(ctx.chain_id, safe_address, &safe_tx);

        // Sign with the signer's private key
        let signer_key = crate::sender::extract_signing_key(
            &run.sender_role,
            resolved_sender,
            ctx.sender_configs,
        )
        .ok_or_else(|| {
            TrebError::Safe(format!("no signing key for Safe sender '{}'", run.sender_role,))
        })?;
        let key_bytes: B256 =
            signer_key.parse().map_err(|e| TrebError::Safe(format!("invalid signer key: {e}")))?;
        let wallet_signer = foundry_wallets::WalletSigner::from_private_key(&key_bytes)
            .map_err(|e| TrebError::Safe(format!("failed to create signer: {e}")))?;
        let signature = treb_safe::sign_safe_tx(&wallet_signer, safe_tx_hash).await?;

        // Build the full execTransaction calldata
        use alloy_sol_types::SolCall;
        let exec_calldata = super::fork_routing::execTransactionCall {
            to,
            value: U256::ZERO,
            data: data.into(),
            operation,
            safeTxGas: U256::ZERO,
            baseGas: U256::ZERO,
            gasPrice: U256::ZERO,
            gasToken: Address::ZERO,
            refundReceiver: Address::ZERO,
            signatures: signature.into(),
        }
        .abi_encode();

        let signer_addr = alloy_signer::Signer::address(&wallet_signer);

        let exec_tx = RoutableTx { to: safe_address, value: U256::ZERO, data: exec_calldata };

        Ok(PlannedAction {
            run: TransactionRun {
                sender_role: run.sender_role.clone(),
                category: run.category,
                sender_address: run.sender_address,
                tx_indices: run.tx_indices.clone(),
            },
            action: RoutingAction::Exec {
                from: signer_addr,
                transactions: vec![exec_tx],
                safe: Some(SafeContext { safe_address, nonce, safe_tx_hash, threshold: 1 }),
                governance,
            },
            queued: None,
        })
    }
}

// ---------------------------------------------------------------------------
// execute_plan — uniform executor
// ---------------------------------------------------------------------------

async fn execute_action_only(
    planned: &PlannedAction,
    ctx: &mut RouteContext<'_>,
    original_btxs: Option<&foundry_cheatcodes::BroadcastableTransactions<Ethereum>>,
) -> Result<RunResult, TrebError> {
    let result = match &planned.action {
        RoutingAction::Exec { from, transactions, safe, governance: _ } => {
            if let Some(safe_ctx) = safe {
                if ctx.is_fork {
                    let provider = crate::provider::build_http_provider(ctx.rpc_url)?;
                    let fallback_btxs =
                        build_btxs_from_routable(safe_ctx.safe_address, transactions);
                    let btxs_to_use = original_btxs.unwrap_or(&fallback_btxs);
                    let receipts = super::fork_routing::execute_safe_on_fork(
                        &provider,
                        &planned.run,
                        btxs_to_use,
                        safe_ctx.safe_address,
                        ctx.chain_id,
                        ctx.quiet,
                    )
                    .await?;
                    RunResult::Broadcast(receipts)
                } else {
                    let receipts = broadcast_routable_txs(
                        ctx.rpc_url,
                        *from,
                        transactions,
                        false,
                        ctx.resolved_senders,
                        ctx.sequence.as_deref_mut(),
                        None,
                    )
                    .await?;
                    RunResult::Broadcast(receipts)
                }
            } else {
                let receipts = broadcast_routable_txs(
                    ctx.rpc_url,
                    *from,
                    transactions,
                    ctx.is_fork,
                    ctx.resolved_senders,
                    ctx.sequence.as_deref_mut(),
                    Some(&planned.run.tx_indices),
                )
                .await?;
                RunResult::Broadcast(receipts)
            }
        }
        RoutingAction::Propose {
            safe_address,
            chain_id,
            operations,
            inner_transactions,
            sender_role,
            nonce,
            governance: _,
        } => {
            if ctx.is_fork || ctx.defer_safe_proposals {
                let safe_tx_hash = match &planned.queued {
                    Some(QueuedExecution::SafeProposal { safe_tx_hash, .. }) => *safe_tx_hash,
                    _ => compute_safe_tx_hash_for_ops(operations, *safe_address, *nonce, *chain_id),
                };
                RunResult::SafeProposed {
                    safe_tx_hash,
                    safe_address: *safe_address,
                    nonce: *nonce,
                    tx_count: inner_transactions.len(),
                }
            } else {
                propose_to_safe_service(
                    *safe_address,
                    *chain_id,
                    *nonce,
                    operations,
                    inner_transactions.len(),
                    sender_role,
                    ctx,
                )
                .await?
            }
        }
    };

    Ok(result)
}

fn safe_proposal_from_action(planned: &PlannedAction) -> Option<QueuedExecution> {
    match (&planned.action, &planned.queued) {
        (_, Some(QueuedExecution::SafeProposal { .. })) => planned.queued.clone(),
        (
            RoutingAction::Propose {
                safe_address,
                chain_id,
                operations,
                nonce,
                inner_transactions,
                ..
            },
            _,
        ) => Some(QueuedExecution::SafeProposal {
            safe_address: *safe_address,
            safe_tx_hash: compute_safe_tx_hash_for_ops(
                operations,
                *safe_address,
                *nonce,
                *chain_id,
            ),
            nonce: *nonce,
            inner_txs: inner_transactions.clone(),
        }),
        _ => None,
    }
}

fn tx_ids_for_run(run: &TransactionRun, recorded_txs: &[RecordedTransaction]) -> Vec<String> {
    run.tx_indices
        .iter()
        .filter_map(|&idx| recorded_txs.get(idx))
        .map(|rt| rt.transaction.id.clone())
        .collect()
}

/// Execute a single planned action, returning the result and optional queued item.
///
/// This remains as a building block for compose's merge path, which still
/// handles proposal deferral separately from the default two-phase executor.
pub async fn execute_single_action(
    planned: &PlannedAction,
    ctx: &mut RouteContext<'_>,
    original_btxs: Option<&foundry_cheatcodes::BroadcastableTransactions<Ethereum>>,
) -> Result<(TransactionRun, RunResult, Option<QueuedExecution>), TrebError> {
    let result = execute_action_only(planned, ctx, original_btxs).await?;

    // Governance wrapping
    let final_result = match &planned.action {
        RoutingAction::Exec { governance: Some(gov), .. } => match &result {
            RunResult::Broadcast(receipts) => {
                let proposal_id =
                    receipts.first().map(|r| format!("{:#x}", r.hash)).unwrap_or_default();
                RunResult::GovernorProposed {
                    proposal_id,
                    governor_address: gov.governor_address,
                    tx_count: planned.run.tx_indices.len(),
                }
            }
            _ => result,
        },
        RoutingAction::Propose { governance: Some(_), .. } => result,
        _ => result,
    };

    Ok((
        TransactionRun {
            sender_role: planned.run.sender_role.clone(),
            category: planned.run.category,
            sender_address: planned.run.sender_address,
            tx_indices: planned.run.tx_indices.clone(),
        },
        final_result,
        planned.queued.clone(),
    ))
}

/// Execute a routing plan in two phases:
/// 1. direct on-chain execution (`Exec`)
/// 2. queued/finalization work (`Propose`, governor queue state)
///
/// Results are returned in original script order even though execution is
/// staged as direct-first and queued-second.
pub async fn execute_plan(
    plan: &RoutingPlan,
    ctx: &mut RouteContext<'_>,
    original_btxs: Option<&foundry_cheatcodes::BroadcastableTransactions<Ethereum>>,
    recorded_txs: &[RecordedTransaction],
    resume: Option<&super::broadcast_writer::ResumeState>,
    mut queued_state: Option<&mut super::broadcast_writer::QueuedOperations>,
) -> Result<ExecutionBundle, TrebError> {
    if let (Some(sequence), Some(queued)) = (ctx.sequence.as_deref(), queued_state.as_deref()) {
        super::broadcast_writer::save_queued_checkpoint(sequence, queued)?;
    }

    let mut direct_results: Vec<Option<RunResult>> =
        (0..plan.actions.len()).map(|_| None).collect();
    let mut final_results: Vec<Option<RunResult>> = (0..plan.actions.len()).map(|_| None).collect();
    let mut queued_executions = Vec::new();

    // Phase 1: execute direct on-chain actions first.
    for (idx, planned) in plan.actions.iter().enumerate() {
        if !matches!(planned.action, RoutingAction::Exec { .. }) {
            continue;
        }

        let skip_governor_direct = planned.queued.as_ref().is_some_and(|queued| {
            matches!(queued, QueuedExecution::GovernanceProposal { governor_address, .. }
            if queued_state.as_deref().is_some_and(|state| {
                super::broadcast_writer::governor_proposal_completed(
                    state,
                    &format!("{:#x}", governor_address),
                    &tx_ids_for_run(&planned.run, recorded_txs),
                )
            }))
        });
        if skip_governor_direct {
            continue;
        }

        let result = if matches!(planned.run.category, SenderCategory::Wallet) {
            if let Some(resume) = resume {
                resume_wallet_run(
                    &planned.run,
                    original_btxs.expect("resume requires btxs"),
                    ctx,
                    resume,
                )
                .await?
            } else {
                execute_action_only(planned, ctx, original_btxs).await?
            }
        } else {
            execute_action_only(planned, ctx, original_btxs).await?
        };

        if planned.queued.is_none() {
            if let Some(cb) = &ctx.on_run_complete {
                cb(&planned.run, &result);
            }
            final_results[idx] = Some(result.clone());
        }

        direct_results[idx] = Some(result);
    }

    // Phase 2: perform queueing/finalization after direct execution settles.
    for (idx, planned) in plan.actions.iter().enumerate() {
        match &planned.action {
            RoutingAction::Exec { governance: Some(gov), .. } => {
                let broadcast =
                    direct_results[idx].clone().unwrap_or(RunResult::Broadcast(Vec::new()));
                let tx_ids = tx_ids_for_run(&planned.run, recorded_txs);
                let proposal_id = match &broadcast {
                    RunResult::Broadcast(receipts) => {
                        receipts.first().map(|r| format!("{:#x}", r.hash)).unwrap_or_default()
                    }
                    _ => queued_state
                        .as_deref()
                        .and_then(|state| {
                            state.governor_proposals.iter().find(|proposal| {
                                proposal
                                    .governor_address
                                    .eq_ignore_ascii_case(&format!("{:#x}", gov.governor_address))
                                    && proposal.transaction_ids == tx_ids
                            })
                        })
                        .map(|proposal| proposal.proposal_id.clone())
                        .unwrap_or_default(),
                };
                if let Some(state) = queued_state.as_deref_mut() {
                    if !super::broadcast_writer::governor_proposal_completed(
                        state,
                        &format!("{:#x}", gov.governor_address),
                        &tx_ids,
                    ) {
                        super::broadcast_writer::mark_governor_proposal_queued(
                            state,
                            &format!("{:#x}", gov.governor_address),
                            &tx_ids,
                            Some(&proposal_id),
                            Some(&proposal_id),
                            None,
                        )?;
                        if let Some(sequence) = ctx.sequence.as_deref() {
                            super::broadcast_writer::save_queued_checkpoint(sequence, state)?;
                        }
                    }
                }

                if let Some(queued) = planned.queued.clone() {
                    queued_executions.push(queued);
                }

                let final_result = RunResult::GovernorProposed {
                    proposal_id,
                    governor_address: gov.governor_address,
                    tx_count: planned.run.tx_indices.len(),
                };
                if let Some(cb) = &ctx.on_run_complete {
                    cb(&planned.run, &final_result);
                }
                final_results[idx] = Some(final_result);
            }
            RoutingAction::Propose { safe_address, nonce, inner_transactions, .. } => {
                let safe_queued = safe_proposal_from_action(planned)
                    .expect("propose action must yield safe queue");
                let safe_tx_hash = match &safe_queued {
                    QueuedExecution::SafeProposal { safe_tx_hash, .. } => {
                        format!("{:#x}", safe_tx_hash)
                    }
                    _ => unreachable!(),
                };
                let already_queued = queued_state.as_deref().is_some_and(|state| {
                    super::broadcast_writer::safe_proposal_completed(state, &safe_tx_hash)
                });

                let safe_result = if already_queued {
                    let safe_tx_hash = safe_tx_hash
                        .parse()
                        .map_err(|e| TrebError::Safe(format!("invalid saved safe tx hash: {e}")))?;
                    RunResult::SafeProposed {
                        safe_tx_hash,
                        safe_address: *safe_address,
                        nonce: *nonce,
                        tx_count: inner_transactions.len(),
                    }
                } else {
                    execute_action_only(planned, ctx, original_btxs).await?
                };

                if let Some(state) = queued_state.as_deref_mut() {
                    super::broadcast_writer::mark_safe_proposal_queued(state, &safe_tx_hash)?;
                    if let Some(sequence) = ctx.sequence.as_deref() {
                        super::broadcast_writer::save_queued_checkpoint(sequence, state)?;
                    }
                }

                queued_executions.push(safe_queued);

                if let Some(QueuedExecution::GovernanceProposal { governor_address, .. }) =
                    planned.queued.as_ref()
                {
                    let tx_ids = tx_ids_for_run(&planned.run, recorded_txs);
                    if let Some(state) = queued_state.as_deref_mut() {
                        if !super::broadcast_writer::governor_proposal_completed(
                            state,
                            &format!("{:#x}", governor_address),
                            &tx_ids,
                        ) {
                            super::broadcast_writer::mark_governor_proposal_queued(
                                state,
                                &format!("{:#x}", governor_address),
                                &tx_ids,
                                None,
                                None,
                                Some(&safe_tx_hash),
                            )?;
                            if let Some(sequence) = ctx.sequence.as_deref() {
                                super::broadcast_writer::save_queued_checkpoint(sequence, state)?;
                            }
                        }
                    }
                    queued_executions
                        .push(planned.queued.clone().expect("governance queue exists"));
                }

                if let Some(cb) = &ctx.on_run_complete {
                    cb(&planned.run, &safe_result);
                }
                final_results[idx] = Some(safe_result);
            }
            _ => {}
        }
    }

    let run_results = plan
        .actions
        .iter()
        .zip(final_results.into_iter())
        .filter_map(|(planned, result)| {
            result.map(|result| {
                (
                    TransactionRun {
                        sender_role: planned.run.sender_role.clone(),
                        category: planned.run.category,
                        sender_address: planned.run.sender_address,
                        tx_indices: planned.run.tx_indices.clone(),
                    },
                    result,
                )
            })
        })
        .collect();

    Ok(ExecutionBundle { run_results, queued_executions })
}

// ---------------------------------------------------------------------------
// Top-level entry points (backward-compatible API)
// ---------------------------------------------------------------------------

/// Route all broadcastable transactions through the queue-reduction model.
///
/// This is the main entry point. Reduces runs into a plan, then executes it.
/// Returns `(TransactionRun, RunResult)` pairs for backward compatibility.
pub async fn route_all(
    btxs: &foundry_cheatcodes::BroadcastableTransactions<Ethereum>,
    ctx: &mut RouteContext<'_>,
) -> Result<Vec<(TransactionRun, RunResult)>, TrebError> {
    let plan = reduce_queue(btxs, ctx).await?;
    let results = execute_plan(&plan, ctx, Some(btxs), &[], None, None).await?;
    Ok(results.run_results)
}

/// Route all with queued execution items returned.
pub async fn route_all_with_queued(
    btxs: &foundry_cheatcodes::BroadcastableTransactions<Ethereum>,
    ctx: &mut RouteContext<'_>,
) -> Result<ExecutionBundle, TrebError> {
    let plan = reduce_queue(btxs, ctx).await?;
    execute_plan(&plan, ctx, Some(btxs), &[], None, None).await
}

/// Route with resume support — skips confirmed, polls pending, re-sends unsent.
///
/// For wallet runs, each transaction is classified as:
/// - **Confirmed**: has an on-chain receipt → skipped, cached receipt used
/// - **Pending**: has a hash but no receipt → polled with retry (3 attempts, 2s delay)
/// - **Unsent**: hash is `None` → re-broadcast
///
/// Pending transactions that remain unconfirmed after polling are treated as
/// dropped and re-broadcast. Nonce conflicts during re-broadcast produce a
/// warning instead of a fatal error.
pub async fn route_all_with_resume(
    btxs: &foundry_cheatcodes::BroadcastableTransactions<Ethereum>,
    ctx: &mut RouteContext<'_>,
    resume: &super::broadcast_writer::ResumeState,
) -> Result<ExecutionBundle, TrebError> {
    let plan = reduce_queue(btxs, ctx).await?;
    execute_plan(&plan, ctx, Some(btxs), &[], Some(resume), None).await
}

// ---------------------------------------------------------------------------
// Resume helpers
// ---------------------------------------------------------------------------

/// Classification of a transaction's resume status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxResumeStatus {
    /// Transaction has an on-chain receipt.
    Confirmed,
    /// Transaction has a hash but no on-chain receipt yet.
    Pending,
    /// Transaction was never sent (hash: None).
    Unsent,
}

/// Classify each transaction in a run by its resume status.
pub fn classify_run_transactions(
    run: &TransactionRun,
    resume: &super::broadcast_writer::ResumeState,
) -> Vec<(usize, TxResumeStatus)> {
    run.tx_indices
        .iter()
        .map(|&idx| {
            let status = resume
                .sequence
                .transactions
                .get(idx)
                .map(|tx_meta| match tx_meta.hash {
                    Some(h) if resume.completed_tx_hashes.contains(&h) => TxResumeStatus::Confirmed,
                    Some(_) => TxResumeStatus::Pending,
                    None => TxResumeStatus::Unsent,
                })
                .unwrap_or(TxResumeStatus::Unsent);
            (idx, status)
        })
        .collect()
}

/// Build a partial `TransactionRun` containing only the specified tx indices.
pub fn build_partial_run(run: &TransactionRun, indices: &[usize]) -> TransactionRun {
    TransactionRun {
        sender_role: run.sender_role.clone(),
        category: run.category,
        sender_address: run.sender_address,
        tx_indices: indices.to_vec(),
    }
}

/// Check whether an error indicates a nonce conflict.
///
/// Nonce conflicts occur when a pending transaction confirms while we attempt
/// to re-send, or when a node has already seen the same nonce. These are
/// non-fatal during resume — the original transaction went through.
pub fn is_nonce_conflict(err: &TrebError) -> bool {
    let msg = match err {
        TrebError::Forge(s) => s,
        _ => return false,
    };
    let lower = msg.to_lowercase();
    lower.contains("nonce too low")
        || lower.contains("nonce has already been used")
        || lower.contains("already known")
        || lower.contains("replacement transaction underpriced")
}

/// Poll for a pending transaction receipt with retry.
///
/// Calls `get_transaction_receipt` up to `max_retries` times with
/// `delay_secs` between attempts. Returns `Some(receipt)` if confirmed,
/// `None` if still pending after all retries.
async fn poll_pending_receipt(
    provider: &impl Provider,
    tx_hash: &B256,
    max_retries: u32,
    delay_secs: u64,
) -> Option<BroadcastReceipt> {
    for attempt in 0..max_retries {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
        }

        if let Ok(receipt) = fetch_receipt(provider, *tx_hash).await {
            return Some(receipt);
        }
    }

    None
}

/// Resume a wallet run: skip confirmed, poll pending, re-broadcast unsent.
async fn resume_wallet_run(
    run: &TransactionRun,
    btxs: &foundry_cheatcodes::BroadcastableTransactions<Ethereum>,
    ctx: &mut RouteContext<'_>,
    resume: &super::broadcast_writer::ResumeState,
) -> Result<RunResult, TrebError> {
    let classified = classify_run_transactions(run, resume);

    // Build a receipt map keyed by tx index
    let mut receipt_map: HashMap<usize, BroadcastReceipt> = HashMap::new();

    // 1. Collect confirmed receipts from resume state
    for &(idx, status) in &classified {
        if status == TxResumeStatus::Confirmed {
            if let Some(tx_meta) = resume.sequence.transactions.get(idx) {
                receipt_map.insert(
                    idx,
                    BroadcastReceipt {
                        hash: tx_meta.hash.unwrap_or_default(),
                        block_number: 0,
                        gas_used: 0,
                        status: true,
                        contract_name: tx_meta.contract_name.clone(),
                        contract_address: tx_meta.contract_address,
                        raw_receipt: None,
                    },
                );
            }
        }
    }

    // 2. Poll pending transactions with retry (3 attempts, 2s delay)
    let pending_items: Vec<(usize, B256)> = classified
        .iter()
        .filter(|(_, s)| *s == TxResumeStatus::Pending)
        .filter_map(|(idx, _)| {
            resume.sequence.transactions.get(*idx).and_then(|tx| tx.hash.map(|h| (*idx, h)))
        })
        .collect();

    if !pending_items.is_empty() {
        let provider = crate::provider::build_http_provider(ctx.rpc_url)?;
        for (idx, hash) in &pending_items {
            match poll_pending_receipt(&provider, hash, 3, 2).await {
                Some(receipt) => {
                    receipt_map.insert(*idx, receipt);
                }
                None => {
                    eprintln!(
                        "warning: tx {:#x} still pending after 3 poll attempts, will re-send",
                        hash
                    );
                }
            }
        }
    }

    // 3. Collect indices that still need broadcasting (unsent + dropped pending)
    let need_broadcast: Vec<usize> =
        run.tx_indices.iter().filter(|idx| !receipt_map.contains_key(idx)).copied().collect();

    // 4. Re-broadcast the unsent/dropped subset
    if !need_broadcast.is_empty() {
        let partial = build_partial_run(run, &need_broadcast);
        match broadcast_wallet_run(
            ctx.rpc_url,
            &partial,
            btxs,
            ctx.is_fork,
            ctx.resolved_senders,
            ctx.sequence.as_deref_mut(),
        )
        .await
        {
            Ok(new_receipts) => {
                for (receipt, &idx) in new_receipts.into_iter().zip(&need_broadcast) {
                    receipt_map.insert(idx, receipt);
                }
            }
            Err(e) if is_nonce_conflict(&e) => {
                eprintln!("warning: nonce conflict during resume broadcast: {e}");
            }
            Err(e) => return Err(e),
        }
    }

    // 5. Assemble receipts in run order
    let receipts: Vec<BroadcastReceipt> =
        run.tx_indices.iter().filter_map(|idx| receipt_map.remove(idx)).collect();

    Ok(RunResult::Broadcast(receipts))
}

// ---------------------------------------------------------------------------
// Flatten receipts
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Wallet broadcast (kept from original)
// ---------------------------------------------------------------------------

/// Broadcast a wallet run's transactions to an RPC endpoint.
///
/// For fork mode (Anvil): uses `anvil_impersonateAccount` + `eth_sendTransaction`.
/// For live mode: signs each transaction with the sender's private key via
/// alloy provider and uses `eth_sendRawTransaction`.
///
/// Returns one `BroadcastReceipt` per transaction.
pub async fn broadcast_wallet_run(
    rpc_url: &str,
    run: &TransactionRun,
    btxs: &foundry_cheatcodes::BroadcastableTransactions<Ethereum>,
    is_fork: bool,
    resolved_senders: &HashMap<String, ResolvedSender>,
    sequence: Option<&mut ScriptSequence<Ethereum>>,
) -> Result<Vec<BroadcastReceipt>, TrebError> {
    if is_fork {
        broadcast_wallet_run_fork(rpc_url, run, btxs, sequence).await
    } else {
        broadcast_wallet_run_live(rpc_url, run, btxs, resolved_senders, sequence).await
    }
}

/// Fork mode: impersonate accounts and send unsigned transactions via Anvil.
async fn broadcast_wallet_run_fork(
    rpc_url: &str,
    run: &TransactionRun,
    btxs: &foundry_cheatcodes::BroadcastableTransactions<Ethereum>,
    sequence: Option<&mut ScriptSequence<Ethereum>>,
) -> Result<Vec<BroadcastReceipt>, TrebError> {
    use alloy_rpc_types::TransactionRequest;

    let provider = crate::provider::build_http_provider(rpc_url)?;
    let mut receipts = Vec::new();
    let mut seq = sequence;

    for &tx_idx in &run.tx_indices {
        let btx = btxs
            .get(tx_idx)
            .ok_or_else(|| TrebError::Forge(format!("transaction index {tx_idx} out of range")))?;

        let from = btx.transaction.from().unwrap_or_default();

        // Build the transaction request
        let mut tx_req = TransactionRequest::default().from(from);

        if let Some(to) = btx.transaction.to() {
            tx_req = tx_req.to(to);
        }

        let input = btx.transaction.input().cloned().unwrap_or_default();
        if !input.is_empty() {
            tx_req.input = alloy_rpc_types::TransactionInput::new(input);
        }

        let value = btx.transaction.value().unwrap_or_default();
        if !value.is_zero() {
            tx_req = tx_req.value(value);
        }

        tx_req.gas = Some(30_000_000);

        // Fork mode: impersonate + sendTransaction (no signing needed)
        super::fork_routing::anvil_impersonate(&provider, from).await?;

        let receipt = provider
            .send_transaction(tx_req)
            .await
            .map_err(|e| TrebError::Forge(format!("tx {} from {:#x} failed: {}", tx_idx, from, e)))?
            .get_receipt()
            .await
            .map_err(|e| TrebError::Forge(format!("get receipt failed: {e}")))?;

        let br = super::fork_routing::receipt_to_broadcast_receipt(&receipt);
        receipts.push(br.clone());

        // Checkpoint: update sequence and save to disk
        if let Some(ref mut s) = seq {
            super::broadcast_writer::update_sequence_checkpoint(s, tx_idx, &br);
            super::broadcast_writer::save_sequence_checkpoint(s)?;
        }

        super::fork_routing::anvil_stop_impersonating(&provider, from).await;
    }

    Ok(receipts)
}

/// Live mode: sign each transaction with the sender's wallet via alloy provider.
async fn broadcast_wallet_run_live(
    rpc_url: &str,
    run: &TransactionRun,
    btxs: &foundry_cheatcodes::BroadcastableTransactions<Ethereum>,
    resolved_senders: &HashMap<String, ResolvedSender>,
    sequence: Option<&mut ScriptSequence<Ethereum>>,
) -> Result<Vec<BroadcastReceipt>, TrebError> {
    use alloy_rpc_types::TransactionRequest;

    let wallet = crate::sender::resolve_wallet_for_address(run.sender_address, resolved_senders)?;

    let provider = crate::provider::build_wallet_provider(rpc_url, wallet)?;

    let mut receipts = Vec::new();
    let mut seq = sequence;

    for &tx_idx in &run.tx_indices {
        let btx = btxs
            .get(tx_idx)
            .ok_or_else(|| TrebError::Forge(format!("transaction index {tx_idx} out of range")))?;

        let mut tx_req = TransactionRequest::default();

        tx_req = tx_req.from(btx.transaction.from().unwrap_or_default());

        if let Some(to) = btx.transaction.to() {
            tx_req = tx_req.to(to);
        }

        let input = btx.transaction.input().cloned().unwrap_or_default();
        if !input.is_empty() {
            tx_req.input = alloy_rpc_types::TransactionInput::new(input);
        }

        let value = btx.transaction.value().unwrap_or_default();
        if !value.is_zero() {
            tx_req = tx_req.value(value);
        }

        let receipt = provider
            .send_transaction(tx_req)
            .await
            .map_err(|e| TrebError::Forge(format!("send tx failed: {e}")))?
            .get_receipt()
            .await
            .map_err(|e| TrebError::Forge(format!("get receipt failed: {e}")))?;

        let br = super::fork_routing::receipt_to_broadcast_receipt(&receipt);

        // Checkpoint: update sequence and save to disk
        if let Some(ref mut s) = seq {
            super::broadcast_writer::update_sequence_checkpoint(s, tx_idx, &br);
            super::broadcast_writer::save_sequence_checkpoint(s)?;
        }

        receipts.push(br);
    }

    Ok(receipts)
}

/// Fetch a transaction receipt by hash via alloy provider.
async fn fetch_receipt(
    provider: &impl Provider,
    tx_hash: B256,
) -> Result<BroadcastReceipt, TrebError> {
    let receipt = provider
        .get_transaction_receipt(tx_hash)
        .await
        .map_err(|e| TrebError::Forge(format!("fetch receipt failed: {e}")))?
        .ok_or_else(|| TrebError::Forge(format!("no receipt for tx {:#x}", tx_hash)))?;

    Ok(super::fork_routing::receipt_to_broadcast_receipt(&receipt))
}

// ---------------------------------------------------------------------------
// Safe proposal to TX Service (live mode)
// ---------------------------------------------------------------------------

/// Sign and propose a transaction to the Safe Transaction Service.
async fn propose_to_safe_service(
    safe_address: Address,
    chain_id: u64,
    nonce: u64,
    operations: &[treb_safe::MultiSendOperation],
    tx_count: usize,
    sender_role: &str,
    ctx: &RouteContext<'_>,
) -> Result<RunResult, TrebError> {
    let (to, data, operation) = if operations.len() == 1 {
        let op = &operations[0];
        (op.to, op.data.clone(), 0u8)
    } else {
        let multi_send_data = treb_safe::encode_multi_send_call(operations);
        (treb_safe::MULTI_SEND_ADDRESS, multi_send_data, 1u8)
    };

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
        nonce: U256::from(nonce),
    };
    let safe_tx_hash = treb_safe::compute_safe_tx_hash(chain_id, safe_address, &safe_tx);

    let resolved_sender = ctx
        .resolved_senders
        .get(sender_role)
        .ok_or_else(|| TrebError::Forge(format!("sender '{}' not found", sender_role)))?;

    let signer_key_hex =
        crate::sender::extract_signing_key(sender_role, resolved_sender, ctx.sender_configs)
            .ok_or_else(|| {
                TrebError::Safe(format!("no signing key for Safe sender '{}'", sender_role,))
            })?;
    let key_bytes: B256 =
        signer_key_hex.parse().map_err(|e| TrebError::Safe(format!("invalid signer key: {e}")))?;
    let wallet_signer = foundry_wallets::WalletSigner::from_private_key(&key_bytes)
        .map_err(|e| TrebError::Safe(format!("failed to create signer: {e}")))?;
    let signature = treb_safe::sign_safe_tx(&wallet_signer, safe_tx_hash).await?;

    let signer_addr = alloy_signer::Signer::address(&wallet_signer);
    let request = treb_safe::types::ProposeRequest {
        to: format!("{}", to),
        value: "0".into(),
        data: Some(format!("0x{}", alloy_primitives::hex::encode(&data))),
        operation,
        safe_tx_gas: "0".into(),
        base_gas: "0".into(),
        gas_price: "0".into(),
        gas_token: format!("{}", Address::ZERO),
        refund_receiver: format!("{}", Address::ZERO),
        nonce,
        contract_transaction_hash: format!("{:#x}", safe_tx_hash),
        sender: format!("{}", signer_addr),
        signature: format!("0x{}", alloy_primitives::hex::encode(&signature)),
        origin: Some("treb".into()),
    };

    let safe_client = treb_safe::SafeServiceClient::new(chain_id).ok_or_else(|| {
        TrebError::Safe(format!("Safe Transaction Service not available for chain {chain_id}"))
    })?;
    safe_client.propose_transaction(&format!("{}", safe_address), &request).await?;

    Ok(RunResult::SafeProposed { safe_tx_hash, safe_address, nonce, tx_count })
}

/// Poll the Safe Transaction Service until a proposed tx is executed.
pub async fn poll_safe_execution(
    chain_id: u64,
    safe_tx_hash: &B256,
    should_continue: impl Fn() -> bool,
) -> Result<Option<String>, TrebError> {
    let safe_client = treb_safe::SafeServiceClient::new(chain_id).ok_or_else(|| {
        TrebError::Safe(format!("Safe Transaction Service not available for chain {chain_id}"))
    })?;
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
// Governor helpers (kept from original)
// ---------------------------------------------------------------------------

/// Extract (targets, values, calldatas) from a governor run's transactions.
#[allow(clippy::type_complexity)]
pub fn extract_governor_tx_data(
    run: &TransactionRun,
    btxs: &foundry_cheatcodes::BroadcastableTransactions<Ethereum>,
) -> Result<(Vec<Address>, Vec<U256>, Vec<Vec<u8>>), TrebError> {
    let mut targets = Vec::with_capacity(run.tx_indices.len());
    let mut values = Vec::with_capacity(run.tx_indices.len());
    let mut calldatas = Vec::with_capacity(run.tx_indices.len());

    for &idx in &run.tx_indices {
        let btx = btxs
            .get(idx)
            .ok_or_else(|| TrebError::Forge(format!("transaction index {idx} out of range")))?;

        let to = btx.transaction.to().unwrap_or(Address::ZERO);

        let value = btx.transaction.value().unwrap_or_default();
        let data = btx.transaction.input().cloned().unwrap_or_default().to_vec();

        targets.push(to);
        values.push(U256::from(value));
        calldatas.push(data);
    }

    Ok((targets, values, calldatas))
}

/// ABI-encode `Governor.propose(address[], uint256[], bytes[], string)`.
///
/// Selector: `0x7d5e81e2` (from OZ Governor).
pub fn encode_governor_propose(
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
    )
        .abi_encode_params();

    let mut calldata = selector.to_vec();
    calldata.extend_from_slice(&encoded);
    calldata
}

/// Build a synthetic `BroadcastableTransactions` with a single transaction.
fn build_single_tx_broadcast(
    from: Address,
    to: Address,
    calldata: Vec<u8>,
) -> foundry_cheatcodes::BroadcastableTransactions<Ethereum> {
    use alloy_rpc_types::{TransactionInput, TransactionRequest};
    use foundry_cheatcodes::BroadcastableTransaction;
    use foundry_common::TransactionMaybeSigned;

    let mut tx_req = TransactionRequest::default().from(from).to(to);
    tx_req.input = TransactionInput::new(alloy_primitives::Bytes::from(calldata));

    let btx =
        BroadcastableTransaction { rpc: None, transaction: TransactionMaybeSigned::new(tx_req) };
    let mut btxs = foundry_cheatcodes::BroadcastableTransactions::<Ethereum>::default();
    btxs.push_back(btx);
    btxs
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Extract `RoutableTx` items from a run's broadcastable transactions.
fn extract_routable_txs(
    run: &TransactionRun,
    btxs: &foundry_cheatcodes::BroadcastableTransactions<Ethereum>,
) -> Result<Vec<RoutableTx>, TrebError> {
    let mut txs = Vec::with_capacity(run.tx_indices.len());
    for &idx in &run.tx_indices {
        let btx = btxs
            .get(idx)
            .ok_or_else(|| TrebError::Forge(format!("transaction index {idx} out of range")))?;
        let to = btx.transaction.to().unwrap_or(Address::ZERO);
        let value = btx.transaction.value().unwrap_or_default();
        let data = btx.transaction.input().cloned().unwrap_or_default().to_vec();
        txs.push(RoutableTx { to, value: U256::from(value), data });
    }
    Ok(txs)
}

/// Build MultiSend operations from a run's broadcastable transactions.
fn build_multisend_operations(
    run: &TransactionRun,
    btxs: &foundry_cheatcodes::BroadcastableTransactions<Ethereum>,
) -> Result<Vec<treb_safe::MultiSendOperation>, TrebError> {
    let mut ops = Vec::with_capacity(run.tx_indices.len());
    for &idx in &run.tx_indices {
        let btx = btxs
            .get(idx)
            .ok_or_else(|| TrebError::Forge(format!("transaction index {idx} out of range")))?;
        let to = btx.transaction.to().unwrap_or(Address::ZERO);
        let value = btx.transaction.value().unwrap_or_default();
        let data = btx.transaction.input().cloned().unwrap_or_default();
        ops.push(treb_safe::MultiSendOperation {
            operation: 0, // Call
            to,
            value: U256::from(value),
            data,
        });
    }
    Ok(ops)
}

/// Compute the safeTxHash for a set of MultiSend operations.
pub fn compute_safe_tx_hash_for_ops(
    operations: &[treb_safe::MultiSendOperation],
    safe_address: Address,
    nonce: u64,
    chain_id: u64,
) -> B256 {
    let (to, data, operation) = if operations.len() == 1 {
        let op = &operations[0];
        (op.to, op.data.to_vec(), 0u8)
    } else {
        let multi_send_data = treb_safe::encode_multi_send_call(operations);
        (treb_safe::MULTI_SEND_ADDRESS, multi_send_data.to_vec(), 1u8)
    };

    let safe_tx = treb_safe::SafeTx {
        to,
        value: U256::ZERO,
        data: data.into(),
        operation,
        safeTxGas: U256::ZERO,
        baseGas: U256::ZERO,
        gasPrice: U256::ZERO,
        gasToken: Address::ZERO,
        refundReceiver: Address::ZERO,
        nonce: U256::from(nonce),
    };
    treb_safe::compute_safe_tx_hash(chain_id, safe_address, &safe_tx)
}

/// Broadcast routable transactions via JSON-RPC.
///
/// For fork mode (Anvil): uses `anvil_impersonateAccount` + `eth_sendTransaction`.
/// For live mode: signs each transaction with the sender's wallet via alloy provider.
async fn broadcast_routable_txs(
    rpc_url: &str,
    from: Address,
    transactions: &[RoutableTx],
    is_fork: bool,
    resolved_senders: &HashMap<String, ResolvedSender>,
    sequence: Option<&mut ScriptSequence<Ethereum>>,
    tx_indices: Option<&[usize]>,
) -> Result<Vec<BroadcastReceipt>, TrebError> {
    if is_fork {
        broadcast_routable_txs_fork(rpc_url, from, transactions, sequence, tx_indices).await
    } else {
        broadcast_routable_txs_live(
            rpc_url,
            from,
            transactions,
            resolved_senders,
            sequence,
            tx_indices,
        )
        .await
    }
}

/// Fork mode: impersonate accounts and send unsigned routable transactions via Anvil.
async fn broadcast_routable_txs_fork(
    rpc_url: &str,
    from: Address,
    transactions: &[RoutableTx],
    sequence: Option<&mut ScriptSequence<Ethereum>>,
    tx_indices: Option<&[usize]>,
) -> Result<Vec<BroadcastReceipt>, TrebError> {
    use alloy_rpc_types::TransactionRequest;

    let provider = crate::provider::build_http_provider(rpc_url)?;
    let mut receipts = Vec::new();
    let mut seq = sequence;
    // Only checkpoint when tx_indices aligns 1:1 with transactions
    let can_checkpoint = tx_indices.is_some_and(|idx| idx.len() == transactions.len());

    for (i, tx) in transactions.iter().enumerate() {
        let mut tx_req = TransactionRequest::default().from(from).to(tx.to);
        if !tx.data.is_empty() {
            tx_req.input = alloy_rpc_types::TransactionInput::new(alloy_primitives::Bytes::from(
                tx.data.clone(),
            ));
        }
        if !tx.value.is_zero() {
            tx_req = tx_req.value(tx.value);
        }
        tx_req.gas = Some(30_000_000);

        super::fork_routing::anvil_impersonate(&provider, from).await?;

        let receipt = provider
            .send_transaction(tx_req)
            .await
            .map_err(|e| {
                TrebError::Forge(format!("tx to {:#x} from {:#x} failed: {}", tx.to, from, e))
            })?
            .get_receipt()
            .await
            .map_err(|e| TrebError::Forge(format!("get receipt failed: {e}")))?;

        let br = super::fork_routing::receipt_to_broadcast_receipt(&receipt);
        receipts.push(br.clone());

        // Checkpoint: update sequence and save to disk
        if can_checkpoint {
            if let (Some(s), Some(idx)) = (&mut seq, tx_indices.and_then(|ti| ti.get(i))) {
                super::broadcast_writer::update_sequence_checkpoint(s, *idx, &br);
                super::broadcast_writer::save_sequence_checkpoint(s)?;
            }
        }

        super::fork_routing::anvil_stop_impersonating(&provider, from).await;
    }

    Ok(receipts)
}

/// Live mode: sign each routable transaction with the sender's wallet via alloy provider.
async fn broadcast_routable_txs_live(
    rpc_url: &str,
    from: Address,
    transactions: &[RoutableTx],
    resolved_senders: &HashMap<String, ResolvedSender>,
    sequence: Option<&mut ScriptSequence<Ethereum>>,
    tx_indices: Option<&[usize]>,
) -> Result<Vec<BroadcastReceipt>, TrebError> {
    use alloy_rpc_types::TransactionRequest;

    let wallet = crate::sender::resolve_wallet_for_address(from, resolved_senders)?;

    let provider = crate::provider::build_wallet_provider(rpc_url, wallet)?;

    let mut receipts = Vec::new();
    let mut seq = sequence;
    // Only checkpoint when tx_indices aligns 1:1 with transactions
    let can_checkpoint = tx_indices.is_some_and(|idx| idx.len() == transactions.len());

    for (i, tx) in transactions.iter().enumerate() {
        let mut tx_req = TransactionRequest::default();
        tx_req = tx_req.from(from);
        tx_req = tx_req.to(tx.to);

        if !tx.data.is_empty() {
            tx_req.input = alloy_rpc_types::TransactionInput::new(alloy_primitives::Bytes::from(
                tx.data.clone(),
            ));
        }

        if !tx.value.is_zero() {
            tx_req = tx_req.value(tx.value);
        }

        let receipt = provider
            .send_transaction(tx_req)
            .await
            .map_err(|e| TrebError::Forge(format!("send tx failed: {e}")))?
            .get_receipt()
            .await
            .map_err(|e| TrebError::Forge(format!("get receipt failed: {e}")))?;

        let br = super::fork_routing::receipt_to_broadcast_receipt(&receipt);

        // Checkpoint: update sequence and save to disk
        if can_checkpoint {
            if let (Some(s), Some(idx)) = (&mut seq, tx_indices.and_then(|ti| ti.get(i))) {
                super::broadcast_writer::update_sequence_checkpoint(s, *idx, &br);
                super::broadcast_writer::save_sequence_checkpoint(s)?;
            }
        }

        receipts.push(br);
    }

    Ok(receipts)
}

/// Build synthetic `BroadcastableTransactions` from routable transactions.
fn build_btxs_from_routable(
    from: Address,
    transactions: &[RoutableTx],
) -> foundry_cheatcodes::BroadcastableTransactions<Ethereum> {
    use alloy_rpc_types::{TransactionInput, TransactionRequest};
    use foundry_common::TransactionMaybeSigned;

    let mut btxs = foundry_cheatcodes::BroadcastableTransactions::<Ethereum>::default();
    for tx in transactions {
        let mut tx_req = TransactionRequest::default().from(from).to(tx.to);
        tx_req.input = TransactionInput::new(alloy_primitives::Bytes::from(tx.data.clone()));
        if !tx.value.is_zero() {
            tx_req = tx_req.value(tx.value);
        }
        btxs.push_back(foundry_cheatcodes::BroadcastableTransaction {
            rpc: None,
            transaction: TransactionMaybeSigned::new(tx_req),
        });
    }
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
        let btxs = foundry_cheatcodes::BroadcastableTransactions::<Ethereum>::default();
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

    #[test]
    fn extract_routable_txs_empty_run() {
        let btxs = foundry_cheatcodes::BroadcastableTransactions::<Ethereum>::default();
        let run = TransactionRun {
            sender_role: "test".into(),
            category: SenderCategory::Wallet,
            sender_address: Address::ZERO,
            tx_indices: vec![],
        };

        let txs = extract_routable_txs(&run, &btxs).unwrap();
        assert!(txs.is_empty());
    }

    #[test]
    fn compute_safe_tx_hash_for_single_op() {
        let ops = vec![treb_safe::MultiSendOperation {
            operation: 0,
            to: Address::ZERO,
            value: U256::ZERO,
            data: alloy_primitives::Bytes::new(),
        }];
        let hash = compute_safe_tx_hash_for_ops(&ops, Address::ZERO, 0, 1);
        // Just verify it returns a non-zero hash
        assert_ne!(hash, B256::ZERO);
    }

    // -----------------------------------------------------------------------
    // Resume helper tests
    // -----------------------------------------------------------------------

    use alloy_primitives::map::HashMap as AlloyHashMap;
    use forge_script_sequence::{ScriptSequence, TransactionWithMetadata};
    use foundry_common::TransactionMaybeSigned;
    use std::collections::VecDeque;

    /// Build a minimal ScriptSequence with transactions having the given hashes.
    fn make_resume_sequence(tx_hashes: &[Option<B256>]) -> ScriptSequence<Ethereum> {
        let from = Address::repeat_byte(0x01);
        let to = Address::repeat_byte(0x02);
        let mut transactions = VecDeque::new();
        for hash in tx_hashes {
            let tx_json = serde_json::json!({
                "from": format!("{:#x}", from),
                "to": format!("{:#x}", to),
                "data": "0x01",
            });
            let tx: TransactionMaybeSigned<Ethereum> =
                serde_json::from_value(tx_json).expect("build test tx");
            let mut tx_meta = TransactionWithMetadata::from_tx_request(tx);
            tx_meta.hash = *hash;
            transactions.push_back(tx_meta);
        }
        ScriptSequence {
            transactions,
            receipts: Vec::new(),
            libraries: Vec::new(),
            pending: Vec::new(),
            paths: None,
            returns: AlloyHashMap::default(),
            timestamp: 0,
            chain: 1,
            commit: None,
        }
    }

    /// Build a ResumeState from given hash classifications.
    fn make_resume_state(
        tx_hashes: &[Option<B256>],
        completed: &[B256],
        pending: &[B256],
    ) -> super::super::broadcast_writer::ResumeState {
        super::super::broadcast_writer::ResumeState {
            sequence: make_resume_sequence(tx_hashes),
            queued: None,
            completed_tx_hashes: completed.iter().copied().collect(),
            pending_tx_hashes: pending.iter().copied().collect(),
            completed_safe_hashes: std::collections::HashSet::new(),
            completed_gov_ids: std::collections::HashSet::new(),
        }
    }

    #[test]
    fn classify_all_confirmed() {
        let h1 = B256::repeat_byte(0x11);
        let h2 = B256::repeat_byte(0x22);
        let resume = make_resume_state(&[Some(h1), Some(h2)], &[h1, h2], &[]);
        let run = TransactionRun {
            sender_role: "deployer".into(),
            category: SenderCategory::Wallet,
            sender_address: Address::repeat_byte(0x01),
            tx_indices: vec![0, 1],
        };

        let classified = classify_run_transactions(&run, &resume);
        assert_eq!(classified.len(), 2);
        assert_eq!(classified[0], (0, TxResumeStatus::Confirmed));
        assert_eq!(classified[1], (1, TxResumeStatus::Confirmed));
    }

    #[test]
    fn classify_all_unsent() {
        let resume = make_resume_state(&[None, None], &[], &[]);
        let run = TransactionRun {
            sender_role: "deployer".into(),
            category: SenderCategory::Wallet,
            sender_address: Address::repeat_byte(0x01),
            tx_indices: vec![0, 1],
        };

        let classified = classify_run_transactions(&run, &resume);
        assert_eq!(classified[0].1, TxResumeStatus::Unsent);
        assert_eq!(classified[1].1, TxResumeStatus::Unsent);
    }

    #[test]
    fn classify_mixed_confirmed_pending_unsent() {
        let confirmed_hash = B256::repeat_byte(0x11);
        let pending_hash = B256::repeat_byte(0x22);
        let resume = make_resume_state(
            &[Some(confirmed_hash), Some(pending_hash), None],
            &[confirmed_hash],
            &[pending_hash],
        );
        let run = TransactionRun {
            sender_role: "deployer".into(),
            category: SenderCategory::Wallet,
            sender_address: Address::repeat_byte(0x01),
            tx_indices: vec![0, 1, 2],
        };

        let classified = classify_run_transactions(&run, &resume);
        assert_eq!(classified[0], (0, TxResumeStatus::Confirmed));
        assert_eq!(classified[1], (1, TxResumeStatus::Pending));
        assert_eq!(classified[2], (2, TxResumeStatus::Unsent));
    }

    #[test]
    fn classify_out_of_range_index_is_unsent() {
        let resume = make_resume_state(&[None], &[], &[]);
        let run = TransactionRun {
            sender_role: "deployer".into(),
            category: SenderCategory::Wallet,
            sender_address: Address::repeat_byte(0x01),
            tx_indices: vec![99], // out of range
        };

        let classified = classify_run_transactions(&run, &resume);
        assert_eq!(classified[0].1, TxResumeStatus::Unsent);
    }

    #[test]
    fn build_partial_run_preserves_metadata() {
        let run = TransactionRun {
            sender_role: "deployer".into(),
            category: SenderCategory::Wallet,
            sender_address: Address::repeat_byte(0x01),
            tx_indices: vec![0, 1, 2, 3, 4],
        };

        let partial = build_partial_run(&run, &[1, 3]);
        assert_eq!(partial.sender_role, "deployer");
        assert_eq!(partial.category, SenderCategory::Wallet);
        assert_eq!(partial.sender_address, Address::repeat_byte(0x01));
        assert_eq!(partial.tx_indices, vec![1, 3]);
    }

    #[test]
    fn build_partial_run_empty_indices() {
        let run = TransactionRun {
            sender_role: "admin".into(),
            category: SenderCategory::Wallet,
            sender_address: Address::repeat_byte(0x02),
            tx_indices: vec![0, 1, 2],
        };

        let partial = build_partial_run(&run, &[]);
        assert!(partial.tx_indices.is_empty());
        assert_eq!(partial.sender_role, "admin");
    }

    #[test]
    fn is_nonce_conflict_detects_nonce_too_low() {
        let err = TrebError::Forge("send tx failed: nonce too low".into());
        assert!(is_nonce_conflict(&err));
    }

    #[test]
    fn is_nonce_conflict_detects_already_known() {
        let err = TrebError::Forge("tx from 0x01 failed: already known".into());
        assert!(is_nonce_conflict(&err));
    }

    #[test]
    fn is_nonce_conflict_detects_replacement_underpriced() {
        let err = TrebError::Forge("send tx failed: replacement transaction underpriced".into());
        assert!(is_nonce_conflict(&err));
    }

    #[test]
    fn is_nonce_conflict_detects_nonce_already_used() {
        let err = TrebError::Forge("nonce has already been used".into());
        assert!(is_nonce_conflict(&err));
    }

    #[test]
    fn is_nonce_conflict_false_for_other_errors() {
        let err = TrebError::Forge("insufficient funds for gas".into());
        assert!(!is_nonce_conflict(&err));
    }

    #[test]
    fn is_nonce_conflict_false_for_non_forge_errors() {
        let err = TrebError::Registry("nonce too low".into());
        assert!(!is_nonce_conflict(&err));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn poll_pending_receipt_returns_receipt_on_confirm() {
        // Start a mock RPC server that returns a complete receipt
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let rpc_url = format!("http://127.0.0.1:{port}");

        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = vec![0u8; 4096];
            let _ = stream.read(&mut buf).await;
            let zeros_bloom = "0x".to_owned() + &"0".repeat(512);
            let resp_body = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "transactionHash": "0x1111111111111111111111111111111111111111111111111111111111111111",
                    "transactionIndex": "0x0",
                    "blockHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
                    "blockNumber": "0xa",
                    "from": "0x0000000000000000000000000000000000000000",
                    "to": "0x0000000000000000000000000000000000000000",
                    "gasUsed": "0x5208",
                    "cumulativeGasUsed": "0x5208",
                    "effectiveGasPrice": "0x0",
                    "status": "0x1",
                    "logs": [],
                    "logsBloom": zeros_bloom,
                    "type": "0x0",
                },
            });
            let resp_str = resp_body.to_string();
            let http_resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                resp_str.len(),
                resp_str,
            );
            let _ = stream.write_all(http_resp.as_bytes()).await;
        });

        let provider = crate::provider::build_http_provider(&rpc_url).unwrap();
        let hash = B256::repeat_byte(0x11);
        let result = poll_pending_receipt(&provider, &hash, 1, 0).await;
        assert!(result.is_some(), "should return receipt");
        let receipt = result.unwrap();
        assert_eq!(receipt.block_number, 10);
        assert!(receipt.status);

        handle.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn poll_pending_receipt_returns_none_when_still_pending() {
        // Mock server that always returns null result
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let rpc_url = format!("http://127.0.0.1:{port}");

        let handle = tokio::spawn(async move {
            for _ in 0..3 {
                let Ok((mut stream, _)) = listener.accept().await else { break };
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = vec![0u8; 4096];
                let _ = stream.read(&mut buf).await;
                let resp_body = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": null,
                });
                let resp_str = resp_body.to_string();
                let http_resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                    resp_str.len(),
                    resp_str,
                );
                let _ = stream.write_all(http_resp.as_bytes()).await;
            }
        });

        let provider = crate::provider::build_http_provider(&rpc_url).unwrap();
        let hash = B256::repeat_byte(0x22);
        // Use 0 delay so test is fast; 3 retries
        let result = poll_pending_receipt(&provider, &hash, 3, 0).await;
        assert!(result.is_none(), "should return None for pending tx");

        handle.abort();
    }
}
