//! `treb queued` command — list pending Safe/Governor operations.

use anyhow::Context;
use owo_colors::OwoColorize;
use treb_core::types::{ProposalStatus, TransactionStatus};
use treb_registry::Registry;

use crate::{output, ui::color};

pub async fn queued_command(
    network: Option<String>,
    json: bool,
) -> anyhow::Result<()> {
    let cwd = std::env::current_dir().context("cannot determine working directory")?;
    let treb_dir = cwd.join(".treb");
    if !treb_dir.exists() {
        anyhow::bail!("no .treb directory found — run `treb init` first");
    }

    let registry = Registry::open(&treb_dir)?;

    // Collect queued safe transactions
    let all_safe_txs = registry.list_safe_transactions();
    let queued_safe: Vec<_> = all_safe_txs
        .into_iter()
        .filter(|stx| stx.status == TransactionStatus::Queued)
        .filter(|stx| {
            network.as_ref().is_none_or(|n| {
                n.parse::<u64>().is_ok_and(|id| id == stx.chain_id)
                    || *n == stx.chain_id.to_string()
            })
        })
        .collect();

    // Collect pending governor proposals
    let all_proposals = registry.list_governor_proposals();
    let queued_proposals: Vec<_> = all_proposals
        .into_iter()
        .filter(|p| !matches!(
            p.status,
            ProposalStatus::Executed | ProposalStatus::Canceled | ProposalStatus::Defeated
        ))
        .filter(|p| {
            network.as_ref().is_none_or(|n| {
                n.parse::<u64>().is_ok_and(|id| id == p.chain_id)
                    || *n == p.chain_id.to_string()
            })
        })
        .collect();

    if json {
        let value = serde_json::json!({
            "safeTransactions": queued_safe,
            "governorProposals": queued_proposals,
        });
        output::print_json(&value)?;
        return Ok(());
    }

    if queued_safe.is_empty() && queued_proposals.is_empty() {
        println!("No queued operations.");
        return Ok(());
    }

    let use_color = color::is_color_enabled();
    println!("Queued operations:\n");

    if !queued_safe.is_empty() {
        if use_color {
            println!("  {}", "SAFE TRANSACTIONS".style(color::BOLD));
        } else {
            println!("  SAFE TRANSACTIONS");
        }
        for stx in &queued_safe {
            let hash_short = truncate_hash(&stx.safe_tx_hash);
            let tx_count = stx.transactions.len();
            let age = format_age(&stx.proposed_at);
            let fork_sim = if stx.fork_executed_at.is_some() { " [simulated]" } else { "" };
            println!(
                "  {}  safeTxHash={}  nonce={}  {} tx{}  proposed {}{}",
                stx.proposed_by,
                hash_short,
                stx.nonce,
                tx_count,
                if tx_count == 1 { "" } else { "s" },
                age,
                fork_sim,
            );
        }
        println!();
    }

    if !queued_proposals.is_empty() {
        if use_color {
            println!("  {}", "GOVERNANCE PROPOSALS".style(color::BOLD));
        } else {
            println!("  GOVERNANCE PROPOSALS");
        }
        for p in &queued_proposals {
            let id_short = truncate_hash(&p.proposal_id);
            let tx_count = p.transaction_ids.len();
            let age = format_age(&p.proposed_at);
            let fork_sim = if p.fork_executed_at.is_some() { " [simulated]" } else { "" };
            println!(
                "  proposalId={}  {} tx{}  proposed {}{}",
                id_short,
                tx_count,
                if tx_count == 1 { "" } else { "s" },
                age,
                fork_sim,
            );
            if !p.governor_address.is_empty() || !p.timelock_address.is_empty() {
                let mut detail = format!("  Governor {}", truncate_hash(&p.governor_address));
                if !p.timelock_address.is_empty() {
                    detail.push_str(&format!(" -> Timelock {}", truncate_hash(&p.timelock_address)));
                }
                if use_color {
                    println!("  {}", detail.style(color::GRAY));
                } else {
                    println!("  {detail}");
                }
            }
        }
    }

    Ok(())
}

fn truncate_hash(s: &str) -> String {
    if s.len() >= 10 {
        format!("{}...{}", &s[..6], &s[s.len() - 4..])
    } else {
        s.to_string()
    }
}

fn format_age(dt: &chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let dur = now.signed_duration_since(*dt);
    let secs = dur.num_seconds().max(0);
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}
