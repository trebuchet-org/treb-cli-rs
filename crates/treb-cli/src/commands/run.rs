//! `treb run` command implementation.

use std::{
    collections::HashMap,
    env,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, bail};
use foundry_common::{
    Shell as FoundryShell,
    shell::{ColorChoice, OutputFormat, OutputMode, Verbosity},
};
use owo_colors::{OwoColorize, Style};
use serde::Serialize;
use treb_config::{ResolveOpts, resolve_config};
#[cfg(test)]
use treb_core::types::Operation;
use treb_forge::{
    pipeline::{
        BroadcastHook, PipelineConfig, PipelineContext,
        PipelineResult, RecordedTransaction, ScriptEntry, SessionPhase,
        SessionPipeline, SessionProgressCallback, resolve_git_commit,
    },
    script::build_script_config_with_senders,
    sender::{ResolvedSender, resolve_all_senders},
    sender_config::encode_sender_configs,
};
use treb_registry::{ForkStateStore, Registry};

use crate::{
    output,
    ui::{color, emoji, interactive::is_non_interactive, selector::fuzzy_select_network},
};

pub(crate) const FOUNDRY_TOML: &str = "foundry.toml";
pub(crate) const TREB_DIR: &str = ".treb";


/// Parse a `KEY=VALUE` environment variable string.
///
/// Returns `(key, value)` where value may be empty. The split is on the
/// first `=` only, so `KEY=value=with=equals` parses correctly.
pub fn parse_env_var(s: &str) -> anyhow::Result<(&str, &str)> {
    let Some(eq_pos) = s.find('=') else {
        bail!("invalid --env value '{}': expected KEY=VALUE format (missing '=')", s);
    };
    let key = &s[..eq_pos];
    let value = &s[eq_pos + 1..];
    if key.is_empty() {
        bail!("invalid --env value '{}': key cannot be empty", s);
    }
    Ok((key, value))
}

#[cfg(test)]
fn format_verbose_sender(role: &str, sender: &ResolvedSender) -> String {
    match sender {
        ResolvedSender::Governor { governor_address, timelock_address, proposer, .. } => {
            let timelock = timelock_address
                .map(|address| address.to_string())
                .unwrap_or_else(|| "none".to_string());
            format!(
                "{}: governor={} timelock={} proposer={}",
                role,
                governor_address,
                timelock,
                proposer.sender_address()
            )
        }
        _ => format!("{}: {}", role, sender.sender_address()),
    }
}

fn format_verbose_senders(resolved_senders: &HashMap<String, ResolvedSender>) -> Vec<String> {
    let max_role_len = resolved_senders.keys().map(|k| k.len()).max().unwrap_or(0);
    let mut senders: Vec<String> = resolved_senders
        .iter()
        .map(|(role, sender)| format_verbose_sender_padded(role, sender, max_role_len))
        .collect();
    senders.sort();
    senders
}

fn format_verbose_sender_padded(role: &str, sender: &ResolvedSender, pad: usize) -> String {
    match sender {
        ResolvedSender::Governor { governor_address, timelock_address, proposer, .. } => {
            let timelock = timelock_address
                .map(|address| address.to_string())
                .unwrap_or_else(|| "none".to_string());
            format!(
                "{:<pad$}  governor={} timelock={} proposer={}",
                role,
                governor_address,
                timelock,
                proposer.sender_address()
            )
        }
        _ => format!("{:<pad$}  {}", role, sender.sender_address()),
    }
}

fn sorted_env_var_entries(env_vars: &[String]) -> Vec<(String, String)> {
    let mut entries: Vec<(String, String)> = env_vars
        .iter()
        .map(|pair| match parse_env_var(pair) {
            Ok((key, value)) => (key.to_string(), value.to_string()),
            Err(_) => (pair.clone(), String::new()),
        })
        .collect();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}

/// Extract an env var name from a `${VAR}` template string.
pub(crate) fn extract_env_var_name(template: &str) -> Option<&str> {
    let s = template.strip_prefix("${")?.strip_suffix('}')?;
    if s.is_empty() { None } else { Some(s) }
}

fn env_var_is_truthy(name: &str) -> bool {
    env::var(name).ok().is_some_and(|value| {
        matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
    })
}

fn active_fork_matches(
    entry: &treb_core::types::fork::ForkEntry,
    cwd: &std::path::Path,
    network: Option<&str>,
    effective_rpc_url: Option<&str>,
) -> bool {
    if entry.rpc_url.is_empty() {
        return false;
    }

    if effective_rpc_url
        .and_then(|target| resolve_rpc_url_for_chain_id(target, cwd))
        .as_deref()
        .is_some_and(|target| target == entry.rpc_url)
    {
        return true;
    }

    network.is_some_and(|net| net == entry.network)
        && !entry.env_var_name.is_empty()
        && env::var(&entry.env_var_name).ok().as_deref() == Some(entry.rpc_url.as_str())
}

fn is_active_fork_run(
    cwd: &std::path::Path,
    network: Option<&str>,
    effective_rpc_url: Option<&str>,
) -> bool {
    if env_var_is_truthy("TREB_FORK_MODE") {
        return true;
    }

    let mut store = ForkStateStore::new(&cwd.join(TREB_DIR));
    if store.load().is_err() {
        return false;
    }

    store
        .list_active_forks()
        .into_iter()
        .any(|entry| active_fork_matches(entry, cwd, network, effective_rpc_url))
}

pub(crate) fn deployment_banner_mode(_dry_run: bool, broadcast: bool, _active_fork: bool) -> (&'static str, Style) {
    if broadcast {
        ("BROADCAST", color::GREEN)
    } else {
        ("DRY_RUN", color::YELLOW)
    }
}

/// Inject `--env KEY=VALUE` pairs into the process environment.
///
/// Must be called before config resolution so that env vars are available
/// for `${VAR}` expansion in config files.
pub fn inject_env_vars(env_vars: &[String]) -> anyhow::Result<()> {
    for pair in env_vars {
        let (key, value) = parse_env_var(pair)?;
        // SAFETY: this is single-threaded CLI code; no concurrent env access.
        unsafe { env::set_var(key, value) };
    }
    Ok(())
}

fn should_prompt_for_broadcast_confirmation(
    broadcast: bool,
    dry_run: bool,
    prompts_enabled: bool,
) -> bool {
    broadcast && !dry_run && prompts_enabled
}

/// Resolve the effective network: CLI flag > local config > interactive picker.
pub fn resolve_network(
    cli_network: Option<String>,
    cwd: &std::path::Path,
    prompts_enabled: bool,
    non_interactive: bool,
) -> anyhow::Result<Option<String>> {
    // 1. Explicit CLI flag wins.
    if cli_network.is_some() {
        return Ok(cli_network);
    }

    // 2. Check local config for a saved default network.
    let local = treb_config::load_local_config(cwd).unwrap_or_default();
    if !local.network.is_empty() {
        return Ok(Some(local.network));
    }

    // 3. Interactive picker as last resort.
    if prompts_enabled {
        let endpoints = treb_config::resolve_rpc_endpoints(cwd)
            .map_err(|e| anyhow::anyhow!("failed to load foundry config: {e}"))?;
        let mut names: Vec<String> = endpoints.keys().cloned().collect();
        names.sort();
        return fuzzy_select_network(&names, non_interactive)
            .map_err(|e| anyhow::anyhow!("{e}"))
            .map(|opt| opt.map(|s| s.to_string()));
    }

    Ok(None)
}

fn should_reject_interactive_json_broadcast(
    broadcast: bool,
    dry_run: bool,
    json: bool,
    prompts_enabled: bool,
) -> bool {
    json && should_prompt_for_broadcast_confirmation(broadcast, dry_run, prompts_enabled)
}

/// Restores Foundry's global shell after temporarily silencing it.
///
/// `forge-script` writes compilation and broadcast chatter through this shell.
/// `treb run --json` needs stdout reserved for the final machine-readable
/// payload, so suppress Foundry's shell output while the pipeline runs.
pub(crate) struct FoundryShellGuard {
    output_format: OutputFormat,
    output_mode: OutputMode,
    color_choice: ColorChoice,
    verbosity: Verbosity,
}

impl FoundryShellGuard {
    pub(crate) fn suppress() -> Self {
        let mut shell = FoundryShell::get();
        let previous = Self {
            output_format: shell.output_format(),
            output_mode: shell.output_mode(),
            color_choice: shell.color_choice(),
            verbosity: shell.verbosity(),
        };
        // Suppress output but preserve color choice so yansi colors
        // stay enabled for trace rendering.
        *shell = FoundryShell::new_with(
            previous.output_format,
            OutputMode::Quiet,
            previous.color_choice,
            0,
        );
        previous
    }
}

impl Drop for FoundryShellGuard {
    fn drop(&mut self) {
        *FoundryShell::get() = FoundryShell::new_with(
            self.output_format,
            self.output_mode,
            self.color_choice,
            self.verbosity,
        );
    }
}

/// Ensure the project is initialized (foundry.toml + .treb/ exist).
pub fn ensure_initialized(cwd: &std::path::Path) -> anyhow::Result<()> {
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

/// Resolve a network name to an RPC URL. If the input is already a URL, returns it directly.
/// Falls back to looking up the name in foundry.toml [rpc_endpoints].
pub(crate) fn resolve_rpc_url_for_chain_id(
    network_or_url: &str,
    cwd: &std::path::Path,
) -> Option<String> {
    if network_or_url.starts_with("http://") || network_or_url.starts_with("https://") {
        return Some(network_or_url.to_string());
    }
    let endpoints = treb_config::resolve_rpc_endpoints(cwd).ok()?;
    let endpoint = endpoints.get(network_or_url)?;
    if endpoint.unresolved || endpoint.expanded_url.trim().is_empty() {
        return None;
    }
    Some(endpoint.expanded_url.clone())
}

/// Fetch the chain ID from an RPC endpoint via `eth_chainId`.
pub(crate) async fn fetch_chain_id(rpc_url: &str) -> anyhow::Result<u64> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("failed to build HTTP client")?;

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "eth_chainId",
        "params": [],
        "id": 1
    });

    let resp: serde_json::Value = client
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .context("eth_chainId request failed")?
        .json()
        .await
        .context("invalid eth_chainId response")?;

    let hex = resp.get("result").and_then(|r| r.as_str()).unwrap_or("0x0");
    let stripped = hex.strip_prefix("0x").or_else(|| hex.strip_prefix("0X")).unwrap_or(hex);
    Ok(u64::from_str_radix(stripped, 16).unwrap_or(0))
}

