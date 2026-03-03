//! `treb run` command implementation.

use std::collections::HashMap;
use std::env;
use std::path::PathBuf;

use anyhow::{bail, Context};
use serde::Serialize;
use treb_config::{resolve_config, ResolveOpts};
use treb_forge::pipeline::{PipelineConfig, PipelineContext, PipelineResult, RunPipeline};
use treb_forge::pipeline::resolve_git_commit;
use treb_forge::script::build_script_config_with_senders;
use treb_forge::sender::resolve_all_senders;
use treb_registry::Registry;

use crate::output;

const FOUNDRY_TOML: &str = "foundry.toml";
const TREB_DIR: &str = ".treb";

/// Ensure the project is initialized (foundry.toml + .treb/ exist).
fn ensure_initialized(cwd: &std::path::Path) -> anyhow::Result<()> {
    if !cwd.join(FOUNDRY_TOML).exists() {
        bail!(
            "no foundry.toml found in {}\n\n\
             Run `forge init` to create a Foundry project, then `treb init`.",
            cwd.display()
        );
    }
    if !cwd.join(TREB_DIR).exists() {
        bail!(
            "project not initialized — .treb/ directory not found in {}\n\n\
             Run `treb init` first.",
            cwd.display()
        );
    }
    Ok(())
}

/// Execute a deployment script.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    script: &str,
    sig: &str,
    args: Vec<String>,
    network: Option<String>,
    rpc_url: Option<String>,
    namespace: Option<String>,
    broadcast: bool,
    dry_run: bool,
    slow: bool,
    legacy: bool,
    verify: bool,
    verbose: bool,
    debug: bool,
    json: bool,
    _env_vars: Vec<String>,
    target_contract: Option<String>,
    non_interactive: bool,
) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    ensure_initialized(&cwd)?;

    // ── Config resolution ────────────────────────────────────────────────
    let resolved = resolve_config(ResolveOpts {
        project_root: cwd.clone(),
        namespace,
        network: network.clone(),
        profile: None,
        sender_overrides: HashMap::new(),
    })
    .context("failed to resolve configuration")?;

    // If --rpc-url is provided, it overrides the network-derived RPC URL.
    let effective_rpc_url = rpc_url.or_else(|| resolved.network.clone());

    // ── Sender resolution ────────────────────────────────────────────────
    let resolved_senders = resolve_all_senders(&resolved.senders)
        .await
        .context("failed to resolve senders")?;

    // ── Verbose context output ───────────────────────────────────────────
    if verbose && !json {
        eprintln!("Config source: {}", resolved.config_source);
        eprintln!("Namespace: {}", resolved.namespace);
        if let Some(ref url) = effective_rpc_url {
            eprintln!("RPC: {}", url);
        }
        if let Some(sender) = resolved_senders.get("deployer") {
            eprintln!("Sender: {:?}", sender.sender_address());
        }
    }

    // ── Build ScriptConfig with all CLI flags ────────────────────────────
    let mut script_config =
        build_script_config_with_senders(&resolved, script, &resolved_senders)
            .context("failed to build script configuration")?;

    script_config
        .sig(sig)
        .args(args)
        .broadcast(broadcast)
        .dry_run(dry_run)
        .slow(slow || resolved.slow)
        .legacy(legacy)
        .verify(verify)
        .debug(debug)
        .non_interactive(non_interactive);

    if let Some(ref tc) = target_contract {
        script_config.target_contract(tc);
    }

    // --rpc-url overrides the network-derived URL
    if let Some(ref url) = effective_rpc_url {
        script_config.rpc_url(url);
    }

    // ── Build PipelineConfig and PipelineContext ─────────────────────────
    let pipeline_config = PipelineConfig {
        script_path: script.to_string(),
        dry_run,
        namespace: resolved.namespace.clone(),
        script_sig: sig.to_string(),
        script_args: Vec::new(), // args already in ScriptConfig
        ..Default::default()
    };

    let git_commit = resolve_git_commit();

    let pipeline_context = PipelineContext {
        config: pipeline_config,
        script_path: PathBuf::from(script),
        git_commit,
        project_root: cwd.clone(),
    };

    // ── Open registry and execute pipeline ───────────────────────────────
    let mut registry = Registry::open(&cwd).context("failed to open registry")?;

    let pipeline = RunPipeline::new(pipeline_context).with_script_config(script_config);
    let result = pipeline
        .execute(&mut registry)
        .await
        .context("pipeline execution failed")?;

    // ── Display results ──────────────────────────────────────────────────
    display_result(&result, json)?;

    Ok(())
}

