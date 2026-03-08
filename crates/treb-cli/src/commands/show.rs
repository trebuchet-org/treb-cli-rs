//! `treb show` command implementation.

use std::env;

use anyhow::{Context, bail};
use owo_colors::{OwoColorize, Style};
use treb_core::types::Deployment;
use treb_registry::Registry;

use crate::{
    commands::resolve::resolve_deployment,
    output,
    ui::{badge, color, selector::fuzzy_select_deployment_id},
};

/// Verifier display order with human-readable labels (matches badge::VERIFIER_ORDER).
const VERIFIER_DISPLAY_ORDER: [(&str, &str); 3] =
    [("etherscan", "Etherscan"), ("sourcify", "Sourcify"), ("blockscout", "Blockscout")];

pub async fn run(deployment_query: Option<String>, json: bool) -> anyhow::Result<()> {
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

    let query = match deployment_query {
        Some(q) => q,
        None => {
            let deployments: Vec<_> = registry.list_deployments().into_iter().cloned().collect();
            fuzzy_select_deployment_id(&deployments)
                .map_err(|e| anyhow::anyhow!("{e}"))?
                .ok_or_else(|| anyhow::anyhow!("no deployment selected"))?
        }
    };

    let deployment = resolve_deployment(&query, &registry, &lookup)?;

    if json {
        output::print_json(deployment)?;
    } else {
        print_deployment_details(deployment);
    }

    Ok(())
}

/// Conditionally apply an owo-colors [`Style`] to text.
///
/// Returns the styled string when color is enabled, plain text otherwise.
fn styled(text: &str, style: Style) -> String {
    if color::is_color_enabled() { format!("{}", text.style(style)) } else { text.to_string() }
}

/// Print a section header with optional STAGE styling.
fn print_header(title: &str) {
    println!("{}", styled(&format!("── {title} ──"), color::STAGE));
}

/// Style a verification status value according to its meaning.
fn styled_verification_status(status: &str) -> String {
    let style = match status.to_uppercase().as_str() {
        "VERIFIED" => color::VERIFIED,
        "FAILED" => color::FAILED,
        _ => color::UNVERIFIED,
    };
    styled(status, style)
}

fn print_deployment_details(d: &Deployment) {
    // Identity
    print_header("Identity");
    let ns_display = match badge::fork_badge(&d.namespace) {
        Some(fb) => format!("{} {}", d.namespace, styled(&fb, color::FORK_BADGE)),
        None => d.namespace.clone(),
    };
    let type_str = d.deployment_type.to_string();
    let type_styled =
        styled(&type_str, color::style_for_deployment_type(d.deployment_type.clone()));
    output::print_kv(&[
        ("ID", &d.id),
        ("Contract", &d.contract_name),
        ("Label", &d.label),
        ("Namespace", &ns_display),
        ("Type", &type_styled),
    ]);

    // On-Chain
    println!();
    print_header("On-Chain");
    let addr_styled = styled(&d.address, color::ADDRESS);
    output::print_kv(&[("Chain ID", &d.chain_id.to_string()), ("Address", &addr_styled)]);

    // Transaction
    println!();
    print_header("Transaction");
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
    println!();
    print_header("Artifact");
    output::print_kv(&[
        ("Path", &d.artifact.path),
        ("Compiler", &d.artifact.compiler_version),
        ("Bytecode Hash", &d.artifact.bytecode_hash),
        ("Script", &d.artifact.script_path),
        ("Git Commit", &d.artifact.git_commit),
    ]);

    // Verification
    println!();
    print_header("Verification");
    if d.verification.verifiers.is_empty() {
        let status = styled_verification_status("UNVERIFIED");
        output::print_kv(&[("Status", &status)]);
    } else {
        let mut pairs: Vec<(String, String)> = Vec::new();
        for (key, label) in VERIFIER_DISPLAY_ORDER {
            if let Some(vs) = d.verification.verifiers.get(key) {
                let status_styled = styled_verification_status(&vs.status);
                let mut detail = status_styled;
                if !vs.url.is_empty() {
                    detail.push_str(&format!(" {}", vs.url));
                }
                if !vs.reason.is_empty() {
                    detail.push_str(&format!(" — {}", vs.reason));
                }
                pairs.push((label.to_string(), detail));
            }
        }
        let kv_refs: Vec<(&str, &str)> =
            pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        output::print_kv(&kv_refs);
    }
    if let Some(ref verified_at) = d.verification.verified_at {
        output::print_kv(&[("Verified At", &verified_at.to_rfc3339())]);
    }

    // Proxy Info (only for proxy deployments)
    if let Some(ref proxy) = d.proxy_info {
        println!();
        print_header("Proxy Info");
        let impl_styled = styled(&proxy.implementation, color::ADDRESS);
        output::print_kv(&[("Proxy Type", &proxy.proxy_type), ("Implementation", &impl_styled)]);
        if !proxy.admin.is_empty() {
            let admin_styled = styled(&proxy.admin, color::ADDRESS);
            output::print_kv(&[("Admin", &admin_styled)]);
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
            println!();
            print_header("Tags");
            println!("  {}", tags.join(", "));
        }
    }

    // Timestamps
    println!();
    print_header("Timestamps");
    output::print_kv(&[
        ("Created At", &d.created_at.to_rfc3339()),
        ("Updated At", &d.updated_at.to_rfc3339()),
    ]);
}
