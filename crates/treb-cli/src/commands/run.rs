//! `treb run` command implementation.

use std::collections::HashMap;
use std::env;

use anyhow::{bail, Context};
use treb_config::{resolve_config, ResolveOpts};
use treb_forge::sender::resolve_all_senders;

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
    env_vars: Vec<String>,
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

    // Stub: the pipeline construction and execution will be implemented in US-003.
    // For now, print a summary of the resolved context.
    let _ = (
        script,
        sig,
        args,
        broadcast,
        dry_run,
        slow,
        legacy,
        verify,
        debug,
        json,
        env_vars,
        target_contract,
        non_interactive,
        effective_rpc_url,
        resolved_senders,
        resolved,
    );

    println!("run: config resolved for {} (pipeline not yet wired)", script);

    Ok(())
}
