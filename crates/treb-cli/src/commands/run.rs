//! `treb run` command implementation.

use std::collections::HashMap;
use std::env;
use std::path::PathBuf;

use anyhow::{bail, Context};
use treb_config::{resolve_config, ResolveOpts};
use treb_forge::pipeline::{PipelineConfig, PipelineContext, RunPipeline};
use treb_forge::script::build_script_config_with_senders;
use treb_forge::sender::resolve_all_senders;
use treb_forge::pipeline::resolve_git_commit;
use treb_registry::Registry;

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

    // Stub: result display will be implemented in US-004.
    let _ = (json, verbose, result);

    Ok(())
}
