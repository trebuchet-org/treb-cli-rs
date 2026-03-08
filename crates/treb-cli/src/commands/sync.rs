use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use alloy_chains::Chain;
use anyhow::Context;
use chrono::Utc;
use owo_colors::OwoColorize;
use serde::Serialize;
use treb_core::types::{ProposalStatus, enums::TransactionStatus, safe_transaction::Confirmation};
use treb_forge::{is_terminal, query_proposal_state};
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
    governor_synced: usize,
    governor_updated: usize,
    governor_newly_executed: usize,
    governor_removed: usize,
    errors: Vec<String>,
}

// ── Detail table row ────────────────────────────────────────────────────

struct SyncDetailRow {
    safe_tx_hash: String,
    safe_address: String,
    status: String,
    confirmations: usize,
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

// ── RPC URL resolution for governor sync ────────────────────────────────

#[derive(Default)]
struct ResolvedRpcUrls {
    rpc_map: HashMap<u64, String>,
    warnings: Vec<String>,
}

fn missing_env_vars(value: &str) -> Vec<String> {
    let mut missing = Vec::new();
    let mut remainder = value;

    while let Some(start) = remainder.find("${") {
        let after_start = &remainder[start + 2..];
        let Some(end) = after_start.find('}') else {
            break;
        };

        let var_name = &after_start[..end];
        if !var_name.is_empty() && std::env::var_os(var_name).is_none() {
            missing.push(var_name.to_string());
        }

        remainder = &after_start[end + 1..];
    }

    missing.sort();
    missing.dedup();
    missing
}

fn expand_rpc_endpoint(name: &str, raw_url: &str) -> Result<String, String> {
    let missing_vars = missing_env_vars(raw_url);
    if !missing_vars.is_empty() {
        return Err(format!(
            "RPC endpoint '{name}' is missing required environment variables after .env expansion: {}",
            missing_vars.join(", ")
        ));
    }

    let expanded = treb_config::expand_env_vars(raw_url);
    if expanded.trim().is_empty() {
        return Err(format!("RPC endpoint '{name}' is empty after .env expansion"));
    }

    Ok(expanded)
}

/// Resolve RPC URLs for a set of chain IDs by probing foundry.toml endpoints.
///
/// Iterates over configured `[rpc_endpoints]`, calls `eth_chainId` on each,
/// and builds a chain_id → URL map for the requested chains.
async fn resolve_rpc_urls(
    cwd: &std::path::Path,
    needed_chains: &HashSet<u64>,
    debug: bool,
) -> ResolvedRpcUrls {
    treb_config::load_dotenv(cwd);

    let config = match treb_config::load_foundry_config(cwd) {
        Ok(c) => c,
        Err(_) => return ResolvedRpcUrls::default(),
    };
    let endpoints = treb_config::rpc_endpoints(&config);

    let mut resolved = ResolvedRpcUrls::default();

    for (name, url) in &endpoints {
        let url = match expand_rpc_endpoint(name, url) {
            Ok(url) => url,
            Err(warning) => {
                resolved.warnings.push(warning);
                continue;
            }
        };

        if resolved.rpc_map.len() == needed_chains.len() {
            break; // found all needed chains
        }

        if debug {
            eprintln!("[debug] probing RPC endpoint '{name}' for chain ID...");
        }

        match super::run::fetch_chain_id(&url).await {
            Ok(chain_id) => {
                if needed_chains.contains(&chain_id) && !resolved.rpc_map.contains_key(&chain_id) {
                    if debug {
                        eprintln!("[debug] endpoint '{name}' → chain {chain_id}");
                    }
                    resolved.rpc_map.insert(chain_id, url);
                }
            }
            Err(e) => {
                if debug {
                    eprintln!("[debug] failed to probe endpoint '{name}': {e}");
                }
            }
        }
    }

    resolved
}

fn persist_governor_proposal_update(
    registry: &mut Registry,
    updated: treb_core::types::GovernorProposal,
    became_executed: bool,
) -> anyhow::Result<()> {
    if became_executed {
        for tx_id in &updated.transaction_ids {
            if let Some(tx) = registry.get_transaction(tx_id) {
                let mut tx = tx.clone();
                if tx.status != TransactionStatus::Executed {
                    tx.status = TransactionStatus::Executed;
                    tx.hash = updated.execution_tx_hash.clone();
                    registry
                        .update_transaction(tx)
                        .with_context(|| format!("failed to update transaction {tx_id}"))?;
                }
            }
        }
    }

    registry.update_governor_proposal(updated).map_err(|e| anyhow::anyhow!("{e}"))
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
    let safe_filtered: Vec<_> = safe_txs
        .into_iter()
        .filter(|stx| match chain_filter {
            Some(cid) => stx.chain_id == cid,
            None => true,
        })
        .collect();

    // List non-terminal governor proposals, optionally filtered by chain
    let gov_proposals = registry.list_governor_proposals();
    let gov_filtered: Vec<_> = gov_proposals
        .into_iter()
        .filter(|p| match chain_filter {
            Some(cid) => p.chain_id == cid,
            None => true,
        })
        .filter(|p| !is_terminal(&p.status))
        .cloned()
        .collect();

    if safe_filtered.is_empty() && gov_filtered.is_empty() {
        if json {
            output::print_json(&SyncOutputJson {
                synced: 0,
                updated: 0,
                newly_executed: 0,
                removed: 0,
                governor_synced: 0,
                governor_updated: 0,
                governor_newly_executed: 0,
                governor_removed: 0,
                errors: vec![],
            })?;
        } else {
            match &network {
                Some(name) => {
                    println!("No safe transactions or governor proposals found for network {name}.")
                }
                None => println!("No safe transactions or governor proposals in the registry."),
            }
        }
        return Ok(());
    }

    let mut updated_count = 0usize;
    let mut newly_executed_count = 0usize;
    let mut removed_count = 0usize;
    let mut errors: Vec<String> = Vec::new();
    let mut detail_rows: Vec<SyncDetailRow> = Vec::new();
    let synced_count = safe_filtered.len();

    // ── Safe transaction sync ───────────────────────────────────────────

    if !safe_filtered.is_empty() {
        if !json {
            output::print_stage("\u{1f50d}", "Syncing safe transactions...");
        }

        // Group safe transactions by (safe_address, chain_id) for batched API calls.
        let mut groups: HashMap<(String, u64), Vec<String>> = HashMap::new();
        for stx in &safe_filtered {
            groups
                .entry((stx.safe_address.clone(), stx.chain_id))
                .or_default()
                .push(stx.safe_tx_hash.clone());
        }

        // Cache SafeServiceClient instances per chain_id to avoid redundant construction.
        let mut clients: HashMap<u64, SafeServiceClient> = HashMap::new();

        if !json {
            output::print_stage("\u{2699}\u{fe0f}", "Processing safe transactions...");
        }

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
                            eprintln!(
                                "{}",
                                output::format_warning_banner("\u{26a0}\u{fe0f}", &msg)
                            );
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
            let service_resp: SafeServiceMultisigResponse = match client
                .get_multisig_transactions(safe_address)
                .await
            {
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
                    let old_status_str = local_stx.status.to_string();
                    let mut updated_stx = local_stx.clone();

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

                    // Update execution fields from service data.
                    let became_executed =
                        service_tx.is_executed && updated_stx.status != TransactionStatus::Executed;
                    if service_tx.is_executed {
                        updated_stx.status = TransactionStatus::Executed;
                        updated_stx.executed_at = service_tx.execution_date;
                        updated_stx.execution_tx_hash =
                            service_tx.transaction_hash.clone().unwrap_or_default();
                    }
                    if became_executed {
                        newly_executed_count += 1;
                    }

                    let confirmations_changed =
                        updated_stx.confirmations != local_stx.confirmations;
                    let status_changed = updated_stx.status != local_stx.status;
                    let executed_at_changed = updated_stx.executed_at != local_stx.executed_at;
                    let execution_tx_hash_changed =
                        updated_stx.execution_tx_hash != local_stx.execution_tx_hash;
                    let has_changes = confirmations_changed
                        || status_changed
                        || executed_at_changed
                        || execution_tx_hash_changed;

                    if has_changes {
                        // Persist updated safe transaction
                        registry.update_safe_transaction(updated_stx.clone()).with_context(
                            || format!("failed to update safe transaction {local_hash}"),
                        )?;
                        updated_count += 1;

                        detail_rows.push(SyncDetailRow {
                            safe_tx_hash: local_hash.clone(),
                            safe_address: safe_address.clone(),
                            status: if became_executed {
                                format!("{old_status_str} -> Executed")
                            } else {
                                "Updated".to_string()
                            },
                            confirmations: updated_stx.confirmations.len(),
                        });
                    }

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
                    detail_rows.push(SyncDetailRow {
                        safe_tx_hash: local_hash.clone(),
                        safe_address: safe_address.clone(),
                        status: "Removed".to_string(),
                        confirmations: 0,
                    });
                }
            }
        }
    } // end if !safe_filtered.is_empty()

    // ── Governor proposal sync ──────────────────────────────────────────

    let gov_synced_count = gov_filtered.len();
    let mut gov_updated_count = 0usize;
    let mut gov_newly_executed_count = 0usize;
    let mut gov_removed_count = 0usize;

    if !gov_filtered.is_empty() {
        if !json {
            output::print_stage("\u{1f3db}\u{fe0f}", "Syncing governor proposals...");
        }

        // Resolve RPC URLs for needed chain_ids from foundry.toml endpoints
        let needed_chains: HashSet<u64> = gov_filtered.iter().map(|p| p.chain_id).collect();
        let resolved_rpc_urls = resolve_rpc_urls(&cwd, &needed_chains, debug).await;
        let rpc_map = resolved_rpc_urls.rpc_map;

        for warning in resolved_rpc_urls.warnings {
            errors.push(warning.clone());
            if !json {
                eprintln!("{}", output::format_warning_banner("\u{26a0}\u{fe0f}", &warning));
            }
        }

        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("failed to build HTTP client")?;

        for proposal in &gov_filtered {
            let rpc_url = match rpc_map.get(&proposal.chain_id) {
                Some(url) => url,
                None => {
                    let msg = format!(
                        "no RPC endpoint found for chain {} (governor {})",
                        proposal.chain_id,
                        output::truncate_address(&proposal.governor_address)
                    );
                    errors.push(msg.clone());
                    if !json {
                        eprintln!("{}", output::format_warning_banner("\u{26a0}\u{fe0f}", &msg));
                    }
                    continue;
                }
            };

            if debug {
                eprintln!(
                    "[debug] querying Governor state for proposal {} on {}",
                    output::truncate_address(&proposal.proposal_id),
                    output::truncate_address(&proposal.governor_address),
                );
            }

            match query_proposal_state(
                &http_client,
                rpc_url,
                &proposal.governor_address,
                &proposal.proposal_id,
            )
            .await
            {
                Ok(new_status) => {
                    if new_status != proposal.status {
                        let mut updated = proposal.clone();
                        updated.status = new_status.clone();

                        let became_executed = new_status == ProposalStatus::Executed;
                        if became_executed {
                            updated.executed_at = Some(Utc::now());
                            gov_newly_executed_count += 1;
                        }

                        persist_governor_proposal_update(&mut registry, updated, became_executed)?;
                        gov_updated_count += 1;
                    }
                }
                Err(e) => {
                    let err_str = e.to_string();
                    if clean && err_str.contains("call reverted") {
                        // --clean: remove proposals whose governor contract is unreachable
                        registry
                            .remove_governor_proposal(&proposal.proposal_id)
                            .map_err(|e| anyhow::anyhow!("{e}"))?;
                        gov_removed_count += 1;
                    } else {
                        let msg = format!(
                            "governor {} (chain {}): {}",
                            output::truncate_address(&proposal.governor_address),
                            proposal.chain_id,
                            err_str
                        );
                        errors.push(msg.clone());
                        if !json {
                            eprintln!(
                                "{}",
                                output::format_warning_banner("\u{26a0}\u{fe0f}", &msg)
                            );
                        }
                    }
                }
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
            governor_synced: gov_synced_count,
            governor_updated: gov_updated_count,
            governor_newly_executed: gov_newly_executed_count,
            governor_removed: gov_removed_count,
            errors,
        })?;
    } else {
        // Print per-transaction detail table when there are changes
        if !detail_rows.is_empty() {
            let mut table = output::build_table(&["SafeTxHash", "Safe", "Status", "Confirmations"]);
            for row in &detail_rows {
                table.add_row(vec![
                    output::truncate_address(&row.safe_tx_hash),
                    output::truncate_address(&row.safe_address),
                    row.status.clone(),
                    row.confirmations.to_string(),
                ]);
            }
            output::print_table(&table);
            println!();
        }

        println!("{}", output::format_stage("\u{2705}", "Sync complete."));
        if synced_count > 0 {
            println!("  Safe transactions synced: {synced_count}");
            if color::is_color_enabled() {
                println!("  Updated:                  {}", updated_count.style(color::WARNING));
                println!(
                    "  Newly executed:           {}",
                    newly_executed_count.style(color::SUCCESS)
                );
            } else {
                println!("  Updated:                  {updated_count}");
                println!("  Newly executed:           {newly_executed_count}");
            }
            if clean {
                println!("  Removed (stale):          {removed_count}");
            }
        }
        if gov_synced_count > 0 {
            println!("  Governor proposals synced: {gov_synced_count}");
            if color::is_color_enabled() {
                println!("  Governor updated:         {}", gov_updated_count.style(color::WARNING));
                println!(
                    "  Governor newly executed:  {}",
                    gov_newly_executed_count.style(color::SUCCESS)
                );
            } else {
                println!("  Governor updated:         {gov_updated_count}");
                println!("  Governor newly executed:  {gov_newly_executed_count}");
            }
            if clean {
                println!("  Governor removed:         {gov_removed_count}");
            }
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
    use chrono::TimeZone;
    use std::{
        collections::HashMap as StdHashMap,
        io::{Read, Write},
        sync::OnceLock,
    };
    use tempfile::TempDir;
    use tokio::sync::{Mutex, MutexGuard};
    use treb_core::types::{GovernorProposal, Operation, Transaction};
    use treb_safe::types::{SafeServiceConfirmation, SafeServiceMultisigResponse};

    fn env_lock_blocking() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).blocking_lock()
    }

    async fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().await
    }

    struct EnvVarGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvVarGuard {
        fn unset(key: &'static str) -> Self {
            let original = std::env::var(key).ok();
            // SAFETY: Serialized by env_lock() in tests that mutate env vars.
            unsafe { std::env::remove_var(key) };
            Self { key, original }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => {
                    // SAFETY: Serialized by env_lock() in tests that mutate env vars.
                    unsafe { std::env::set_var(self.key, value) };
                }
                None => {
                    // SAFETY: Serialized by env_lock() in tests that mutate env vars.
                    unsafe { std::env::remove_var(self.key) };
                }
            }
        }
    }

    fn spawn_chain_id_server(chain_id: u64) -> Option<String> {
        let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return None,
            Err(err) => panic!("failed to bind test RPC listener: {err}"),
        };
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0_u8; 4096];
            let _ = stream.read(&mut buf);

            let body = format!(r#"{{"jsonrpc":"2.0","id":1,"result":"0x{chain_id:x}"}}"#);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
        });

        Some(format!("http://127.0.0.1:{port}"))
    }

    fn sample_transaction(id: &str) -> Transaction {
        Transaction {
            id: id.to_string(),
            chain_id: 1,
            hash: String::new(),
            status: TransactionStatus::Queued,
            block_number: 0,
            sender: "0xSender".into(),
            nonce: 0,
            deployments: vec![],
            operations: vec![Operation {
                operation_type: "CALL".into(),
                target: "0xTarget".into(),
                method: "noop".into(),
                result: StdHashMap::new(),
            }],
            safe_context: None,
            environment: "test".into(),
            created_at: Utc.with_ymd_and_hms(2026, 3, 8, 10, 0, 0).unwrap(),
        }
    }

    fn sample_governor_proposal(proposal_id: &str, tx_id: &str) -> GovernorProposal {
        GovernorProposal {
            proposal_id: proposal_id.to_string(),
            governor_address: "0xGovernor".into(),
            timelock_address: String::new(),
            chain_id: 1,
            status: ProposalStatus::Pending,
            transaction_ids: vec![tx_id.to_string()],
            proposed_by: "0xProposer".into(),
            proposed_at: Utc.with_ymd_and_hms(2026, 3, 8, 10, 5, 0).unwrap(),
            description: String::new(),
            executed_at: None,
            execution_tx_hash: String::new(),
        }
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

    #[test]
    fn expand_rpc_endpoint_reports_missing_env_vars() {
        let _lock = env_lock_blocking();
        let _guard = EnvVarGuard::unset("TREB_SYNC_MISSING_RPC");

        let err = expand_rpc_endpoint("mainnet", "https://rpc.example/${TREB_SYNC_MISSING_RPC}")
            .unwrap_err();

        assert!(err.contains("mainnet"), "got: {err}");
        assert!(err.contains("TREB_SYNC_MISSING_RPC"), "got: {err}");
    }

    #[tokio::test]
    async fn resolve_rpc_urls_expands_dotenv_backed_endpoints() {
        let _lock = env_lock().await;
        let _guard = EnvVarGuard::unset("TREB_SYNC_RPC_URL");

        let tmp = TempDir::new().unwrap();
        let Some(rpc_url) = spawn_chain_id_server(1) else {
            return;
        };
        std::fs::write(
            tmp.path().join("foundry.toml"),
            r#"
[profile.default]
src = "src"
out = "out"

[rpc_endpoints]
mainnet = "${TREB_SYNC_RPC_URL}"
"#,
        )
        .unwrap();
        std::fs::write(tmp.path().join(".env"), format!("TREB_SYNC_RPC_URL={rpc_url}\n")).unwrap();

        let needed_chains = HashSet::from([1_u64]);
        let resolved = resolve_rpc_urls(tmp.path(), &needed_chains, false).await;

        assert!(resolved.warnings.is_empty(), "warnings: {:?}", resolved.warnings);
        assert_eq!(resolved.rpc_map.get(&1), Some(&rpc_url));
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
            governor_synced: 2,
            governor_updated: 1,
            governor_newly_executed: 0,
            governor_removed: 0,
            errors: vec!["some error".into()],
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"synced\":5"));
        assert!(json.contains("\"updated\":3"));
        assert!(json.contains("\"newlyExecuted\":1"));
        assert!(json.contains("\"removed\":0"));
        assert!(json.contains("\"governorSynced\":2"));
        assert!(json.contains("\"governorUpdated\":1"));
        assert!(json.contains("\"governorNewlyExecuted\":0"));
        assert!(json.contains("\"governorRemoved\":0"));
        assert!(json.contains("some error"));
    }

    #[test]
    fn persist_governor_proposal_update_updates_transactions_before_terminal_status() {
        let tmp = TempDir::new().unwrap();
        let mut registry = Registry::init(tmp.path()).unwrap();

        let tx_id = "tx-1";
        let proposal_id = "proposal-1";
        registry.insert_transaction(sample_transaction(tx_id)).unwrap();
        registry.insert_governor_proposal(sample_governor_proposal(proposal_id, tx_id)).unwrap();

        let lock_path = tmp.path().join(".treb").join("transactions.lock");
        if lock_path.exists() {
            std::fs::remove_file(&lock_path).unwrap();
        }
        std::fs::create_dir(&lock_path).unwrap();

        let mut executed = sample_governor_proposal(proposal_id, tx_id);
        executed.status = ProposalStatus::Executed;
        executed.executed_at = Some(Utc.with_ymd_and_hms(2026, 3, 8, 10, 15, 0).unwrap());
        executed.execution_tx_hash = "0xexecuted".into();

        let err = persist_governor_proposal_update(&mut registry, executed, true).unwrap_err();
        let err = err.to_string();
        assert!(err.contains("failed to update transaction"), "got: {err}");

        drop(registry);

        let reopened = Registry::open(tmp.path()).unwrap();
        assert_eq!(
            reopened.get_governor_proposal(proposal_id).unwrap().status,
            ProposalStatus::Pending
        );
        assert_eq!(reopened.get_transaction(tx_id).unwrap().status, TransactionStatus::Queued);
    }
}