// ── JSON output type ─────────────────────────────────────────────────────

#[derive(Serialize)]
struct RunOutputJson {
    success: bool,
    dry_run: bool,
    deployments: Vec<DeploymentJson>,
    transactions: Vec<TransactionJson>,
    skipped: Vec<SkippedJson>,
    console_logs: Vec<String>,
}

#[derive(Serialize)]
struct DeploymentJson {
    id: String,
    contract_name: String,
    address: String,
    namespace: String,
    chain_id: u64,
    deployment_type: String,
}

#[derive(Serialize)]
struct TransactionJson {
    id: String,
    hash: String,
    status: String,
}

#[derive(Serialize)]
struct SkippedJson {
    contract_name: String,
    address: String,
    reason: String,
}

// ── Display logic ────────────────────────────────────────────────────────

fn display_result(result: &PipelineResult, json: bool) -> anyhow::Result<()> {
    if json {
        display_result_json(result)?;
    } else {
        display_result_human(result);
    }
    Ok(())
}

fn display_result_json(result: &PipelineResult) -> anyhow::Result<()> {
    let output = RunOutputJson {
        success: result.success,
        dry_run: result.dry_run,
        deployments: result
            .deployments
            .iter()
            .map(|rd| DeploymentJson {
                id: rd.deployment.id.clone(),
                contract_name: rd.deployment.contract_name.clone(),
                address: rd.deployment.address.clone(),
                namespace: rd.deployment.namespace.clone(),
                chain_id: rd.deployment.chain_id,
                deployment_type: rd.deployment.deployment_type.to_string(),
            })
            .collect(),
        transactions: result
            .transactions
            .iter()
            .map(|rt| TransactionJson {
                id: rt.transaction.id.clone(),
                hash: rt.transaction.hash.clone(),
                status: rt.transaction.status.to_string(),
            })
            .collect(),
        skipped: result
            .skipped
            .iter()
            .map(|s| SkippedJson {
                contract_name: s.deployment.contract_name.clone(),
                address: s.deployment.address.clone(),
                reason: s.reason.clone(),
            })
            .collect(),
        console_logs: result.console_logs.clone(),
    };

    output::print_json(&output)?;
    Ok(())
}

fn display_result_human(result: &PipelineResult) {
    // Dry-run banner
    if result.dry_run {
        println!("[DRY RUN] No changes were written to the registry.");
        println!();
    }

    // Console.log output
    if !result.console_logs.is_empty() {
        for log in &result.console_logs {
            println!("{}", log);
        }
        println!();
    }

    // Deployment table
    if !result.deployments.is_empty() {
        let mut table = output::build_table(&[
            "Contract",
            "Address",
            "Type",
            "Namespace",
            "Chain",
        ]);

        for rd in &result.deployments {
            let d = &rd.deployment;
            table.add_row(vec![
                d.contract_name.as_str(),
                &d.address,
                &d.deployment_type.to_string(),
                d.namespace.as_str(),
                &d.chain_id.to_string(),
            ]);
        }

        output::print_table(&table);
        println!();
    }

    // Skipped deployments
    if !result.skipped.is_empty() {
        println!("Skipped:");
        for s in &result.skipped {
            println!(
                "  {} ({}) — {}",
                s.deployment.contract_name, s.deployment.address, s.reason
            );
        }
        println!();
    }

    // Collisions
    if !result.collisions.is_empty() {
        println!("Collisions detected: {}", result.collisions.len());
        println!();
    }

    // Summary
    let dep_count = result.deployments.len();
    let tx_count = result.transactions.len();
    let skip_count = result.skipped.len();

    if dep_count == 0 && skip_count == 0 {
        println!("No deployments found in script output.");
    } else {
        let action = if result.dry_run { "would be recorded" } else { "recorded" };
        let mut parts = Vec::new();
        if dep_count > 0 {
            parts.push(format!(
                "{} deployment{} {}",
                dep_count,
                if dep_count == 1 { "" } else { "s" },
                action
            ));
        }
        if tx_count > 0 {
            parts.push(format!(
                "{} transaction{}",
                tx_count,
                if tx_count == 1 { "" } else { "s" }
            ));
        }
        if skip_count > 0 {
            parts.push(format!(
                "{} skipped",
                skip_count
            ));
        }
        println!("{}", parts.join(", "));
    }
}
