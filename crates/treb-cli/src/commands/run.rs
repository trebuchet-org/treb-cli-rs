//! `treb run` command implementation.

use std::{
    collections::HashMap,
    env, fs,
    io::{self, BufRead, Write},
    path::PathBuf,
    time::Duration,
};

use anyhow::{Context, bail};
use foundry_common::{
    Shell as FoundryShell,
    shell::{ColorChoice, OutputFormat, OutputMode, Verbosity},
};
use owo_colors::OwoColorize;
use serde::Serialize;
use treb_config::{ResolveOpts, resolve_config};
use treb_core::types::{Operation, TransactionStatus};
use treb_forge::{
    pipeline::{PipelineConfig, PipelineContext, PipelineResult, RunPipeline, resolve_git_commit},
    script::build_script_config_with_senders,
    sender::{ResolvedSender, resolve_all_senders},
};
use treb_registry::Registry;

use crate::{
    output,
    ui::{color, emoji, interactive::is_non_interactive, selector::fuzzy_select_network},
};

const FOUNDRY_TOML: &str = "foundry.toml";
const TREB_DIR: &str = ".treb";

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

fn format_verbose_sender(role: &str, sender: &ResolvedSender) -> String {
    match sender {
        ResolvedSender::Governor { governor_address, timelock_address, proposer } => {
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
struct FoundryShellGuard {
    output_format: OutputFormat,
    output_mode: OutputMode,
    color_choice: ColorChoice,
    verbosity: Verbosity,
}

impl FoundryShellGuard {
    fn suppress() -> Self {
        let mut shell = FoundryShell::get();
        let previous = Self {
            output_format: shell.output_format(),
            output_mode: shell.output_mode(),
            color_choice: shell.color_choice(),
            verbosity: shell.verbosity(),
        };
        *shell = FoundryShell::empty();
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
    let config = treb_config::load_foundry_config(cwd).ok()?;
    let endpoints = treb_config::rpc_endpoints(&config);
    let url = endpoints.get(network_or_url)?;
    if url.contains("${") {
        return None; // unresolved env vars
    }
    Some(url.clone())
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
    dump_command: bool,
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

    // ── Interactive network selection when --network is omitted ───────────
    let network = if network.is_none() && prompts_enabled {
        let foundry_cfg = treb_config::load_foundry_config(&cwd)
            .map_err(|e| anyhow::anyhow!("failed to load foundry config: {e}"))?;
        let endpoints = treb_config::rpc_endpoints(&foundry_cfg);
        let mut names: Vec<String> = endpoints.keys().cloned().collect();
        names.sort();
        fuzzy_select_network(&names).map_err(|e| anyhow::anyhow!("{e}"))?.map(|s| s.to_string())
    } else {
        network
    };

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
    let mut resolved_senders =
        resolve_all_senders(&resolved.senders).await.context("failed to resolve senders")?;

    // ── Build ScriptConfig with all CLI flags ────────────────────────────
    let mut script_config = build_script_config_with_senders(&resolved, script, &resolved_senders)
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

    // ── Dump command and exit ─────────────────────────────────────────
    if dump_command {
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

    // ── Resolve chain ID from RPC ──────────────────────────────────────
    let chain_id = if let Some(ref network_or_url) = effective_rpc_url {
        let actual_url = resolve_rpc_url_for_chain_id(network_or_url, &cwd);
        if let Some(url) = actual_url { fetch_chain_id(&url).await.unwrap_or(0) } else { 0 }
    } else {
        0
    };

    // ── Verbose pre-execution context ──────────────────────────────────
    if verbose && !json {
        let broadcast_mode = if dry_run {
            "dry-run"
        } else if broadcast {
            "broadcast"
        } else {
            "simulate"
        };
        let rpc_display = effective_rpc_url.as_deref().unwrap_or("(none)");
        let chain_id_str = chain_id.to_string();
        let mut kv_pairs: Vec<(&str, &str)> = vec![
            ("Config source", &resolved.config_source),
            ("Namespace", &resolved.namespace),
            ("Chain ID", &chain_id_str),
            ("RPC", rpc_display),
        ];
        // Collect sender lines
        let mut sender_lines: Vec<String> =
            resolved_senders.iter().map(|(role, s)| format_verbose_sender(role, s)).collect();
        sender_lines.sort();
        for line in &sender_lines {
            kv_pairs.push(("Sender", line));
        }
        kv_pairs.push(("Script path", script));
        kv_pairs.push(("Function sig", sig));
        kv_pairs.push(("Broadcast mode", broadcast_mode));
        output::eprint_kv(&kv_pairs);
        eprintln!();
    }

    // ── Build PipelineConfig and PipelineContext ─────────────────────────
    let pipeline_config = PipelineConfig {
        script_path: script.to_string(),
        dry_run,
        namespace: resolved.namespace.clone(),
        chain_id,
        script_sig: sig.to_string(),
        script_args: Vec::new(), // args already in ScriptConfig
        ..Default::default()
    };

    let git_commit = resolve_git_commit();

    // Extract the deployer sender so the pipeline can detect Safe/Governor flows.
    let deployer_sender = resolved_senders.remove("deployer");
    let is_governor_sender = deployer_sender.as_ref().is_some_and(|s| s.is_governor());

    let pipeline_context = PipelineContext {
        config: pipeline_config,
        script_path: PathBuf::from(script),
        git_commit,
        project_root: cwd.clone(),
        deployer_sender,
    };

    // ── Broadcast confirmation prompt ──────────────────────────────────
    if should_prompt_for_broadcast_confirmation(broadcast, dry_run, prompts_enabled) {
        eprintln!("About to broadcast transactions to the network.");
        eprintln!("  Script: {}", script);
        eprintln!("  Namespace: {}", resolved.namespace);
        if let Some(ref url) = effective_rpc_url {
            eprintln!("  RPC: {}", url);
        }
        eprint!("Proceed? [y/N] ");
        io::stderr().flush().ok();

        let mut input = String::new();
        io::stdin().lock().read_line(&mut input).ok();
        let answer = input.trim().to_lowercase();

        if answer != "y" && answer != "yes" {
            bail!("broadcast cancelled by user");
        }
    }

    // ── Open registry and execute pipeline ───────────────────────────────
    let mut registry = Registry::open(&cwd).context("failed to open registry")?;

    if !json {
        output::print_stage("\u{1f528}", &format!("Compiling and executing {}...", script));
    }

    let pipeline = RunPipeline::new(pipeline_context).with_script_config(script_config);

    if !json && !dry_run && broadcast {
        if is_governor_sender {
            output::print_stage("\u{1f3db}\u{fe0f}", "Creating governance proposal...");
        } else {
            output::print_stage("\u{1f4e1}", "Broadcasting...");
        }
    } else if !json && dry_run {
        output::print_stage("\u{1f9ea}", "Simulating...");
    }

    let result = {
        let _foundry_shell = json.then(FoundryShellGuard::suppress);
        pipeline.execute(&mut registry).await.context("pipeline execution failed")?
    };

    if !json {
        output::print_stage("\u{2705}", "Execution complete.");
    }

    // ── Verbose post-execution output ────────────────────────────────────
    if verbose && !json {
        eprintln!();
        // Event count summary
        let event_str = format!("{} event(s) decoded", result.event_count);
        let console_str = format!("{} console.log line(s)", result.console_logs.len());
        let dep_str = format!("{} deployment(s)", result.deployments.len());
        let tx_str = format!("{} transaction(s)", result.transactions.len());
        let proposal_str = format!("{} governor proposal(s)", result.governor_proposals.len());
        let skip_str = format!("{} skipped", result.skipped.len());
        let summary_pairs: Vec<(&str, &str)> = vec![
            ("Events", &event_str),
            ("Console", &console_str),
            ("Deployments", &dep_str),
            ("Transactions", &tx_str),
            ("Governor proposals", &proposal_str),
            ("Skipped", &skip_str),
        ];
        output::eprint_kv(&summary_pairs);

        // Per-deployment registry write confirmations
        if !result.deployments.is_empty() {
            eprintln!();
            let action = if result.dry_run { "Would register" } else { "Registered" };
            for rd in &result.deployments {
                let d = &rd.deployment;
                eprintln!(
                    "  {} {} ({})",
                    action,
                    d.contract_name,
                    output::truncate_address(&d.address),
                );
            }
        }
    }

    // ── Display results ──────────────────────────────────────────────────
    display_result(&result, json)?;

    // ── Debug log output ────────────────────────────────────────────────
    if debug {
        let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
        let log_filename = format!("debug-{}.log", timestamp);
        let log_path = cwd.join(TREB_DIR).join(&log_filename);

        let mut log_content = String::new();
        log_content.push_str(&format!("treb run debug log — {}\n", timestamp));
        log_content.push_str(&format!("Script: {}\n", script));
        log_content.push_str(&format!("Sig: {}\n", sig));
        log_content.push_str(&format!("Namespace: {}\n", resolved.namespace));
        log_content.push_str(&format!("Chain ID: {}\n", chain_id));
        if let Some(ref url) = effective_rpc_url {
            log_content.push_str(&format!("RPC: {}\n", url));
        }
        log_content.push_str(&format!("Broadcast: {}\n", broadcast));
        log_content.push_str(&format!("Dry run: {}\n", dry_run));
        log_content.push_str(&format!("Success: {}\n", result.success));
        log_content.push_str(&format!("Gas used: {}\n", result.gas_used));
        log_content.push_str(&format!("Events decoded: {}\n", result.event_count));
        log_content.push_str(&format!("Deployments: {}\n", result.deployments.len()));
        log_content.push_str(&format!("Transactions: {}\n", result.transactions.len()));
        log_content.push_str(&format!("Skipped: {}\n", result.skipped.len()));
        log_content.push_str(&format!("Collisions: {}\n", result.collisions.len()));

        if !result.console_logs.is_empty() {
            log_content.push_str("\n--- Console Output ---\n");
            for line in &result.console_logs {
                log_content.push_str(line);
                log_content.push('\n');
            }
        }

        if !result.deployments.is_empty() {
            log_content.push_str("\n--- Deployments ---\n");
            for rd in &result.deployments {
                let d = &rd.deployment;
                log_content.push_str(&format!(
                    "  {} {} ({}) chain={}\n",
                    d.deployment_type, d.contract_name, d.address, d.chain_id
                ));
            }
        }

        if !result.transactions.is_empty() {
            log_content.push_str("\n--- Transactions ---\n");
            for rt in &result.transactions {
                let tx = &rt.transaction;
                log_content.push_str(&format!("  {} {} ({})\n", tx.id, tx.hash, tx.status));
            }
        }

        fs::write(&log_path, &log_content)
            .with_context(|| format!("failed to write debug log to {}", log_path.display()))?;
        eprintln!("Debug log saved to {}", log_path.display());
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

fn format_governor_proposal_details(gp: &treb_core::types::GovernorProposal) -> String {
    let timelock = if gp.timelock_address.is_empty() {
        "none".to_string()
    } else {
        output::truncate_address(&gp.timelock_address)
    };
    let tx_count = gp.transaction_ids.len();

    format!(
        "Governor: {} | Timelock: {} | Status: {} | {} linked transaction{}",
        output::truncate_address(&gp.governor_address),
        timelock,
        gp.status,
        tx_count,
        if tx_count == 1 { "" } else { "s" },
    )
}

fn registry_update_section_message(result: &PipelineResult) -> Option<&'static str> {
    if result.dry_run { None } else { Some("Registry update details pending.") }
}

fn print_registry_update_section(result: &PipelineResult) {
    output::print_section_header(emoji::CHECK_MARK, "Registry Update", 50);
    if let Some(message) = registry_update_section_message(result) {
        println!("  {message}");
    }
    println!();
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
// Transaction rendering helpers (Go: render/transaction.go)
// ---------------------------------------------------------------------------

/// Format a transaction status as a fixed-width (9-char) lowercase string
/// with Go-matching color: simulated=faint, queued=yellow, executed=green,
/// failed=red.
fn format_tx_status(status: &TransactionStatus) -> String {
    let (label, style) = match status {
        TransactionStatus::Simulated => ("simulated", color::MUTED),
        TransactionStatus::Queued => ("queued   ", color::YELLOW),
        TransactionStatus::Executed => ("executed ", color::GREEN),
        TransactionStatus::Failed => ("failed   ", color::RED),
    };
    if color::is_color_enabled() { format!("{}", label.style(style)) } else { label.to_string() }
}

/// Format a summary of the first operation for display after the `→` arrow.
fn format_tx_operations(operations: &[Operation]) -> String {
    if operations.is_empty() {
        return String::new();
    }
    let op = &operations[0];
    if op.method.is_empty() {
        format!("{} {}", op.operation_type, op.target)
    } else {
        format!("{}::{}({})", op.target, op.method, op.operation_type)
    }
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

fn tx_sender_label(tx: &treb_forge::pipeline::RecordedTransaction) -> &str {
    tx.sender_name.as_deref().filter(|name| !name.is_empty()).unwrap_or(&tx.transaction.sender)
}

fn display_result_human(result: &PipelineResult) {
    // ── Transactions ────────────────────────────────────────────────────
    output::print_section_header(emoji::REFRESH, "Transactions", 50);
    if result.transactions.is_empty() {
        let msg = "No transactions executed (dry run or all deployments skipped)";
        if color::is_color_enabled() {
            println!("  {}", msg.style(color::GRAY));
        } else {
            println!("  {}", msg);
        }
    } else {
        for rt in &result.transactions {
            let tx = &rt.transaction;
            let status_str = format_tx_status(&tx.status);
            let sender_str = if color::is_color_enabled() {
                format!("{}", tx_sender_label(rt).style(color::GREEN))
            } else {
                tx_sender_label(rt).to_string()
            };
            let ops_str = format_tx_operations(&tx.operations);
            if ops_str.is_empty() {
                println!("\n  {} {}", status_str, sender_str);
            } else {
                println!("\n  {} {} → {}", status_str, sender_str, ops_str);
            }
            let footer = format_tx_footer(rt);
            if !footer.is_empty() {
                println!("{}", footer);
            }
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

    // ── Registry Update ────────────────────────────────────────────────
    print_registry_update_section(result);

    // ── Governor Proposals ──────────────────────────────────────────────
    if !result.governor_proposals.is_empty() {
        println!("Governor Proposals:");
        for (i, gp) in result.governor_proposals.iter().enumerate() {
            let action = if result.dry_run { "would be proposed" } else { "proposed" };
            println!("  {}. {} ({})", i + 1, output::truncate_address(&gp.proposal_id), action,);
            println!("     {}", format_governor_proposal_details(gp));
        }
        println!();
    }

    // ── Skipped Deployments ─────────────────────────────────────────────
    if !result.skipped.is_empty() {
        println!("Skipped:");
        for s in &result.skipped {
            println!(
                "  - {} ({}) — {}",
                s.deployment.contract_name,
                output::truncate_address(&s.deployment.address),
                s.reason
            );
        }
        println!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, B256};
    use chrono::{TimeZone, Utc};
    use treb_core::types::{GovernorProposal, ProposalStatus, enums::DeploymentMethod};
    use treb_forge::{events::ExtractedCollision, in_memory_signer};

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
        }
    }

    fn sample_pipeline_result() -> PipelineResult {
        PipelineResult {
            deployments: Vec::new(),
            transactions: Vec::new(),
            collisions: Vec::new(),
            skipped: Vec::new(),
            dry_run: false,
            success: true,
            gas_used: 0,
            event_count: 0,
            console_logs: Vec::new(),
            governor_proposals: Vec::new(),
        }
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
    fn format_governor_proposal_details_includes_status() {
        let mut proposal = sample_governor_proposal();
        proposal.status = ProposalStatus::Queued;
        proposal.timelock_address = "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd".into();
        proposal.transaction_ids = vec!["tx-001".into(), "tx-002".into()];

        let details = format_governor_proposal_details(&proposal);

        assert!(details.contains("Status: queued"), "details should include status: {details}");
        assert!(
            details.contains("Timelock: 0xabcd...abcd"),
            "details should include the timelock address: {details}"
        );
        assert!(
            details.contains("2 linked transactions"),
            "details should include the linked transaction count: {details}"
        );
    }

    #[test]
    fn format_governor_proposal_details_uses_none_for_missing_timelock() {
        let proposal = sample_governor_proposal();

        let details = format_governor_proposal_details(&proposal);

        assert!(
            details.contains("Timelock: none"),
            "details should show an explicit placeholder when timelock is absent: {details}"
        );
        assert!(details.contains("Status: pending"), "details should include status: {details}");
    }

    #[test]
    fn format_verbose_sender_includes_governor_context() {
        let proposer = ResolvedSender::InMemory(in_memory_signer(0).unwrap());
        let sender = ResolvedSender::Governor {
            governor_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse().unwrap(),
            timelock_address: Some("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".parse().unwrap()),
            proposer: Box::new(proposer),
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
        };

        let line = format_verbose_sender("deployer", &sender);

        assert!(line.contains("timelock=none"), "got: {line}");
        assert!(
            line.contains("proposer=0x70997970C51812dc3A010C7d01b50e0d17dc79C8"),
            "got: {line}"
        );
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
    fn warning_section_header_uses_warning_spacing_and_style() {
        owo_colors::set_override(true);

        let header =
            format_warning_section_header_with_style("Deployment Collisions Detected", true);

        assert!(header.starts_with("⚠️  "), "got: {header}");
        assert!(header.contains("\u{1b}["), "header should contain ANSI styling: {header}");

        owo_colors::set_override(false);
    }

    #[test]
    fn registry_update_section_message_uses_placeholder_for_non_dry_run() {
        let result = sample_pipeline_result();

        assert_eq!(
            registry_update_section_message(&result),
            Some("Registry update details pending.")
        );
    }

    #[test]
    fn registry_update_section_message_omits_placeholder_for_dry_run() {
        let mut result = sample_pipeline_result();
        result.dry_run = true;

        assert_eq!(registry_update_section_message(&result), None);
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
                environment: "default".into(),
                created_at: Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(),
            },
            sender_name: Some("deployer".into()),
            gas_used: Some(123456),
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
                environment: "default".into(),
                created_at: Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(),
            },
            sender_name: Some("anvil".into()),
            gas_used: None,
        };

        assert_eq!(tx_sender_label(&recorded), "anvil");
    }
}