// ── Shared script execution ──────────────────────────────────────────────

/// Options for [`execute_script`].
pub struct ExecuteScriptOpts {
    pub script: String,
    pub sig: String,
    pub args: Vec<String>,
    pub target_contract: Option<String>,
    pub broadcast: bool,
    pub dry_run: bool,
    pub verbose: u8,
    pub json: bool,
    pub slow: bool,
    pub legacy: bool,
    pub verify: bool,
    pub non_interactive: bool,
    /// Already-resolved effective RPC URL (after fork override).
    pub effective_rpc_url: Option<String>,
    /// Already-resolved effective network name.
    pub effective_network: Option<String>,
    /// Whether an active fork is detected.
    pub is_fork: bool,
    /// Resume a previous run, skipping already-completed transactions.
    pub resume: bool,
    /// Optional broadcast confirmation hook.
    pub broadcast_hook: Option<BroadcastHook>,
    /// Source identifier for fork run snapshots (set when `is_fork` is true).
    pub fork_run_source: Option<treb_core::types::fork::ForkRunSource>,
}

/// Execute a single script through the full pipeline.
///
/// Shared execution path used by both `treb run` and `treb compose`.
/// Handles chain ID resolution, pipeline construction with full sender
/// context, shell suppression, and progress spinner.
pub async fn execute_script(
    opts: ExecuteScriptOpts,
    resolved: &treb_config::ResolvedConfig,
    resolved_senders: &mut HashMap<String, ResolvedSender>,
    cwd: &std::path::Path,
    registry: &mut Registry,
) -> anyhow::Result<PipelineResult> {
    let is_safe = resolved_senders.get("deployer").is_some_and(|s| s.is_safe());
    let is_gov = resolved_senders.get("deployer").is_some_and(|s| s.is_governor());

    // Build ScriptConfig (all wallet keys injected)
    let mut script_config =
        build_script_config_with_senders(resolved, &opts.script, resolved_senders)
            .context("failed to build script configuration")?;

    script_config
        .sig(&opts.sig)
        .args(opts.args.clone())
        .broadcast(opts.broadcast && !is_safe && !is_gov)
        .dry_run(opts.dry_run)
        .slow(opts.slow || resolved.slow)
        .legacy(opts.legacy)
        .verify(opts.verify)
        .non_interactive(opts.non_interactive);

    if let Some(ref tc) = opts.target_contract {
        script_config.target_contract(tc);
    }
    if let Some(ref url) = opts.effective_rpc_url {
        script_config.rpc_url(url);
    }

    // Resolve chain ID from RPC
    let chain_id = if let Some(ref network_or_url) = opts.effective_rpc_url {
        let actual_url = resolve_rpc_url_for_chain_id(network_or_url, cwd);
        if let Some(url) = actual_url { fetch_chain_id(&url).await.unwrap_or(0) } else { 0 }
    } else {
        0
    };

    // Build PipelineConfig + PipelineContext
    let pipeline_config = PipelineConfig {
        script_path: opts.script.clone(),
        broadcast: opts.broadcast,
        namespace: resolved.namespace.clone(),
        chain_id,
        script_sig: opts.sig.clone(),
        script_args: Vec::new(),
        verbosity: opts.verbose,
        is_fork: opts.is_fork,
        rpc_url: opts.effective_rpc_url.clone(),
        quiet: opts.json,
        ..Default::default()
    };

    let sender_role_names: Vec<String> = resolved_senders.keys().cloned().collect();
    let sender_labels = resolved_senders
        .iter()
        .map(|(role, sender)| (sender.broadcast_address(), role.clone()))
        .collect();

    // Take ownership of all resolved senders for the pipeline context.
    let owned_senders = std::mem::take(resolved_senders);

    let pipeline_context = PipelineContext {
        config: pipeline_config,
        script_path: PathBuf::from(&opts.script),
        git_commit: resolve_git_commit(),
        project_root: cwd.to_path_buf(),
        resolved_senders: owned_senders,
        sender_configs: resolved.senders.clone(),
        sender_labels,
        sender_role_names,
    };

    // Build SessionPipeline with a single script entry
    let script_name = PathBuf::from(&opts.script)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| opts.script.clone());

    let mut session = SessionPipeline::new();
    session.add_script(ScriptEntry {
        name: script_name.clone(),
        context: pipeline_context,
        config: script_config,
    });

    if opts.resume {
        session = session.with_resume(true);
    }

    // Wire progress spinner
    let wants_broadcast = opts.broadcast && !opts.dry_run && !is_safe && !is_gov;
    let spinner: Arc<Mutex<Option<spinoff::Spinner>>> = Arc::new(Mutex::new(None));
    if !opts.json {
        let is_broadcast = wants_broadcast;
        let spinner_ref = spinner.clone();
        let progress_cb: SessionProgressCallback = Box::new(move |phase| {
            let mut guard = spinner_ref.lock().unwrap();
            if let Some(mut s) = guard.take() {
                s.clear();
            }
            let msg = match phase {
                SessionPhase::Compiling => Some("Compiling".to_string()),
                SessionPhase::Simulating(_) if is_broadcast => Some("Simulating".to_string()),
                SessionPhase::Broadcasting(_) => Some("Broadcasting".to_string()),
                _ => None,
            };
            if let Some(msg) = msg {
                *guard = Some(spinoff::Spinner::new(
                    spinoff::spinners::Dots2,
                    msg,
                    spinoff::Color::Cyan,
                ));
            }
        });
        session = session.with_progress(progress_cb);
    }

    // Phase 1: Simulate
    let simulated = {
        let _foundry_shell = FoundryShellGuard::suppress();
        let r = session.simulate_all(registry).await;
        drop(spinner.lock().unwrap().take());
        eprint!("\x1b[2K\r");
        match r {
            Ok(s) => s,
            Err((_partial, _name, e)) => return Err(anyhow::anyhow!(e)),
        }
    };

    // Phase 2: Preview simulation results → confirm → broadcast
    let has_transactions = simulated.results().any(|(_, r)| !r.transactions.is_empty());
    let should_broadcast = if wants_broadcast && has_transactions {
        opts.broadcast_hook
            .is_none_or(|hook| {
                let (_, result) = simulated.results().next().unwrap();
                hook(&result.transactions)
            })
    } else {
        false
    };

    if should_broadcast {
        // Pre-broadcast fork snapshot (if in fork mode)
        if opts.is_fork {
            if let Some(ref source) = opts.fork_run_source {
                let treb_dir = cwd.join(TREB_DIR);
                let mut store = ForkStateStore::new(&treb_dir);
                if store.load().is_ok() && store.is_fork_mode_active() {
                    super::fork::snapshot_fork_before_broadcast(
                        &treb_dir, &mut store, source.clone(),
                    )
                    .await
                    .ok(); // best-effort
                }
            }
        }

        let spinner2: Arc<Mutex<Option<spinoff::Spinner>>> = Arc::new(Mutex::new(None));
        if !opts.json {
            let spinner_ref = spinner2.clone();
            *spinner_ref.lock().unwrap() = Some(spinoff::Spinner::new(
                spinoff::spinners::Dots2,
                "Broadcasting",
                spinoff::Color::Cyan,
            ));
        }

        let result = {
            let _foundry_shell = FoundryShellGuard::suppress();
            let r = simulated.broadcast_all(registry).await;
            drop(spinner2.lock().unwrap().take());
            eprint!("\x1b[2K\r");
            match r {
                Ok(results) => results
                    .into_iter()
                    .next()
                    .map(|sr| sr.result)
                    .ok_or_else(|| anyhow::anyhow!("pipeline returned no results")),
                Err((_partial, _name, e)) => Err(anyhow::anyhow!(e)),
            }
        }?;

        // Update deployment/transaction counts on the fork snapshot
        if opts.is_fork {
            let treb_dir = cwd.join(TREB_DIR);
            let mut store = ForkStateStore::new(&treb_dir);
            if store.load().is_ok() && store.is_fork_mode_active() {
                if let Some(last) = store.data_mut().run_snapshots.last_mut() {
                    last.deployment_count = result.deployments.len();
                    last.transaction_count = result.transactions.len();
                }
                let _ = store.save();
            }
        }

        Ok(result)
    } else {
        Ok(simulated.into_results().into_iter().next()
            .map(|sr| sr.result)
            .ok_or_else(|| anyhow::anyhow!("pipeline returned no results"))?)
    }
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
    verbose: u8,
    dump_command: bool,
    resume: bool,
    json: bool,
    env_vars: Vec<String>,
    target_contract: Option<String>,
    non_interactive: bool,
) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    ensure_initialized(&cwd)?;

    // ── Inject environment variables (before config resolution) ──────────
    inject_env_vars(&env_vars)?;

    let prompts_enabled = !is_non_interactive(non_interactive);

    if should_reject_interactive_json_broadcast(broadcast, dry_run, json, prompts_enabled) {
        bail!(
            "interactive broadcast confirmation is not available in JSON mode; rerun with --non-interactive"
        );
    }

    // ── Network resolution: CLI flag > local config > interactive prompt ──
    let network = resolve_network(network, &cwd, prompts_enabled, non_interactive)?;

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
    let mut effective_rpc_url = rpc_url.or_else(|| resolved.network.clone());

    // ── Derive effective network name from CLI flag or resolved config ──
    let effective_network = network.clone().or_else(|| resolved.network.clone());

    // ── Fork override: swap RPC URL to the fork's Anvil instance ────────
    let active_fork = {
        let net = effective_network.as_deref();
        let mut store = ForkStateStore::new(&cwd.join(TREB_DIR));
        if store.load().is_ok() {
            net.and_then(|n| store.get_active_fork(n).cloned())
        } else {
            None
        }
    };
    if let Some(ref fork_entry) = active_fork {
        effective_rpc_url = Some(fork_entry.rpc_url.clone());
        // Override the RPC env var so that foundry.toml alias resolution
        // (in forge's load_config_and_evm_opts) resolves to the Anvil URL
        // instead of the upstream mainnet URL.
        //
        // Extract the env var name from the foundry.toml raw_url template
        // (e.g., "${CELO_RPC_URL}" → "CELO_RPC_URL") and set it to the
        // fork's Anvil URL.
        if let Some(ref net) = effective_network {
            if let Ok(endpoints) = treb_config::resolve_rpc_endpoints(&cwd) {
                if let Some(endpoint) = endpoints.get(net.as_str()) {
                    if let Some(var) = extract_env_var_name(&endpoint.raw_url) {
                        unsafe { env::set_var(var, &fork_entry.rpc_url) };
                    }
                }
            }
        }
    }

    // ── Sender resolution ────────────────────────────────────────────────
    let mut resolved_senders =
        resolve_all_senders(&resolved.senders).await.context("failed to resolve senders")?;

    // ── Inject treb context env vars for Solidity consumption ──────────
    // SAFETY: this is single-threaded CLI code; no concurrent env access.
    unsafe { env::set_var("NAMESPACE", &resolved.namespace) };
    if let Some(ref net) = effective_network {
        unsafe { env::set_var("NETWORK", net) };
    }
    let encoded_senders = encode_sender_configs(&resolved_senders);
    unsafe { env::set_var("SENDER_CONFIGS", &encoded_senders) };

    // ── Dump command and exit ─────────────────────────────────────────
    if dump_command {
        let script_config =
            build_script_config_with_senders(&resolved, script, &resolved_senders)
                .context("failed to build script configuration")?;
        let cmd_parts = script_config.to_forge_command();
        let cmd_str = cmd_parts
            .iter()
            .map(|p| {
                if p.contains(' ') || p.contains('"') {
                    format!("'{}'", p.replace('\'', "'\\''"))
                } else {
                    p.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!("{}", cmd_str);
        return Ok(());
    }

    // ── Auto-fund senders on fork ──────────────────────────────────────
    if active_fork.is_some() {
        if let Some(ref rpc) = effective_rpc_url {
            let fund_results =
                treb_forge::fund_senders_on_fork(rpc, &resolved_senders, 10_000).await;
            let funded_count = fund_results.iter().filter(|(_, _, ok)| *ok).count();
            if funded_count > 0 && !json {
                eprintln!(
                    "Funded {} sender address{} on fork",
                    funded_count,
                    if funded_count == 1 { "" } else { "es" },
                );
            }
        }
    }

    let is_safe = resolved_senders.get("deployer").is_some_and(|s| s.is_safe());
    let is_gov = resolved_senders.get("deployer").is_some_and(|s| s.is_governor());

    // ── Resolve chain ID from RPC ──────────────────────────────────────
    let chain_id = if let Some(ref network_or_url) = effective_rpc_url {
        let actual_url = resolve_rpc_url_for_chain_id(network_or_url, &cwd);
        if let Some(url) = actual_url { fetch_chain_id(&url).await.unwrap_or(0) } else { 0 }
    } else {
        0
    };

    // ── Pre-execution banner (Go: PrintDeploymentBanner) ──────────────
    if !json {
        let separator: String = "─".repeat(50);
        let use_color = color::is_color_enabled();
        let is_fork = active_fork.is_some()
            || is_active_fork_run(&cwd, network.as_deref(), effective_rpc_url.as_deref());

        // Header
        if use_color {
            println!("{}", separator.style(color::GRAY));
        } else {
            println!("{separator}");
        }

        // Script
        if use_color {
            println!("  {:10} {}", "Script:", script.style(color::CYAN));
        } else {
            println!("  {:10} {}", "Script:", script);
        }

        // Network
        let network_name = resolved.network.as_deref().unwrap_or("(none)");
        let fork_tag = if is_fork { " [fork]" } else { "" };
        if use_color {
            let chain_suffix = if chain_id > 0 {
                format!(" {}", format!("({})", chain_id).style(color::GRAY))
            } else {
                String::new()
            };
            let fork_suffix = if is_fork {
                format!(" {}", "[fork]".style(color::MAGENTA))
            } else {
                String::new()
            };
            println!(
                "  {:10} {}{}{}",
                "Network:",
                network_name.style(color::BLUE),
                chain_suffix,
                fork_suffix,
            );
        } else if chain_id > 0 {
            println!("  {:10} {} ({}){}", "Network:", network_name, chain_id, fork_tag);
        } else {
            println!("  {:10} {}{}", "Network:", network_name, fork_tag);
        }

        // Namespace
        if use_color {
            println!("  {:10} {}", "Namespace:", resolved.namespace.style(color::MAGENTA));
        } else {
            println!("  {:10} {}", "Namespace:", resolved.namespace);
        }

        // Mode
        let (mode_label, mode_style) = deployment_banner_mode(dry_run, broadcast, is_fork);
        if use_color {
            println!("  {:10} {}", "Mode:", mode_label.style(mode_style));
        } else {
            println!("  {:10} {}", "Mode:", mode_label);
        }

        // Env Vars (only when present)
        let sorted_env_vars = sorted_env_var_entries(&env_vars);
        if !sorted_env_vars.is_empty() {
            for (i, (key, value)) in sorted_env_vars.iter().enumerate() {
                let formatted = if use_color {
                    format!("{}={}", key.style(color::YELLOW), value.style(color::GREEN))
                } else {
                    format!("{key}={value}")
                };
                if i == 0 {
                    println!("  {:10} {}", "Env Vars:", formatted);
                } else {
                    println!("  {:10} {}", "", formatted);
                }
            }
        }

        // Senders
        let sender_lines = format_verbose_senders(&resolved_senders);
        for (i, line) in sender_lines.iter().enumerate() {
            let label = if i == 0 { "Senders:" } else { "" };
            if use_color {
                println!("  {:10} {}", label, line.style(color::GRAY));
            } else {
                println!("  {:10} {}", label, line);
            }
        }

        // Bottom separator
        if use_color {
            println!("{}", separator.style(color::GRAY));
        } else {
            println!("{separator}");
        }
    }

    // ── Open registry and execute pipeline ───────────────────────────────
    let mut registry = Registry::open(&cwd).context("failed to open registry")?;

    let wants_broadcast = broadcast && !dry_run && !is_safe && !is_gov;
    let broadcast_previewed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    // Build broadcast confirmation hook for interactive mode
    let broadcast_hook = if wants_broadcast && prompts_enabled && !json {
        let previewed = broadcast_previewed.clone();
        let hook: BroadcastHook = Box::new(move |transactions: &[RecordedTransaction]| {
            display_transactions_ordered(transactions, 0);
            let confirmed = crate::ui::prompt::confirm("Broadcast these transactions?", false);
            if confirmed {
                previewed.store(true, std::sync::atomic::Ordering::Relaxed);
                println!();
            }
            confirmed
        });
        Some(hook)
    } else {
        None
    };

    let mut result = execute_script(
        ExecuteScriptOpts {
            script: script.to_string(),
            sig: sig.to_string(),
            args,
            target_contract,
            broadcast,
            dry_run,
            verbose,
            json,
            slow,
            legacy,
            verify,
            non_interactive,
            effective_rpc_url,
            effective_network,
            is_fork: active_fork.is_some(),
            resume,
            broadcast_hook,
            fork_run_source: if active_fork.is_some() {
                Some(treb_core::types::fork::ForkRunSource::Run {
                    script: script.to_string(),
                })
            } else {
                None
            },
        },
        &resolved,
        &mut resolved_senders,
        &cwd,
        &mut registry,
    )
    .await?;

    // ── Inline queued execution handling ────────────────────────────────
    if !result.queued_executions.is_empty() && !json {
        handle_queued_executions(
            &mut result, chain_id, prompts_enabled,
            active_fork.is_some(), &cwd, &mut registry,
        ).await?;
    } else if !result.proposed_results.is_empty() && prompts_enabled && !json {
        // Fallback: old-style polling for live mode (no queued items)
        poll_proposed_results(&mut result, chain_id, &mut registry).await?;
    }

    // ── Display results ──────────────────────────────────────────────────
    let skip_traces = broadcast_previewed.load(std::sync::atomic::Ordering::Relaxed);
    let script_name = std::path::Path::new(script)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| script.to_string());
    display_result(
        &result, json, verbose, resolved.network.as_deref(), &resolved.namespace, skip_traces,
        &script_name,
    )?;

    Ok(())
}

// ── Inline queued execution ──────────────────────────────────────────────

/// Handle queued executions inline after broadcast.
///
/// For each `QueuedExecution` item, prints a status line and optionally
/// prompts the user to simulate execution on fork.
async fn handle_queued_executions(
    result: &mut PipelineResult,
    chain_id: u64,
    prompts_enabled: bool,
    is_fork: bool,
    cwd: &std::path::Path,
    registry: &mut treb_registry::Registry,
) -> anyhow::Result<()> {
    use treb_forge::pipeline::QueuedExecution;

    let queued = std::mem::take(&mut result.queued_executions);

    for item in &queued {
        match item {
            QueuedExecution::SafeProposal { safe_address, safe_tx_hash, nonce, inner_txs } => {
                let safe_display = output::truncate_address(&format!("{:#x}", safe_address));
                let hash_display = output::truncate_address(&format!("{:#x}", safe_tx_hash));

                if color::is_color_enabled() {
                    eprintln!(
                        "  {} queued  Safe {} (safeTxHash={}, nonce={}, {} tx)",
                        emoji::HOURGLASS,
                        safe_display,
                        hash_display.style(color::YELLOW),
                        nonce,
                        inner_txs.len(),
                    );
                } else {
                    eprintln!(
                        "  {} queued  Safe {} (safeTxHash={}, nonce={}, {} tx)",
                        emoji::HOURGLASS, safe_display, hash_display, nonce, inner_txs.len(),
                    );
                }

                if is_fork && prompts_enabled {
                    let simulate = crate::ui::prompt::confirm(
                        "  Simulate Safe execution on fork?", true,
                    );
                    if simulate {
                        let fork_rpc = resolve_fork_rpc(cwd, chain_id)?;
                        match treb_forge::pipeline::fork_routing::exec_safe_from_registry(
                            &fork_rpc,
                            &format!("{:#x}", safe_address),
                            chain_id,
                            &inner_txs.iter().map(|tx| {
                                treb_core::types::safe_transaction::SafeTxData {
                                    to: format!("{:#x}", tx.to),
                                    value: format!("{}", tx.value),
                                    data: format!("0x{}", tx.data.iter().map(|b| format!("{b:02x}")).collect::<String>()),
                                    operation: 0,
                                }
                            }).collect::<Vec<_>>(),
                        ).await {
                            Ok(receipts) => {
                                let all_ok = receipts.iter().all(|r| r.status);
                                if all_ok {
                                    if color::is_color_enabled() {
                                        eprintln!(
                                            "  {} {}  executed Safe {} ({} tx, {} batch)",
                                            emoji::CHECK_MARK.style(color::GREEN),
                                            "simulated".style(color::GREEN),
                                            safe_display,
                                            inner_txs.len(),
                                            receipts.len(),
                                        );
                                    } else {
                                        eprintln!(
                                            "  {} simulated  executed Safe {} ({} tx, {} batch)",
                                            emoji::CHECK_MARK, safe_display, inner_txs.len(), receipts.len(),
                                        );
                                    }
                                    // Update fork_executed_at in registry
                                    for safe_tx in &mut result.safe_transactions {
                                        if safe_tx.safe_tx_hash == format!("{:#x}", safe_tx_hash) {
                                            safe_tx.fork_executed_at = Some(chrono::Utc::now());
                                            let _ = registry.update_safe_transaction(safe_tx.clone());
                                        }
                                    }
                                } else {
                                    eprintln!("  Safe simulation reverted on fork");
                                }
                            }
                            Err(e) => {
                                eprintln!("  Safe simulation failed: {e}");
                            }
                        }
                    } else {
                        eprintln!("  Saved as queued — execute later via `treb fork exec`");
                    }
                } else if is_fork {
                    // Non-interactive fork: save as queued
                    eprintln!("  Saved as queued — execute later via `treb fork exec`");
                }
                // Live mode: already proposed to Safe TX Service
            }

            QueuedExecution::GovernanceProposal {
                governor_address, timelock_address, actions, proposal_description: _,
            } => {
                let gov_display = output::truncate_address(&format!("{:#x}", governor_address));
                let tl_display = timelock_address
                    .map(|tl| format!(" via Timelock {}", output::truncate_address(&format!("{:#x}", tl))))
                    .unwrap_or_default();

                if color::is_color_enabled() {
                    eprintln!(
                        "  {} queued  Governor {}{} ({} action{})",
                        emoji::HOURGLASS,
                        gov_display.style(color::YELLOW),
                        tl_display,
                        actions.len(),
                        if actions.len() == 1 { "" } else { "s" },
                    );
                } else {
                    eprintln!(
                        "  {} queued  Governor {}{} ({} action{})",
                        emoji::HOURGLASS, gov_display, tl_display,
                        actions.len(),
                        if actions.len() == 1 { "" } else { "s" },
                    );
                }

                if is_fork && prompts_enabled {
                    let simulate = crate::ui::prompt::confirm(
                        "  Simulate governance execution on fork?", true,
                    );
                    if simulate {
                        let fork_rpc = resolve_fork_rpc(cwd, chain_id)?;
                        let governance_addr = timelock_address.unwrap_or(*governor_address);
                        let targets: Vec<_> = actions.iter()
                            .map(|a| a.target)
                            .collect();
                        let values: Vec<_> = actions.iter()
                            .map(|a| a.value)
                            .collect();
                        let calldatas: Vec<Vec<u8>> = actions.iter()
                            .map(|a| a.calldata.clone())
                            .collect();

                        match treb_forge::pipeline::fork_routing::simulate_governance_on_fork(
                            &treb_forge::pipeline::fork_routing::AnvilRpc::new(&fork_rpc),
                            &targets,
                            &values,
                            &calldatas,
                            governance_addr,
                        ).await {
                            Ok(receipts) => {
                                let all_ok = receipts.iter().all(|r| r.status);
                                if all_ok {
                                    if color::is_color_enabled() {
                                        eprintln!(
                                            "  {} {}  executed Governor{} ({} action{})",
                                            emoji::CHECK_MARK.style(color::GREEN),
                                            "simulated".style(color::GREEN),
                                            tl_display,
                                            actions.len(),
                                            if actions.len() == 1 { "" } else { "s" },
                                        );
                                    } else {
                                        eprintln!(
                                            "  {} simulated  executed Governor{} ({} action{})",
                                            emoji::CHECK_MARK, tl_display,
                                            actions.len(),
                                            if actions.len() == 1 { "" } else { "s" },
                                        );
                                    }
                                    // Update fork_executed_at on governor proposals
                                    for proposal in &mut result.governor_proposals {
                                        if proposal.governor_address == format!("{:#x}", governor_address) {
                                            proposal.fork_executed_at = Some(chrono::Utc::now());
                                            let _ = registry.update_governor_proposal(proposal.clone());
                                        }
                                    }
                                } else {
                                    eprintln!("  Governance simulation reverted on fork");
                                }
                            }
                            Err(e) => {
                                eprintln!("  Governance simulation failed: {e}");
                            }
                        }
                    } else {
                        eprintln!("  Saved as queued — execute later via `treb fork exec`");
                    }
                } else if is_fork {
                    eprintln!("  Saved as queued — execute later via `treb fork exec`");
                }
                // Live mode: governance takes time, just record
            }
        }
    }

    Ok(())
}

/// Resolve the fork RPC URL from the fork state store.
fn resolve_fork_rpc(cwd: &std::path::Path, chain_id: u64) -> anyhow::Result<String> {
    let treb_dir = cwd.join(TREB_DIR);
    let mut store = treb_registry::ForkStateStore::new(&treb_dir);
    store.load().map_err(|e| anyhow::anyhow!("failed to load fork state: {e}"))?;
    let fork = store.data().forks.values()
        .find(|f| f.chain_id == chain_id)
        .ok_or_else(|| anyhow::anyhow!("no active fork for chain {chain_id}"))?;
    Ok(format!("http://127.0.0.1:{}", fork.port))
}

// ── Safe execution polling (legacy, for live mode) ───────────────────────

/// Interactively poll for Safe transaction execution.
///
/// For each `SafeProposed` result, prompts the user to wait. If confirmed,
/// polls the Safe Transaction Service every 5 seconds until executed.
/// Updates the registry records on execution.
async fn poll_proposed_results(
    result: &mut PipelineResult,
    chain_id: u64,
    registry: &mut treb_registry::Registry,
) -> anyhow::Result<()> {
    use treb_core::types::TransactionStatus;

    for pr in &result.proposed_results {
        if let treb_forge::pipeline::RunResult::SafeProposed {
            safe_tx_hash, safe_address, nonce, tx_count,
        } = &pr.run_result
        {
            let hash_display = output::truncate_address(&format!("{:#x}", safe_tx_hash));
            let safe_display = output::truncate_address(&format!("{:#x}", safe_address));

            println!(
                "\n  {} {}  Proposed to Safe {} (safeTxHash={}, nonce={}, {} tx)",
                emoji::HOURGLASS, pr.sender_role, safe_display, hash_display, nonce, tx_count,
            );

            let wait = crate::ui::prompt::confirm("Wait for execution?", false);
            if !wait {
                println!("  Saved as queued — check status later with `treb safe status`");
                continue;
            }

            // Start spinner + poll
            let mut spinner = spinoff::Spinner::new(
                spinoff::spinners::Dots2,
                "  Waiting for Safe execution (polling every 5s)...".to_string(),
                spinoff::Color::Cyan,
            );

            let poll_result = treb_forge::pipeline::routing::poll_safe_execution(
                chain_id,
                safe_tx_hash,
                || false, // never skip — user explicitly asked to wait
            )
            .await;

            spinner.clear();
            eprint!("\x1b[2K\r");

            match poll_result {
                Ok(Some(tx_hash)) => {
                    if color::is_color_enabled() {
                        println!(
                            "  {} {}  Executed! tx={}",
                            emoji::CHECK_MARK,
                            pr.sender_role,
                            tx_hash.style(color::GREEN),
                        );
                    } else {
                        println!(
                            "  {} {}  Executed! tx={}",
                            emoji::CHECK_MARK, pr.sender_role, tx_hash,
                        );
                    }

                    // Update safe transaction status in registry
                    for safe_tx in &mut result.safe_transactions {
                        if safe_tx.safe_tx_hash == format!("{:#x}", safe_tx_hash) {
                            safe_tx.status = TransactionStatus::Executed;
                            safe_tx.execution_tx_hash = tx_hash.clone();
                            safe_tx.executed_at = Some(chrono::Utc::now());
                            let _ = registry.update_safe_transaction(safe_tx.clone());
                        }
                    }
                }
                Ok(None) => {
                    println!("  Skipped — saved as queued");
                }
                Err(e) => {
                    eprintln!("  Polling failed: {e}");
                }
            }
        }
    }

    Ok(())
}

// ── JSON output type ─────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RunOutputJson {
    success: bool,
    dry_run: bool,
    deployments: Vec<DeploymentJson>,
    transactions: Vec<TransactionJson>,
    skipped: Vec<SkippedJson>,
    gas_used: u64,
    console_logs: Vec<String>,
    governor_proposals: Vec<GovernorProposalJson>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeploymentJson {
    id: String,
    contract_name: String,
    address: String,
    namespace: String,
    chain_id: u64,
    deployment_type: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TransactionJson {
    id: String,
    hash: String,
    status: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SkippedJson {
    contract_name: String,
    address: String,
    reason: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GovernorProposalJson {
    proposal_id: String,
    governor_address: String,
    timelock_address: String,
    status: String,
    transaction_ids: Vec<String>,
}

// ── Display logic ────────────────────────────────────────────────────────

fn display_result(
    result: &PipelineResult,
    json: bool,
    verbose: u8,
    network: Option<&str>,
    namespace: &str,
    skip_traces: bool,
    script_name: &str,
) -> anyhow::Result<()> {
    if json {
        display_result_json(result)?;
    } else {
        display_result_human(result, verbose, network, namespace, skip_traces, script_name);
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
        gas_used: result.gas_used,
        console_logs: result.console_logs.clone(),
        governor_proposals: result
            .governor_proposals
            .iter()
            .map(|gp| GovernorProposalJson {
                proposal_id: gp.proposal_id.clone(),
                governor_address: gp.governor_address.clone(),
                timelock_address: gp.timelock_address.clone(),
                status: gp.status.to_string(),
                transaction_ids: gp.transaction_ids.clone(),
            })
            .collect(),
    };

    output::print_json(&output)?;
    Ok(())
}

#[cfg(test)]
fn registry_update_message(
    result: &PipelineResult,
    network: Option<&str>,
    namespace: &str,
) -> Option<(String, bool)> {
    if result.dry_run || !result.success {
        return None;
    }

    let network_display = network.unwrap_or("unknown");
    let updated = result.registry_updated;
    let msg = if updated {
        format!(
            "{} Updated registry for {} network in namespace {}",
            emoji::CHECK_MARK,
            network_display,
            namespace,
        )
    } else {
        format!(
            "- No registry changes recorded for {} network in namespace {}",
            network_display, namespace,
        )
    };

    Some((msg, updated))
}

fn format_warning_section_header_with_style(title: &str, style_enabled: bool) -> String {
    let title = format!("{title}:");
    if style_enabled {
        format!("{}  {}", emoji::WARNING, title.style(color::WARNING))
    } else {
        format!("{}  {}", emoji::WARNING, title)
    }
}

fn format_warning_section_header(title: &str) -> String {
    format_warning_section_header_with_style(title, color::is_color_enabled())
}

fn print_warning_section_header(title: &str, separator_width: usize) {
    let separator = "─".repeat(separator_width);
    println!("\n{}", format_warning_section_header(title));
    if color::is_color_enabled() {
        println!("{}", separator.style(color::GRAY));
    } else {
        println!("{separator}");
    }
}

fn collision_metadata_lines(collision: &treb_forge::events::ExtractedCollision) -> Vec<String> {
    let mut lines = Vec::new();
    if !collision.label.is_empty() {
        lines.push(format!("    Label: {}", collision.label));
    }
    if !collision.entropy.is_empty() {
        lines.push(format!("    Entropy: {}", collision.entropy));
    }
    lines
}

// ---------------------------------------------------------------------------
// Broadcast preview (shown before confirmation prompt)
// ---------------------------------------------------------------------------

/// Display transactions grouped by sender, with traces and optional footers.
///
/// Used for both the broadcast preview and the post-execution results.
/// Display transactions in script execution order (not grouped by sender).
///
/// Each transaction is numbered and shows its sender, with optional trace
/// and footer. Used by both `treb run` and `treb compose`.
pub(crate) fn display_transactions_ordered(transactions: &[RecordedTransaction], verbose: u8) {
    if transactions.is_empty() {
        return;
    }

    let use_color = color::is_color_enabled();

    // Count by category for header.
    // Governor/Safe txs are grouped into proposals — count unique sender roles,
    // not individual txs, so "2 governor txs from the same sender" = 1 proposal.
    let mut wallet_count = 0usize;
    let mut governor_tx_count = 0usize;
    let mut safe_tx_count = 0usize;
    let mut governor_senders = std::collections::HashSet::new();
    let mut safe_senders = std::collections::HashSet::new();
    for rt in transactions {
        match rt.sender_category {
            Some(treb_forge::SenderCategory::Governor) => {
                governor_tx_count += 1;
                governor_senders.insert(rt.sender_name.as_deref().unwrap_or(""));
            }
            Some(treb_forge::SenderCategory::Safe) => {
                safe_tx_count += 1;
                safe_senders.insert(rt.sender_name.as_deref().unwrap_or(""));
            }
            _ => wallet_count += 1,
        }
    }
    let mut header_parts: Vec<String> = Vec::new();
    if wallet_count > 0 {
        let s = if wallet_count == 1 { "" } else { "s" };
        header_parts.push(format!("{wallet_count} transaction{s}"));
    }
    if !governor_senders.is_empty() {
        let n = governor_senders.len();
        let s = if n == 1 { "" } else { "s" };
        let inner = if governor_tx_count == 1 {
            "1 transaction".into()
        } else {
            format!("{governor_tx_count} transactions")
        };
        header_parts.push(format!("{n} governance proposal{s} ({inner})"));
    }
    if !safe_senders.is_empty() {
        let n = safe_senders.len();
        let s = if n == 1 { "" } else { "s" };
        let inner = if safe_tx_count == 1 {
            "1 transaction".into()
        } else {
            format!("{safe_tx_count} transactions")
        };
        header_parts.push(format!("{n} safe execution{s} ({inner})"));
    }
    let header = format!("\n{}:", header_parts.join(", "));
    if use_color {
        println!("{}", header.style(color::BOLD));
    } else {
        println!("{header}");
    }

    for (i, rt) in transactions.iter().enumerate() {
        let sender_label = tx_sender_label(rt);

        // Category tag for non-wallet senders
        let category_tag = match rt.sender_category {
            Some(treb_forge::SenderCategory::Governor) => " [governor]",
            Some(treb_forge::SenderCategory::Safe) => " [safe]",
            _ => "",
        };

        // Transaction header: index + sender + category
        if use_color {
            println!(
                "\n  {} {}{}",
                format!("{i}:").style(color::GRAY),
                format!("from={sender_label}").style(color::CYAN),
                category_tag.style(color::YELLOW),
            );
        } else {
            println!("\n  {i}: from={sender_label}{category_tag}");
        }

        // Per-transaction decoded trace sub-tree
        if verbose == 0 {
            if let Some(ref trace) = rt.trace {
                for line in trace.lines() {
                    println!("  {line}");
                }
            } else {
                // Fallback: show operations when no trace is available
                for op in &rt.transaction.operations {
                    let target = output::truncate_address(&op.target);
                    let line = if op.method.is_empty() || op.method.starts_with("0x") {
                        format!("      {} {}", op.operation_type, target)
                    } else {
                        format!("      {} {}.{}()", op.operation_type, target, op.method)
                    };
                    if use_color {
                        println!("{}", line.style(color::MUTED));
                    } else {
                        println!("{line}");
                    }
                }
            }
        }

        // Footer: hash, block, gas
        let footer = format_tx_footer(rt);
        if !footer.is_empty() {
            println!("    {footer}");
        }
    }
    println!();
}

/// Display broadcast summary for a single script result.
///
/// Renders the compose-style format used by both `treb run` and `treb compose`:
/// ```text
///   ✓ Deploy  (1 tx, 1 deployed)
///     deployer 0xbde3... gas=577982
/// ```
pub(super) fn display_script_broadcast_summary(
    name: &str,
    result: &PipelineResult,
) {
    if result.transactions.is_empty() {
        return;
    }

    let use_color = color::is_color_enabled();
    let has_proposed = !result.proposed_results.is_empty();
    let dep_count = result.deployments.len();

    // Count transactions by category, grouping governor/safe into proposals
    let mut wallet_count = 0usize;
    let mut governor_senders = std::collections::HashSet::new();
    let mut safe_senders = std::collections::HashSet::new();
    for rt in &result.transactions {
        match rt.sender_category {
            Some(treb_forge::SenderCategory::Governor) => {
                governor_senders.insert(rt.sender_name.as_deref().unwrap_or(""));
            }
            Some(treb_forge::SenderCategory::Safe) => {
                safe_senders.insert(rt.sender_name.as_deref().unwrap_or(""));
            }
            _ => wallet_count += 1,
        }
    }

    // Status icon
    let status_icon = if has_proposed {
        if use_color {
            format!("{}", emoji::HOURGLASS.style(color::YELLOW))
        } else {
            emoji::HOURGLASS.to_string()
        }
    } else if use_color {
        format!("{}", emoji::CHECK_MARK.style(color::GREEN))
    } else {
        emoji::CHECK_MARK.to_string()
    };

    // Summary line: "1 tx, 1 governor, 1 deployed"
    let mut parts: Vec<String> = Vec::new();
    if wallet_count > 0 {
        parts.push(format!("{wallet_count} tx"));
    }
    if !governor_senders.is_empty() {
        parts.push(format!("{} governor", governor_senders.len()));
    }
    if !safe_senders.is_empty() {
        parts.push(format!("{} safe", safe_senders.len()));
    }
    if dep_count > 0 {
        parts.push(format!("{dep_count} deployed"));
    }
    if has_proposed {
        parts.push("proposed".into());
    }
    let detail = if parts.is_empty() {
        format!("{} tx", result.transactions.len())
    } else {
        parts.join(", ")
    };
    if use_color {
        eprintln!(
            "  {} {}  ({})",
            status_icon,
            name,
            detail.style(color::GRAY),
        );
    } else {
        eprintln!("  {} {}  ({})", status_icon, name, detail);
    }

    // Per-transaction detail lines
    for rt in &result.transactions {
        let tx = &rt.transaction;
        let sender_label = tx_sender_label(rt);
        let hash_display = &tx.hash;
        let gas_display = rt.gas_used
            .map(|g| format!(" gas={g}"))
            .unwrap_or_default();
        let block_display = if tx.block_number > 0 {
            format!(" block={}", tx.block_number)
        } else {
            String::new()
        };
        let line = format!(
            "    {sender_label} {hash_display}{block_display}{gas_display}",
        );
        if use_color {
            eprintln!("{}", line.style(color::GRAY));
        } else {
            eprintln!("{line}");
        }
    }

    // Proposed summary lines
    for pr in &result.proposed_results {
        match &pr.run_result {
            treb_forge::pipeline::RunResult::SafeProposed {
                safe_tx_hash, nonce, ..
            } => {
                let hash = output::truncate_address(&format!("{:#x}", safe_tx_hash));
                let line = format!(
                    "{} proposed to Safe (safeTxHash={}, nonce={})",
                    pr.sender_role, hash, nonce,
                );
                if use_color {
                    eprintln!("    {}", line.style(color::YELLOW));
                } else {
                    eprintln!("    {line}");
                }
            }
            treb_forge::pipeline::RunResult::GovernorProposed {
                proposal_id, governor_address, ..
            } => {
                let gov = output::truncate_address(&format!("{:#x}", governor_address));
                let line = format!(
                    "{} proposed to Governor {} (proposal={})",
                    pr.sender_role, gov, proposal_id,
                );
                if use_color {
                    eprintln!("    {}", line.style(color::YELLOW));
                } else {
                    eprintln!("    {line}");
                }
            }
            _ => {}
        }
    }
}



#[cfg(test)]
fn format_tx_operation(operation: &Operation) -> String {
    let target = if color::is_color_enabled() {
        format!("{}", output::truncate_address(&operation.target).style(color::CYAN))
    } else {
        output::truncate_address(&operation.target)
    };
    if operation.method.is_empty() || operation.method.starts_with("0x") {
        // No decoded method or raw selector — just show type + target
        format!("{} {}", operation.operation_type, target)
    } else {
        format!("{} {}.{}()", operation.operation_type, target, operation.method)
    }
}

/// Format a summary of all operations for display after the `→` arrow.
#[cfg(test)]
fn format_tx_operations(operations: &[Operation]) -> String {
    operations.iter().map(format_tx_operation).collect::<Vec<_>>().join(" | ")
}

/// Build a pipe-separated gray footer line with hash, block number (only
/// non-empty / non-zero fields).
fn format_tx_footer(tx: &treb_forge::pipeline::RecordedTransaction) -> String {
    let mut parts = Vec::new();
    if !tx.transaction.hash.is_empty() {
        parts.push(format!("Tx: {}", tx.transaction.hash));
    }
    if tx.transaction.block_number > 0 {
        parts.push(format!("Block: {}", tx.transaction.block_number));
    }
    if let Some(gas_used) = tx.gas_used.filter(|gas| *gas > 0) {
        parts.push(format!("Gas: {gas_used}"));
    }
    if parts.is_empty() {
        return String::new();
    }
    let footer = parts.join(" | ");
    if color::is_color_enabled() {
        format!("   {}", footer.style(color::GRAY))
    } else {
        format!("   {}", footer)
    }
}

pub(crate) fn tx_sender_label(tx: &treb_forge::pipeline::RecordedTransaction) -> &str {
    tx.sender_name.as_deref().filter(|name| !name.is_empty()).unwrap_or(&tx.transaction.sender)
}

fn format_skipped_deployment_line(skipped: &treb_forge::SkippedDeployment) -> String {
    format!(
        "  - {} ({}) — {}",
        skipped.deployment.contract_name,
        output::truncate_address(&skipped.deployment.address),
        skipped.reason
    )
}

fn display_result_human(result: &PipelineResult, verbose: u8, _network: Option<&str>, _namespace: &str, skip_traces: bool, script_name: &str) {
    // ── Transactions ────────────────────────────────────────────────────
    if result.transactions.is_empty() && result.deployments.is_empty() {
        let msg = "No transactions recorded";
        if color::is_color_enabled() {
            println!("\n  {}", msg.style(color::GRAY));
        } else {
            println!("\n  {}", msg);
        }
    } else if !result.transactions.is_empty() {
        if skip_traces {
            // Preview already shown — display compact broadcast summary
            display_script_broadcast_summary(script_name, result);
        } else {
            display_transactions_ordered(&result.transactions, verbose);
        }
    }
    println!();

    // ── Collisions ──────────────────────────────────────────────────────
    if !result.collisions.is_empty() {
        print_warning_section_header("Deployment Collisions Detected", 50);
        for collision in &result.collisions {
            let name = if color::is_color_enabled() {
                format!("{}", collision.contract_name.style(color::CYAN))
            } else {
                collision.contract_name.clone()
            };
            let addr = format!("{}", collision.existing_address);
            let addr_display = if color::is_color_enabled() {
                format!("{}", addr.style(color::YELLOW))
            } else {
                addr
            };
            println!("  {} already deployed at {}", name, addr_display);
            for line in collision_metadata_lines(collision) {
                println!("{line}");
            }
        }
        let note = "Existing deployments were not overwritten";
        if color::is_color_enabled() {
            println!("\n  {}", note.style(color::GRAY));
        } else {
            println!("\n  {}", note);
        }
        println!();
    }

    // ── Deployment Summary ──────────────────────────────────────────────
    if !result.deployments.is_empty() {
        output::print_section_header(emoji::PACKAGE, "Deployment Summary", 50);
        for rd in &result.deployments {
            let d = &rd.deployment;
            // Build name: contract_name + optional :label + optional [impl]
            let mut name = d.contract_name.clone();
            if !d.label.is_empty() {
                name = format!("{name}:{}", d.label);
            }
            if let Some(ref pi) = d.proxy_info {
                let impl_name = result
                    .deployments
                    .iter()
                    .find(|other| other.deployment.address == pi.implementation)
                    .map(|other| other.deployment.contract_name.as_str())
                    .unwrap_or("UnknownImplementation");
                name = format!("{name}[{impl_name}]");
            }
            let name_display = if color::is_color_enabled() {
                format!("{}", name.style(color::CYAN))
            } else {
                name
            };
            let addr_display = if color::is_color_enabled() {
                format!("{}", d.address.style(color::GREEN))
            } else {
                d.address.clone()
            };
            println!("  {name_display} at {addr_display}");
        }
        println!();
    }

    // ── Console Logs ────────────────────────────────────────────────────
    if !result.console_logs.is_empty() {
        output::print_section_header(emoji::MEMO, "Script Logs", 40);
        for log in &result.console_logs {
            println!("  {}", log);
        }
        println!();
    }

    // ── Governor Proposals ──────────────────────────────────────────────
    if !result.governor_proposals.is_empty() {
        output::print_section_header(emoji::CLASSICAL_BUILDING, "Governor Proposals", 50);
        for gp in &result.governor_proposals {
            let status_label = if result.dry_run { "would be proposed" } else { "proposed" };
            let proposal_id = output::truncate_address(&gp.proposal_id);
            let governor = output::truncate_address(&gp.governor_address);
            let timelock = if gp.timelock_address.is_empty() {
                "none".to_string()
            } else {
                output::truncate_address(&gp.timelock_address)
            };
            let tx_count = gp.transaction_ids.len();
            let tx_suffix = if tx_count == 1 { "" } else { "s" };

            if color::is_color_enabled() {
                println!("\n  {} ({})", proposal_id.style(color::CYAN), status_label,);
                println!(
                    "   Governor: {} | Timelock: {} | Status: {} | {} linked transaction{}",
                    governor.style(color::GRAY),
                    timelock.style(color::GRAY),
                    format!("{}", gp.status).style(color::YELLOW),
                    tx_count,
                    tx_suffix,
                );
            } else {
                println!("\n  {} ({})", proposal_id, status_label);
                println!(
                    "   Governor: {} | Timelock: {} | Status: {} | {} linked transaction{}",
                    governor, timelock, gp.status, tx_count, tx_suffix,
                );
            }
        }
        println!();
    }

    // ── Proposed Transactions (from routing) ─────────────────────────────
    if !result.proposed_results.is_empty() {
        output::print_section_header(emoji::HOURGLASS, "Pending Proposals", 50);
        for pr in &result.proposed_results {
            match &pr.run_result {
                treb_forge::pipeline::RunResult::SafeProposed {
                    safe_tx_hash, safe_address, nonce, tx_count,
                } => {
                    let hash = output::truncate_address(&format!("{:#x}", safe_tx_hash));
                    let safe = output::truncate_address(&format!("{:#x}", safe_address));
                    let tx_suffix = if *tx_count == 1 { "" } else { "s" };
                    if color::is_color_enabled() {
                        println!(
                            "  {} {}  safeTxHash={} nonce={} ({} tx{})",
                            emoji::HOURGLASS,
                            pr.sender_role.style(color::CYAN),
                            hash.style(color::YELLOW),
                            nonce,
                            tx_count,
                            tx_suffix,
                        );
                        println!(
                            "   {}",
                            format!("Proposed to Safe {safe}").style(color::GRAY),
                        );
                    } else {
                        println!(
                            "  {} {}  safeTxHash={} nonce={} ({} tx{})",
                            emoji::HOURGLASS,
                            pr.sender_role, hash, nonce, tx_count, tx_suffix,
                        );
                        println!("   Proposed to Safe {safe}");
                    }
                }
                treb_forge::pipeline::RunResult::GovernorProposed {
                    proposal_id, governor_address, tx_count,
                } => {
                    let gov = output::truncate_address(&format!("{:#x}", governor_address));
                    let tx_suffix = if *tx_count == 1 { "" } else { "s" };
                    if color::is_color_enabled() {
                        println!(
                            "  {} {}  proposal={} ({} tx{})",
                            emoji::CLASSICAL_BUILDING,
                            pr.sender_role.style(color::CYAN),
                            proposal_id.style(color::YELLOW),
                            tx_count,
                            tx_suffix,
                        );
                        println!(
                            "   {}",
                            format!("Governor {gov}").style(color::GRAY),
                        );
                    } else {
                        println!(
                            "  {} {}  proposal={} ({} tx{})",
                            emoji::CLASSICAL_BUILDING,
                            pr.sender_role, proposal_id, tx_count, tx_suffix,
                        );
                        println!("   Governor {gov}");
                    }
                }
                _ => {}
            }
        }
        println!();
    }

    // ── Skipped Deployments ─────────────────────────────────────────────
    if !result.skipped.is_empty() {
        println!("Skipped:");
        for skipped in &result.skipped {
            println!("{}", format_skipped_deployment_line(skipped));
        }
        println!();
    }

    // ── Traces ───────────────────────────────────────────────────────────
    if verbose >= 3 {
        if let Some(ref setup) = result.setup_traces {
            println!("Setup Traces:");
            println!("{setup}");
        }
    }
    if verbose >= 1 {
        if let Some(ref traces) = result.execution_traces {
            println!("Traces:");
            println!("{traces}");
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use alloy_primitives::{Address, B256};
    use chrono::{TimeZone, Utc};
    use treb_core::types::{
        GovernorProposal, ProposalStatus, TransactionStatus,
        deployment::{ArtifactInfo, Deployment, DeploymentStrategy, VerificationInfo},
        enums::{DeploymentMethod, DeploymentType, VerificationStatus},
    };
    use treb_forge::{SkippedDeployment, events::ExtractedCollision, in_memory_signer};

    struct TestEnvVarGuard {
        name: &'static str,
        original: Option<String>,
    }

    impl TestEnvVarGuard {
        fn set(name: &'static str, value: &str) -> Self {
            let original = env::var(name).ok();
            // SAFETY: tests here are scoped and restore the original environment on drop.
            unsafe { env::set_var(name, value) };
            Self { name, original }
        }

        fn unset(name: &'static str) -> Self {
            let original = env::var(name).ok();
            // SAFETY: tests here are scoped and restore the original environment on drop.
            unsafe { env::remove_var(name) };
            Self { name, original }
        }
    }

    impl Drop for TestEnvVarGuard {
        fn drop(&mut self) {
            match self.original.as_deref() {
                Some(value) => {
                    // SAFETY: restores process env to its original test value.
                    unsafe { env::set_var(self.name, value) };
                }
                None => {
                    // SAFETY: restores process env to its original test value.
                    unsafe { env::remove_var(self.name) };
                }
            }
        }
    }

    fn sample_governor_proposal() -> GovernorProposal {
        GovernorProposal {
            proposal_id: "proposal-001".into(),
            governor_address: "0x1234567890abcdef1234567890abcdef12345678".into(),
            timelock_address: String::new(),
            chain_id: 1,
            status: ProposalStatus::Pending,
            transaction_ids: vec!["tx-001".into()],
            proposed_by: "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd".into(),
            proposed_at: Utc.with_ymd_and_hms(2025, 4, 1, 10, 0, 0).unwrap(),
            description: String::new(),
            executed_at: None,
            execution_tx_hash: String::new(),
            fork_executed_at: None,
            actions: Vec::new(),
        }
    }

    fn sample_skipped_deployment(reason: &str) -> SkippedDeployment {
        let timestamp = Utc.with_ymd_and_hms(2025, 4, 1, 10, 0, 0).unwrap();
        SkippedDeployment {
            deployment: Deployment {
                id: "production/1/Counter:v1".into(),
                namespace: "production".into(),
                chain_id: 1,
                contract_name: "Counter".into(),
                label: "v1".into(),
                address: "0x1234567890abcdef1234567890abcdef12345678".into(),
                deployment_type: DeploymentType::Singleton,
                transaction_id: "tx-001".into(),
                deployment_strategy: DeploymentStrategy {
                    method: DeploymentMethod::Create,
                    salt: String::new(),
                    init_code_hash: String::new(),
                    factory: String::new(),
                    constructor_args: String::new(),
                    entropy: String::new(),
                },
                proxy_info: None,
                artifact: ArtifactInfo {
                    path: "out/Counter.sol/Counter.json".into(),
                    compiler_version: "0.8.24".into(),
                    bytecode_hash: "0xabcdef".into(),
                    script_path: "script/Deploy.s.sol".into(),
                    git_commit: "abc1234".into(),
                },
                verification: VerificationInfo {
                    status: VerificationStatus::Unverified,
                    etherscan_url: String::new(),
                    verified_at: None,
                    reason: String::new(),
                    verifiers: HashMap::new(),
                },
                tags: None,
                created_at: timestamp,
                updated_at: timestamp,
            },
            reason: reason.into(),
        }
    }

    fn sample_pipeline_result() -> PipelineResult {
        PipelineResult {
            deployments: Vec::new(),
            transactions: Vec::new(),
            registry_updated: false,
            collisions: Vec::new(),
            skipped: Vec::new(),
            dry_run: false,
            success: true,
            gas_used: 0,
            event_count: 0,
            console_logs: Vec::new(),
            governor_proposals: Vec::new(),
            execution_traces: None,
            setup_traces: None,
            safe_transactions: Vec::new(),
            proposed_results: Vec::new(),
            queued_executions: Vec::new(),
        }
    }

    fn sample_active_fork_entry(
        network: &str,
        rpc_url: &str,
        env_var_name: &str,
    ) -> treb_core::types::fork::ForkEntry {
        let ts = Utc.with_ymd_and_hms(2026, 3, 9, 12, 0, 0).unwrap();
        treb_core::types::fork::ForkEntry {
            network: network.into(),
            instance_name: None,
            rpc_url: rpc_url.into(),
            port: 8545,
            chain_id: 1,
            fork_url: "https://eth.example.com".into(),
            fork_block_number: Some(19_000_000),
            snapshot_dir: ".treb/priv/snapshots/mainnet".into(),
            started_at: ts,
            env_var_name: env_var_name.into(),
            original_rpc: "https://eth.example.com".into(),
            anvil_pid: 1234,
            pid_file: ".treb/anvil.pid".into(),
            log_file: ".treb/anvil.log".into(),
            entered_at: ts,
            snapshots: vec![],
        }
    }

    fn write_active_fork(project_root: &std::path::Path, entry: treb_core::types::fork::ForkEntry) {
        let treb_dir = project_root.join(TREB_DIR);
        fs::create_dir_all(&treb_dir).unwrap();
        let mut store = ForkStateStore::new(&treb_dir);
        store.insert_active_fork(entry).unwrap();
    }

    fn write_foundry_rpc_project(project_root: &std::path::Path) {
        fs::write(
            project_root.join("foundry.toml"),
            r#"
[profile.default]
src = "src"

[rpc_endpoints]
mainnet = "${TREB_RUN_RPC_URL_P3_FIX}"
needs_env = "https://rpc.example/${TREB_RUN_MISSING_KEY_P3_FIX}"
"#,
        )
        .unwrap();
    }

    #[test]
    fn parse_env_var_valid_pair() {
        let (key, value) = parse_env_var("FOO=bar").unwrap();
        assert_eq!(key, "FOO");
        assert_eq!(value, "bar");
    }

    #[test]
    fn parse_env_var_empty_value() {
        let (key, value) = parse_env_var("KEY=").unwrap();
        assert_eq!(key, "KEY");
        assert_eq!(value, "");
    }

    #[test]
    fn parse_env_var_value_with_equals() {
        let (key, value) = parse_env_var("KEY=value=with=equals").unwrap();
        assert_eq!(key, "KEY");
        assert_eq!(value, "value=with=equals");
    }

    #[test]
    fn parse_env_var_missing_equals_fails() {
        let err = parse_env_var("INVALID").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("INVALID"), "should mention the value: {msg}");
        assert!(msg.contains("missing '='"), "should mention missing '=': {msg}");
    }

    #[test]
    fn parse_env_var_empty_key_fails() {
        let err = parse_env_var("=value").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("key cannot be empty"), "should mention empty key: {msg}");
    }

    #[test]
    fn inject_env_vars_sets_vars() {
        let vars = vec!["TREB_TEST_ENV_A=hello".to_string(), "TREB_TEST_ENV_B=world".to_string()];
        inject_env_vars(&vars).unwrap();

        assert_eq!(env::var("TREB_TEST_ENV_A").unwrap(), "hello");
        assert_eq!(env::var("TREB_TEST_ENV_B").unwrap(), "world");

        // Cleanup
        unsafe {
            env::remove_var("TREB_TEST_ENV_A");
            env::remove_var("TREB_TEST_ENV_B");
        }
    }

    #[test]
    fn inject_env_vars_fails_on_bad_pair() {
        let vars = vec!["GOOD=value".to_string(), "BAD".to_string()];
        let err = inject_env_vars(&vars).unwrap_err();
        assert!(err.to_string().contains("BAD"));
    }

    #[test]
    fn prompt_for_broadcast_confirmation_requires_interactive_broadcast() {
        assert!(should_prompt_for_broadcast_confirmation(true, false, true));
        assert!(!should_prompt_for_broadcast_confirmation(true, true, true));
        assert!(!should_prompt_for_broadcast_confirmation(true, false, false));
        assert!(!should_prompt_for_broadcast_confirmation(false, false, true));
    }

    #[test]
    fn interactive_json_broadcast_is_rejected_before_prompting() {
        assert!(should_reject_interactive_json_broadcast(true, false, true, true));
        assert!(!should_reject_interactive_json_broadcast(true, false, true, false));
        assert!(!should_reject_interactive_json_broadcast(true, false, false, true));
        assert!(!should_reject_interactive_json_broadcast(true, true, true, true));
    }

    #[test]
    fn deployment_banner_mode_uses_go_parity_labels() {
        // Mode is driven by broadcast flag only — fork/live distinction removed
        assert_eq!(deployment_banner_mode(false, false, false).0, "DRY_RUN");
        assert_eq!(deployment_banner_mode(false, false, true).0, "DRY_RUN");
        assert_eq!(deployment_banner_mode(false, true, true).0, "BROADCAST");
        assert_eq!(deployment_banner_mode(false, true, false).0, "BROADCAST");
    }

    #[test]
    fn sorted_env_var_entries_orders_by_key() {
        let sorted = sorted_env_var_entries(&[
            "ZETA=last".to_string(),
            "ALPHA=first".to_string(),
            "MIDDLE=value".to_string(),
        ]);

        assert_eq!(
            sorted,
            vec![
                ("ALPHA".to_string(), "first".to_string()),
                ("MIDDLE".to_string(), "value".to_string()),
                ("ZETA".to_string(), "last".to_string()),
            ]
        );
    }

    #[test]
    fn format_verbose_sender_includes_governor_context() {
        let proposer = ResolvedSender::InMemory(in_memory_signer(0).unwrap());
        let sender = ResolvedSender::Governor {
            governor_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse().unwrap(),
            timelock_address: Some("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".parse().unwrap()),
            proposer: Box::new(proposer),
            proposer_script: None,
        };

        let line = format_verbose_sender("deployer", &sender);

        assert_eq!(
            line,
            "deployer: governor=0xaAaAaAaaAaAaAaaAaAAAAAAAAaaaAaAaAaaAaaAa timelock=0xbBbBBBBbbBBBbbbBbbBbbbbBBbBbbbbBbBbbBBbB proposer=0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
        );
    }

    #[test]
    fn format_verbose_sender_uses_none_for_missing_timelock() {
        let proposer = ResolvedSender::InMemory(in_memory_signer(1).unwrap());
        let sender = ResolvedSender::Governor {
            governor_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse().unwrap(),
            timelock_address: None,
            proposer: Box::new(proposer),
            proposer_script: None,
        };

        let line = format_verbose_sender("deployer", &sender);

        assert!(line.contains("timelock=none"), "got: {line}");
        assert!(
            line.contains("proposer=0x70997970C51812dc3A010C7d01b50e0d17dc79C8"),
            "got: {line}"
        );
    }

    #[test]
    fn format_verbose_senders_returns_sorted_rows() {
        let mut senders = HashMap::new();
        senders
            .insert("deployer".to_string(), ResolvedSender::InMemory(in_memory_signer(0).unwrap()));
        senders.insert("anvil".to_string(), ResolvedSender::InMemory(in_memory_signer(1).unwrap()));

        let lines = format_verbose_senders(&senders);

        assert_eq!(lines.len(), 2);
        // Padded format: role is left-aligned to max role length, then two spaces
        assert_eq!(lines[0], "anvil     0x70997970C51812dc3A010C7d01b50e0d17dc79C8");
        assert_eq!(lines[1], "deployer  0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    }

    #[test]
    fn is_active_fork_run_detects_direct_rpc_match() {
        let tmp = tempfile::tempdir().unwrap();
        write_active_fork(
            tmp.path(),
            sample_active_fork_entry("mainnet", "http://127.0.0.1:8545", "ETH_RPC_URL_MAINNET"),
        );

        assert!(is_active_fork_run(tmp.path(), None, Some("http://127.0.0.1:8545")));
    }

    #[test]
    fn is_active_fork_run_detects_network_env_override() {
        let tmp = tempfile::tempdir().unwrap();
        write_active_fork(
            tmp.path(),
            sample_active_fork_entry("mainnet", "http://127.0.0.1:8545", "ETH_RPC_URL_MAINNET"),
        );
        let _rpc_override = TestEnvVarGuard::set("ETH_RPC_URL_MAINNET", "http://127.0.0.1:8545");

        assert!(is_active_fork_run(tmp.path(), Some("mainnet"), Some("mainnet")));
    }

    #[test]
    fn is_active_fork_run_honors_treb_fork_mode_env() {
        let tmp = tempfile::tempdir().unwrap();
        let _fork_mode = TestEnvVarGuard::set("TREB_FORK_MODE", "true");

        assert!(is_active_fork_run(tmp.path(), None, None));
    }

    #[test]
    fn resolve_rpc_url_for_chain_id_expands_dotenv_backed_alias() {
        let tmp = tempfile::tempdir().unwrap();
        let _rpc = TestEnvVarGuard::unset("TREB_RUN_RPC_URL_P3_FIX");
        write_foundry_rpc_project(tmp.path());
        fs::write(tmp.path().join(".env"), "TREB_RUN_RPC_URL_P3_FIX=https://mainnet.rpc.example\n")
            .unwrap();

        let url = resolve_rpc_url_for_chain_id("mainnet", tmp.path());

        assert_eq!(url.as_deref(), Some("https://mainnet.rpc.example"));
    }

    #[test]
    fn resolve_rpc_url_for_chain_id_rejects_missing_env_alias() {
        let tmp = tempfile::tempdir().unwrap();
        let _key = TestEnvVarGuard::unset("TREB_RUN_MISSING_KEY_P3_FIX");
        write_foundry_rpc_project(tmp.path());

        let url = resolve_rpc_url_for_chain_id("needs_env", tmp.path());

        assert!(url.is_none());
    }

    #[test]
    fn collision_metadata_lines_include_label_and_entropy_when_present() {
        let collision = ExtractedCollision {
            existing_address: Address::ZERO,
            contract_name: "Counter".into(),
            label: "counter-v1".into(),
            entropy: "entropy-seed".into(),
            strategy: DeploymentMethod::Create2,
            salt: B256::ZERO,
            bytecode_hash: B256::ZERO,
            init_code_hash: B256::ZERO,
        };

        assert_eq!(
            collision_metadata_lines(&collision),
            vec!["    Label: counter-v1".to_string(), "    Entropy: entropy-seed".to_string(),]
        );
    }

    #[test]
    fn format_skipped_deployment_line_includes_name_address_and_reason() {
        let skipped = sample_skipped_deployment(
            "Deployment with ID 'production/1/Counter:v1' already exists",
        );

        assert_eq!(
            format_skipped_deployment_line(&skipped),
            "  - Counter (0x1234...5678) — Deployment with ID 'production/1/Counter:v1' already exists"
        );
    }

    #[test]
    fn format_tx_operations_includes_all_operations() {
        // Disable color so we get predictable output
        owo_colors::set_override(false);
        let operations = vec![
            Operation {
                operation_type: "DEPLOY".into(),
                target: "0x0000000000000000000000000000000000001001".into(),
                method: "CREATE".into(),
                result: HashMap::new(),
            },
            Operation {
                operation_type: "DEPLOY".into(),
                target: "0x0000000000000000000000000000000000001002".into(),
                method: "CREATE".into(),
                result: HashMap::new(),
            },
        ];

        // Format: "type truncated_target.method()" joined by " | "
        assert_eq!(
            format_tx_operations(&operations),
            "DEPLOY 0x0000...1001.CREATE() | DEPLOY 0x0000...1002.CREATE()"
        );
    }

    #[test]
    fn warning_section_header_uses_warning_spacing_and_style() {
        owo_colors::set_override(true);

        let header =
            format_warning_section_header_with_style("Deployment Collisions Detected", true);

        assert!(header.starts_with("⚠️  "), "got: {header}");
        assert!(header.contains("\u{1b}["), "header should contain ANSI styling: {header}");

        owo_colors::set_override(false);
    }

    #[test]
    fn format_tx_footer_includes_gas_when_present() {
        let recorded = treb_forge::pipeline::RecordedTransaction {
            transaction: treb_core::types::Transaction {
                id: "tx-001".into(),
                chain_id: 1,
                hash: "0xabc".into(),
                status: TransactionStatus::Executed,
                block_number: 42,
                sender: "0xsender".into(),
                nonce: 0,
                deployments: Vec::new(),
                operations: Vec::new(),
                safe_context: None,
                broadcast_file: None,
                environment: "default".into(),
                created_at: Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(),
            },
            sender_name: Some("deployer".into()),
            sender_category: None,
            gas_used: Some(123456),
            trace: None,
        };

        let footer = format_tx_footer(&recorded);
        assert!(footer.contains("Tx: 0xabc"), "got: {footer}");
        assert!(footer.contains("Block: 42"), "got: {footer}");
        assert!(footer.contains("Gas: 123456"), "got: {footer}");
    }

    #[test]
    fn tx_sender_label_prefers_sender_name() {
        let recorded = treb_forge::pipeline::RecordedTransaction {
            transaction: treb_core::types::Transaction {
                id: "tx-001".into(),
                chain_id: 1,
                hash: String::new(),
                status: TransactionStatus::Simulated,
                block_number: 0,
                sender: "0xsender".into(),
                nonce: 0,
                deployments: Vec::new(),
                operations: Vec::new(),
                safe_context: None,
                broadcast_file: None,
                environment: "default".into(),
                created_at: Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(),
            },
            sender_name: Some("anvil".into()),
            sender_category: None,
            gas_used: None,
            trace: None,
        };

        assert_eq!(tx_sender_label(&recorded), "anvil");
    }

    #[test]
    fn registry_update_message_uses_provided_network_name() {
        let mut result = sample_pipeline_result();
        result.registry_updated = true;

        let (message, updated) =
            registry_update_message(&result, Some("sepolia"), "default").unwrap();

        assert!(updated);
        assert_eq!(message, "✓ Updated registry for sepolia network in namespace default");
    }

    #[test]
    fn registry_update_message_treats_governor_only_results_as_registry_updates() {
        let mut result = sample_pipeline_result();
        result.registry_updated = true;
        result.governor_proposals.push(sample_governor_proposal());

        let (message, updated) =
            registry_update_message(&result, Some("anvil-31337"), "default").unwrap();

        assert!(updated);
        assert!(message.contains("Updated registry for anvil-31337 network"));
        assert!(!message.contains("No registry changes recorded"));
    }
}
