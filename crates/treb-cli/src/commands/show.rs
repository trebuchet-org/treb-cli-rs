//! `treb show` command implementation.

use std::env;

use anyhow::{bail, Context};
use treb_core::types::Deployment;
use treb_registry::Registry;

use crate::commands::resolve::resolve_deployment;
use crate::output;

pub async fn run(deployment_query: &str, json: bool) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;

    if !cwd.join("foundry.toml").exists() {
        bail!(
            "no foundry.toml found in {}\n\n\
             Run `forge init` to create a Foundry project, then `treb init`.",
            cwd.display()
        );
    }
    if !cwd.join(".treb").exists() {
        bail!(
            "project not initialized — .treb/ directory not found in {}\n\n\
             Run `treb init` first.",
            cwd.display()
        );
    }

    let registry = Registry::open(&cwd).context("failed to open registry")?;
    let lookup = registry.load_lookup_index().context("failed to load lookup index")?;
    let deployment = resolve_deployment(deployment_query, &registry, &lookup)?;

    if json {
        output::print_json(deployment)?;
    } else {
        print_deployment_details(deployment);
    }

    Ok(())
}

fn print_deployment_details(d: &Deployment) {
    // Identity
    println!("── Identity ──");
    output::print_kv(&[
        ("ID", &d.id),
        ("Contract", &d.contract_name),
        ("Label", &d.label),
        ("Namespace", &d.namespace),
        ("Type", &d.deployment_type.to_string()),
    ]);

    // On-Chain
    println!("\n── On-Chain ──");
    output::print_kv(&[
        ("Chain ID", &d.chain_id.to_string()),
        ("Address", &d.address),
    ]);

    // Transaction
    println!("\n── Transaction ──");
    output::print_kv(&[
        ("Transaction ID", &d.transaction_id),
        ("Method", &d.deployment_strategy.method.to_string()),
    ]);
    if !d.deployment_strategy.salt.is_empty() {
        output::print_kv(&[("Salt", &d.deployment_strategy.salt)]);
    }
    if !d.deployment_strategy.factory.is_empty() {
        output::print_kv(&[("Factory", &d.deployment_strategy.factory)]);
    }

    // Artifact
    println!("\n── Artifact ──");
    output::print_kv(&[
        ("Path", &d.artifact.path),
        ("Compiler", &d.artifact.compiler_version),
        ("Bytecode Hash", &d.artifact.bytecode_hash),
        ("Script", &d.artifact.script_path),
        ("Git Commit", &d.artifact.git_commit),
    ]);

    // Verification
    println!("\n── Verification ──");
    output::print_kv(&[("Status", &d.verification.status.to_string())]);
    if !d.verification.etherscan_url.is_empty() {
        output::print_kv(&[("Etherscan URL", &d.verification.etherscan_url)]);
    }
    if let Some(ref verified_at) = d.verification.verified_at {
        output::print_kv(&[("Verified At", &verified_at.to_rfc3339())]);
    }

    // Proxy Info (only for proxy deployments)
    if let Some(ref proxy) = d.proxy_info {
        println!("\n── Proxy Info ──");
        output::print_kv(&[
            ("Proxy Type", &proxy.proxy_type),
            ("Implementation", &proxy.implementation),
        ]);
        if !proxy.admin.is_empty() {
            output::print_kv(&[("Admin", &proxy.admin)]);
        }
        if !proxy.history.is_empty() {
            println!("  Upgrade History:");
            for upgrade in &proxy.history {
                println!(
                    "    - {} at {} (tx: {})",
                    upgrade.implementation_id,
                    upgrade.upgraded_at.to_rfc3339(),
                    upgrade.upgrade_tx_id
                );
            }
        }
    }

    // Tags (only when present)
    if let Some(ref tags) = d.tags {
        if !tags.is_empty() {
            println!("\n── Tags ──");
            println!("  {}", tags.join(", "));
        }
    }

    // Timestamps
    println!("\n── Timestamps ──");
    output::print_kv(&[
        ("Created At", &d.created_at.to_rfc3339()),
        ("Updated At", &d.updated_at.to_rfc3339()),
    ]);
}
