//! `treb run` command implementation.

use std::{
    collections::{BTreeMap, HashMap},
    env,
    io::{self, BufRead, IsTerminal, Write},
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
    sender::resolve_all_senders,
};
use treb_registry::Registry;

use console::Term;

use crate::{
    output,
    ui::{badge, color, selector::fuzzy_select_network, tree::TreeNode},
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
fn resolve_rpc_url_for_chain_id(network_or_url: &str, cwd: &std::path::Path) -> Option<String> {
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
async fn fetch_chain_id(rpc_url: &str) -> anyhow::Result<u64> {
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
    json: bool,
    env_vars: Vec<String>,
    target_contract: Option<String>,
    non_interactive: bool,
) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    ensure_initialized(&cwd)?;

    // ── Inject environment variables (before config resolution) ──────────
    inject_env_vars(&env_vars)?;

    // ── Interactive network selection when --network is omitted ───────────
    let network = if network.is_none() && !non_interactive && Term::stdout().is_term() {
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

    // ── Verbose context output ───────────────────────────────────────────
    if verbose && !json {
        eprintln!("Config source: {}", resolved.config_source);
        eprintln!("Namespace: {}", resolved.namespace);
        if let Some(ref url) = effective_rpc_url {
            eprintln!("RPC: {}", url);
        }
        for (role, sender) in &resolved_senders {
            eprintln!("Sender [{}]: {:?}", role, sender.sender_address());
        }
    }

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

    // ── Resolve chain ID from RPC ──────────────────────────────────────
    let chain_id = if let Some(ref network_or_url) = effective_rpc_url {
        let actual_url = resolve_rpc_url_for_chain_id(network_or_url, &cwd);
        if let Some(url) = actual_url { fetch_chain_id(&url).await.unwrap_or(0) } else { 0 }
    } else {
        0
    };

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

    let pipeline_context = PipelineContext {
        config: pipeline_config,
        script_path: PathBuf::from(script),
        git_commit,
        project_root: cwd.clone(),
        deployer_sender,
    };

    // ── Broadcast confirmation prompt ──────────────────────────────────
    if broadcast && !dry_run {
        let should_prompt = !non_interactive && io::stdin().is_terminal();
        if should_prompt {
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
    }

    // ── Open registry and execute pipeline ───────────────────────────────
    let mut registry = Registry::open(&cwd).context("failed to open registry")?;

    if !json {
        output::print_stage("\u{1f528}", &format!("Compiling and executing {}...", script));
    }

    let pipeline = RunPipeline::new(pipeline_context).with_script_config(script_config);

    if !json && !dry_run && broadcast {
        output::print_stage("\u{1f4e1}", "Broadcasting...");
    } else if !json && dry_run {
        output::print_stage("\u{1f9ea}", "Simulating...");
    }

    let result = pipeline.execute(&mut registry).await.context("pipeline execution failed")?;

    if !json {
        output::print_stage("\u{2705}", "Execution complete.");
    }

    // ── Verbose post-execution output ────────────────────────────────────
    if verbose && !json {
        if !result.console_logs.is_empty() {
            eprintln!("Console output: {} line(s)", result.console_logs.len());
        }
        eprintln!(
            "Result: {} deployment(s), {} transaction(s), {} skipped",
            result.deployments.len(),
            result.transactions.len(),
            result.skipped.len()
        );
    }

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
    gas_used: u64,
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
        gas_used: result.gas_used,
        console_logs: result.console_logs.clone(),
    };

    output::print_json(&output)?;
    Ok(())
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

        if let Some(group) = type_groups
            .iter_mut()
            .find(|g| g.deployment_type == d.deployment_type)
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
        let impl_label =
            format!("Implementation {}", output::truncate_address(&pi.implementation));
        node = node.child(TreeNode::new(impl_label));
    }
    node
}

fn display_result_human(result: &PipelineResult) {
    // Dry-run banner
    if result.dry_run {
        output::print_warning_banner("\u{1f6a7}", "[DRY RUN] No changes were written to the registry.");
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
                let mut chain_node =
                    TreeNode::new(chain_id.to_string()).with_style(color::CHAIN);
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
}
