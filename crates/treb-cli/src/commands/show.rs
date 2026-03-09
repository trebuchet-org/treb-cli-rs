//! `treb show` command implementation.

use std::{collections::HashMap, env};

use anyhow::{Context, bail};
use owo_colors::{OwoColorize, Style};
use treb_core::types::{Deployment, VerificationStatus, contract_display_name};
use treb_registry::Registry;

use crate::{
    commands::resolve::resolve_deployment,
    output,
    ui::{badge, color, selector::fuzzy_select_deployment_id},
};

/// Lookup table for resolving proxy implementation addresses to display names.
type ImplNameLookup = HashMap<(String, u64, String), String>;

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
    let deployments = registry.list_deployments();
    let impl_lookup = build_impl_name_lookup(&deployments);

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
        print_deployment_details(deployment, &impl_lookup);
    }

    Ok(())
}

/// Conditionally apply an owo-colors [`Style`] to text.
///
/// Returns the styled string when color is enabled, plain text otherwise.
fn styled(text: &str, style: Style) -> String {
    if color::is_color_enabled() { format!("{}", text.style(style)) } else { text.to_string() }
}

/// Print the deployment header: `Deployment: {id}` in cyan bold + 80-char `=` divider.
fn print_deployment_header(id: &str, fork_badge: Option<&str>) {
    let header = match fork_badge {
        Some(badge) => format!("Deployment: {} {}", id, styled(badge, color::FORK_BADGE)),
        None => format!("Deployment: {}", id),
    };
    println!("{}", styled(&header, color::STAGE));
    println!("{}", "=".repeat(80));
}

/// Print a plain-text section header: `\nSection Name:\n`.
fn print_section(title: &str) {
    println!("\n{}:", title);
}

/// Print a 2-space-indented key-value field: `  Key: Value`.
fn print_field(key: &str, value: &str) {
    println!("  {}: {}", key, value);
}

fn build_impl_name_lookup(deployments: &[&Deployment]) -> ImplNameLookup {
    deployments
        .iter()
        .map(|d| {
            let display_name = contract_display_name(&d.contract_name, &d.label);
            ((d.namespace.to_lowercase(), d.chain_id, d.address.to_lowercase()), display_name)
        })
        .collect()
}

fn resolve_proxy_implementation_display(
    namespace: &str,
    chain_id: u64,
    implementation: &str,
    impl_lookup: &ImplNameLookup,
) -> String {
    let key = (namespace.to_lowercase(), chain_id, implementation.to_lowercase());

    match impl_lookup.get(&key) {
        Some(display_name) => {
            let styled_name = styled(display_name, Style::new().yellow().bold());
            format!("{styled_name} at {implementation}")
        }
        None => implementation.to_string(),
    }
}

fn print_deployment_details(d: &Deployment, impl_lookup: &ImplNameLookup) {
    // Header: Deployment: {id} [fork]
    let fork = badge::fork_badge(&d.namespace);
    print_deployment_header(&d.id, fork.as_deref());

    // Basic Information (Go: Identity + On-Chain merged)
    print_section("Basic Information");
    let display_name = contract_display_name(&d.contract_name, &d.label);
    let contract_styled = styled(&display_name, color::YELLOW);
    print_field("Contract", &contract_styled);
    print_field("Address", &d.address);
    let type_str = d.deployment_type.to_string();
    print_field("Type", &type_str);
    print_field("Namespace", &d.namespace);
    print_field("Network", &d.chain_id.to_string());
    if !d.label.is_empty() {
        let label_styled = styled(&d.label, Style::new().magenta());
        print_field("Label", &label_styled);
    }

    // Deployment Strategy (Go: Transaction)
    print_section("Deployment Strategy");
    print_field("Method", &d.deployment_strategy.method.to_string());
    if !d.deployment_strategy.factory.is_empty() {
        print_field("Factory", &d.deployment_strategy.factory);
    }
    let zero_hash = "0x0000000000000000000000000000000000000000000000000000000000000000";
    if !d.deployment_strategy.salt.is_empty() && d.deployment_strategy.salt != zero_hash {
        print_field("Salt", &d.deployment_strategy.salt);
    }
    if !d.deployment_strategy.entropy.is_empty() {
        print_field("Entropy", &d.deployment_strategy.entropy);
    }
    if !d.deployment_strategy.init_code_hash.is_empty() {
        print_field("InitCodeHash", &d.deployment_strategy.init_code_hash);
    }

    // Proxy Information (only for proxy deployments)
    if let Some(ref proxy) = d.proxy_info {
        print_section("Proxy Information");
        print_field("Type", &proxy.proxy_type);
        let implementation = resolve_proxy_implementation_display(
            &d.namespace,
            d.chain_id,
            &proxy.implementation,
            impl_lookup,
        );
        print_field("Implementation", &implementation);
        if !proxy.admin.is_empty() {
            print_field("Admin", &proxy.admin);
        }
        if !proxy.history.is_empty() {
            println!("  Upgrade History:");
            for (i, upgrade) in proxy.history.iter().enumerate() {
                println!(
                    "    {}. {} (upgraded at {})",
                    i + 1,
                    upgrade.implementation_id,
                    upgrade.upgraded_at.format("%Y-%m-%d %H:%M:%S"),
                );
            }
        }
    }

    // Artifact Information (Go: Artifact)
    print_section("Artifact Information");
    print_field("Path", &d.artifact.path);
    print_field("Compiler", &d.artifact.compiler_version);
    if !d.artifact.bytecode_hash.is_empty() {
        print_field("BytecodeHash", &d.artifact.bytecode_hash);
    }
    if !d.artifact.script_path.is_empty() {
        print_field("Script", &d.artifact.script_path);
    }
    if !d.artifact.git_commit.is_empty() {
        print_field("GitCommit", &d.artifact.git_commit);
    }

    // Verification Status (Go: Verification)
    print_section("Verification Status");
    let status_str = d.verification.status.to_string();
    let status_style = match d.verification.status {
        VerificationStatus::Verified => color::VERIFIED,
        _ => color::NOT_VERIFIED,
    };
    print_field("Status", &styled(&status_str, status_style));
    if !d.verification.etherscan_url.is_empty() {
        print_field("Etherscan", &d.verification.etherscan_url);
    }
    if let Some(ref verified_at) = d.verification.verified_at {
        print_field(
            "Verified At",
            &verified_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        );
    }

    // Tags (only when present)
    if let Some(ref tags) = d.tags {
        if !tags.is_empty() {
            print_section("Tags");
            for tag in tags {
                println!("  - {}", tag);
            }
        }
    }

    // Timestamps
    print_section("Timestamps");
    print_field(
        "Created",
        &d.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
    );
    print_field(
        "Updated",
        &d.updated_at.format("%Y-%m-%d %H:%M:%S").to_string(),
    );
}
