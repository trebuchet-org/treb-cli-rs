//! `treb run` command implementation.

use std::{
    collections::{BTreeMap, HashMap},
    env, fs,
    io::{self, BufRead, Write},
    path::PathBuf,
    time::Duration,
};

use anyhow::{Context, bail};
use serde::Serialize;
use treb_config::{ResolveOpts, resolve_config};
use treb_core::types::DeploymentType;
use treb_forge::{
    pipeline::{
        PipelineConfig, PipelineContext, PipelineResult, RecordedDeployment, RunPipeline,
        resolve_git_commit,
    },
    script::build_script_config_with_senders,
    sender::{ResolvedSender, resolve_all_senders},
};
use treb_registry::Registry;

use crate::{
    output,
    ui::{
        badge, color, interactive::is_non_interactive, selector::fuzzy_select_network,
        tree::TreeNode,
    },
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

    let result = pipeline.execute(&mut registry).await.context("pipeline execution failed")?;

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

// ---------------------------------------------------------------------------
// Deployment grouping for tree display: namespace > chain_id > type
// ---------------------------------------------------------------------------

/// Returns the sort key for the fixed display order of deployment types.
fn type_sort_key(dt: &DeploymentType) -> u8 {
    match dt {
        DeploymentType::Proxy => 0,
        DeploymentType::Singleton => 1,
        DeploymentType::Library => 2,
        DeploymentType::Unknown => 3,
    }
}

/// A group of recorded deployments sharing the same deployment type.
struct RunTypeGroup<'a> {
    deployment_type: DeploymentType,
    deployments: Vec<&'a RecordedDeployment>,
}

/// Group recorded deployments by namespace > chain_id > deployment type.
fn group_recorded_deployments<'a>(
    deployments: &'a [RecordedDeployment],
) -> BTreeMap<String, BTreeMap<u64, Vec<RunTypeGroup<'a>>>> {
    let mut result: BTreeMap<String, BTreeMap<u64, Vec<RunTypeGroup<'a>>>> = BTreeMap::new();

    for rd in deployments {
        let d = &rd.deployment;
        let chain_map = result.entry(d.namespace.clone()).or_default();
        let type_groups = chain_map.entry(d.chain_id).or_default();

        if let Some(group) = type_groups.iter_mut().find(|g| g.deployment_type == d.deployment_type)
        {
            group.deployments.push(rd);
        } else {
            type_groups.push(RunTypeGroup {
                deployment_type: d.deployment_type.clone(),
                deployments: vec![rd],
            });
        }
    }

    // Sort type groups by fixed order and deployments by contract name
    for chain_map in result.values_mut() {
        for type_groups in chain_map.values_mut() {
            type_groups.sort_by_key(|g| type_sort_key(&g.deployment_type));
            for group in type_groups.iter_mut() {
                group
                    .deployments
                    .sort_by(|a, b| a.deployment.contract_name.cmp(&b.deployment.contract_name));
            }
        }
    }

    result
}

/// Format a deployment entry label for tree display.
///
/// Format: `ContractName[:label] address badge [fork]`
fn format_run_deployment_entry(d: &treb_core::types::Deployment) -> String {
    let addr = output::truncate_address(&d.address);
    let name_part = if d.label.is_empty() {
        d.contract_name.clone()
    } else {
        format!("{}:{}", d.contract_name, d.label)
    };
    let ver_badge = badge::verification_badge(&d.verification.verifiers);
    let mut parts = vec![name_part, addr, ver_badge];
    if let Some(fb) = badge::fork_badge(&d.namespace) {
        parts.push(fb);
    }
    parts.join(" ")
}

/// Build a TreeNode for a recorded deployment, including an implementation
/// child node when proxy_info is present.
fn build_run_deployment_node(d: &treb_core::types::Deployment) -> TreeNode {
    let label = format_run_deployment_entry(d);
    let mut node = TreeNode::new(label);
    if let Some(ref pi) = d.proxy_info {
        let impl_label = format!("Implementation {}", output::truncate_address(&pi.implementation));
        node = node.child(TreeNode::new(impl_label));
    }
    node
}

fn display_result_human(result: &PipelineResult) {
    // Dry-run banner
    if result.dry_run {
        output::print_warning_banner(
            "\u{1f6a7}",
            "[DRY RUN] No changes were written to the registry.",
        );
        println!();
    }

    // Console.log output
    if !result.console_logs.is_empty() {
        for log in &result.console_logs {
            println!("{}", log);
        }
        println!();
    }

    // Deployment tree
    if !result.deployments.is_empty() {
        let grouped = group_recorded_deployments(&result.deployments);
        let mut first = true;
        for (namespace, chains) in &grouped {
            if !first {
                println!();
            }
            first = false;
            let mut ns_node = TreeNode::new(namespace.clone()).with_style(color::NAMESPACE);
            for (chain_id, type_groups) in chains {
                let mut chain_node = TreeNode::new(chain_id.to_string()).with_style(color::CHAIN);
                for tg in type_groups {
                    let type_label = tg.deployment_type.to_string();
                    let type_style = color::style_for_deployment_type(tg.deployment_type.clone());
                    let mut type_node = TreeNode::new(type_label).with_style(type_style);
                    for rd in &tg.deployments {
                        type_node = type_node.child(build_run_deployment_node(&rd.deployment));
                    }
                    chain_node = chain_node.child(type_node);
                }
                ns_node = ns_node.child(chain_node);
            }
            if color::is_color_enabled() {
                println!("{}", ns_node.render_styled());
            } else {
                println!("{}", ns_node.render());
            }
        }
        println!();
    }

    // Transactions
    if !result.transactions.is_empty() {
        println!("Transactions:");
        for (i, rt) in result.transactions.iter().enumerate() {
            let tx = &rt.transaction;
            let status = tx.status.to_string();
            println!("  {}. {} ({})", i + 1, tx.hash, status);
        }
        println!();
    }

    // Governor proposals
    if !result.governor_proposals.is_empty() {
        println!("Governor Proposals:");
        for (i, gp) in result.governor_proposals.iter().enumerate() {
            let action = if result.dry_run { "would be proposed" } else { "proposed" };
            println!("  {}. {} ({})", i + 1, output::truncate_address(&gp.proposal_id), action,);
            println!("     {}", format_governor_proposal_details(gp));
        }
        println!();
    }

    // Skipped deployments
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

    // Collisions
    if !result.collisions.is_empty() {
        println!("Collisions detected: {}", result.collisions.len());
        println!();
    }

    // Summary
    let dep_count = result.deployments.len();
    let tx_count = result.transactions.len();
    let skip_count = result.skipped.len();
    let proposal_count = result.governor_proposals.len();

    if dep_count == 0 && skip_count == 0 && proposal_count == 0 {
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
            parts.push(format!("{} transaction{}", tx_count, if tx_count == 1 { "" } else { "s" }));
        }
        if proposal_count > 0 {
            let proposal_verb = if result.dry_run { "would be proposed" } else { "proposed" };
            parts.push(format!(
                "{} governor proposal{} {}",
                proposal_count,
                if proposal_count == 1 { "" } else { "s" },
                proposal_verb,
            ));
        }
        if skip_count > 0 {
            parts.push(format!("{} skipped", skip_count));
        }
        if result.gas_used > 0 {
            let gas_verb = if result.dry_run { "gas would be used" } else { "gas used" };
            parts.push(format!("{} {}", output::format_gas(result.gas_used), gas_verb));
        }
        println!("{}", parts.join(", "));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use treb_core::types::{GovernorProposal, ProposalStatus};
    use treb_forge::in_memory_signer;

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
}
