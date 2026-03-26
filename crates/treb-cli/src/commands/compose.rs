//! `treb compose` command implementation.
//!
//! YAML-based multi-step deployment orchestration that executes multiple
//! Forge scripts in dependency order.

use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    env,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, bail};
use owo_colors::{OwoColorize, Style};
use serde::{Deserialize, Serialize};
use treb_config::{ResolveOpts, resolve_config};
use treb_forge::{
    pipeline::{
        self, PipelineConfig, PipelineContext, PipelineResult,
        compose_plan::{self, ComponentStatus as PlanComponentStatus, ComposePlan},
        resolve_git_commit,
    },
    script::{apply_deployed_libraries, build_script_config_with_senders},
    sender::resolve_all_senders,
    sender_config::encode_sender_configs,
};
use treb_registry::{Registry, addressbook_path, solidity_registry_path};

use crate::{
    output,
    ui::{color, emoji, interactive::is_non_interactive},
};

// ── Compose file schema ──────────────────────────────────────────────────

/// Top-level compose file structure.
#[derive(Debug, Deserialize, Serialize)]
pub struct ComposeFile {
    /// Deployment group name.
    pub group: String,
    /// Map of component name → component definition (sorted for determinism).
    pub components: BTreeMap<String, Component>,
}

/// A single component in the compose file.
#[derive(Debug, Deserialize, Serialize)]
pub struct Component {
    /// Path to the Forge script (e.g., `script/Deploy.s.sol`).
    pub script: String,
    /// Names of components this one depends on (must execute first).
    #[serde(default)]
    pub deps: Option<Vec<String>>,
    /// Per-component environment variables (merged with global `--env`).
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    /// Function signature to call (defaults to `run()` at execution time).
    #[serde(default)]
    pub sig: Option<String>,
    /// Arguments to pass to the script function.
    #[serde(default)]
    pub args: Option<Vec<String>>,
    /// Per-component verify override (overrides global `--verify` when set).
    #[serde(default)]
    pub verify: Option<bool>,
}

// ── Parsing and validation ───────────────────────────────────────────────

/// Load and parse a compose YAML file from disk.
pub fn load_compose_file(path: &str) -> anyhow::Result<ComposeFile> {
    let path = Path::new(path);
    if !path.exists() {
        bail!("compose file not found: {}", path.display());
    }
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read compose file: {}", path.display()))?;
    let compose: ComposeFile = serde_yaml::from_str(&contents)
        .with_context(|| format!("failed to parse compose file: {}", path.display()))?;
    Ok(compose)
}

/// Validate a parsed compose file.
///
/// Checks structural invariants that serde alone cannot enforce:
/// non-empty group, non-empty components, valid script paths,
/// valid dependency references, no self-dependencies, and valid
/// component names.
pub fn validate_compose_file(compose: &ComposeFile) -> anyhow::Result<()> {
    if compose.group.is_empty() {
        bail!("invalid orchestration configuration: group name is required");
    }
    if compose.components.is_empty() {
        bail!("invalid orchestration configuration: at least one component is required");
    }
    for (name, component) in &compose.components {
        // Validate component name: alphanumeric, hyphens, underscores only
        if !name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
            bail!(
                "invalid orchestration configuration: component '{}' has an invalid name: must contain only alphanumeric characters, hyphens, and underscores",
                name
            );
        }
        // Validate script is non-empty
        if component.script.is_empty() {
            bail!(
                "invalid orchestration configuration: component '{}' must specify a script",
                name
            );
        }
        // Validate dependency references
        if let Some(deps) = &component.deps {
            for dep in deps {
                if dep == name {
                    bail!(
                        "invalid orchestration configuration: component '{}' cannot depend on itself",
                        name
                    );
                }
                if !compose.components.contains_key(dep) {
                    bail!(
                        "invalid orchestration configuration: component '{}' depends on non-existent component '{}'",
                        name,
                        dep
                    );
                }
            }
        }
    }
    Ok(())
}

// ── Resume plan helpers ─────────────────────────────────────────────────

fn queued_file_for_broadcast_path(path: &str) -> Option<String> {
    let path = Path::new(path);
    let stem = path.file_stem()?.to_string_lossy();
    Some(path.with_file_name(format!("{stem}.queued.json")).to_string_lossy().to_string())
}

async fn resolve_resume_chain_id(
    cwd: &Path,
    network: Option<&str>,
    rpc_url: Option<&str>,
) -> Option<u64> {
    if let Some(url) = rpc_url {
        return super::run::fetch_chain_id(url).await.ok();
    }
    if let Some(net) = network {
        if let Some(url) = super::run::resolve_rpc_url_for_chain_id(net, cwd) {
            return super::run::fetch_chain_id(&url).await.ok();
        }
    }
    None
}

fn load_resume_plan(
    project_root: &Path,
    compose_file: &str,
    preferred_chain_id: Option<u64>,
) -> Option<ComposePlan> {
    if let Some(chain_id) = preferred_chain_id {
        if let Some(plan) = compose_plan::load_plan(project_root, compose_file, chain_id) {
            return Some(plan);
        }
    }

    let plans_dir =
        project_root.join("broadcast").join(compose_plan::compose_artifact_dir_name(compose_file));

    let mut plans = Vec::new();
    for entry in std::fs::read_dir(plans_dir).ok()? {
        let entry = entry.ok()?;
        if !entry.file_type().ok()?.is_dir() {
            continue;
        }
        let path = entry.path().join("compose-latest.json");
        if !path.exists() {
            continue;
        }
        let contents = std::fs::read_to_string(&path).ok()?;
        let plan: ComposePlan = serde_json::from_str(&contents).ok()?;
        plans.push(plan);
    }

    if plans.len() == 1 { plans.pop() } else { None }
}

// ── Dependency graph and execution order ─────────────────────────────────

/// Build a valid execution order via topological sort (Kahn's algorithm).
///
/// Returns component names in an order where all dependencies of a component
/// appear before it. Independent components are ordered alphabetically for
/// determinism. Returns an error if a dependency cycle is detected.
pub fn build_execution_order(compose: &ComposeFile) -> anyhow::Result<Vec<String>> {
    // Build in-degree map and adjacency list.
    let mut in_degree: BTreeMap<&str, usize> = BTreeMap::new();
    let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();

    for name in compose.components.keys() {
        in_degree.entry(name.as_str()).or_insert(0);
        dependents.entry(name.as_str()).or_default();
    }

    for (name, component) in &compose.components {
        if let Some(deps) = &component.deps {
            *in_degree.entry(name.as_str()).or_insert(0) += deps.len();
            for dep in deps {
                dependents.entry(dep.as_str()).or_default().push(name.as_str());
            }
        }
    }

    // Seed queue with zero-degree nodes (alphabetically sorted via BTreeMap).
    let mut queue: VecDeque<&str> =
        in_degree.iter().filter(|&(_, deg)| *deg == 0).map(|(&name, _)| name).collect();

    let mut order: Vec<String> = Vec::with_capacity(compose.components.len());

    while let Some(current) = queue.pop_front() {
        order.push(current.to_string());

        // Collect and sort dependents for deterministic ordering.
        let mut next: Vec<&str> =
            dependents.get(current).map(|v| v.as_slice()).unwrap_or_default().to_vec();
        next.sort();

        for dep in next {
            let deg = in_degree.get_mut(dep).unwrap();
            *deg -= 1;
            if *deg == 0 {
                queue.push_back(dep);
            }
        }
    }

    if order.len() != compose.components.len() {
        // Find components still in the cycle (non-zero in-degree).
        let cycle_members: Vec<&str> =
            in_degree.iter().filter(|&(_, deg)| *deg > 0).map(|(&name, _)| name).collect();
        bail!("circular dependency detected involving components: [{}]", cycle_members.join(", "));
    }

    Ok(order)
}

/// An entry in the dry-run execution plan.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanEntry {
    pub step: usize,
    pub component: String,
    pub script: String,
    pub deps: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub skipped: bool,
}

/// Build the execution plan for display.
fn build_plan(
    compose: &ComposeFile,
    order: &[String],
    skip_set: &HashSet<String>,
) -> Vec<PlanEntry> {
    order
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let component = &compose.components[name];
            PlanEntry {
                step: i + 1,
                component: name.clone(),
                script: component.script.clone(),
                deps: component.deps.as_ref().cloned().unwrap_or_default(),
                skipped: skip_set.contains(name),
            }
        })
        .collect()
}

fn format_env_map(env: &HashMap<String, String>) -> String {
    let mut entries: Vec<_> = env.iter().collect();
    entries.sort_by_key(|(key, _)| *key);

    let rendered =
        entries.into_iter().map(|(key, value)| format!("{key:?}: {value:?}")).collect::<Vec<_>>();
    format!("{{{}}}", rendered.join(", "))
}

fn format_execution_plan_header_lines(compose: &ComposeFile, plan_len: usize) -> [String; 3] {
    let lines = [
        format!("{} Orchestrating {}", emoji::TARGET, compose.group),
        format!("{} Execution plan: {} components", emoji::CLIPBOARD, plan_len),
        format!("{} Execution Plan:", emoji::CLIPBOARD),
    ];

    if color::is_color_enabled() { lines.map(|line| styled(&line, color::STAGE)) } else { lines }
}

/// Display the execution plan in human-readable format (matches Go `RenderExecutionPlan`).
/// Print the compose banner with plan, network, namespace, senders, and components.
#[allow(clippy::too_many_arguments)]
fn print_compose_banner(
    compose: &ComposeFile,
    plan: &[PlanEntry],
    network: Option<&str>,
    chain_id: u64,
    namespace: &str,
    is_fork: bool,
    broadcast: bool,
    dry_run: bool,
    senders: &[(String, String)], // (role, address) sorted
) {
    let use_color = color::is_color_enabled();
    let separator = "─".repeat(50);

    if use_color {
        eprintln!("{}", separator.style(color::GRAY));
    } else {
        eprintln!("{separator}");
    }

    // Plan
    let compose_file_display = format!("{} ({})", compose.group, plan.len());
    if use_color {
        eprintln!("  {:12} {}", "Plan:", compose_file_display.style(color::CYAN));
    } else {
        eprintln!("  {:12} {}", "Plan:", compose_file_display);
    }

    // Network
    let network_name = network.unwrap_or("(none)");
    if use_color {
        let chain_suffix = if chain_id > 0 {
            format!(" {}", format!("({})", chain_id).style(color::GRAY))
        } else {
            String::new()
        };
        let fork_suffix =
            if is_fork { format!(" {}", "[fork]".style(color::MAGENTA)) } else { String::new() };
        eprintln!(
            "  {:12} {}{}{}",
            "Network:",
            network_name.style(color::BLUE),
            chain_suffix,
            fork_suffix
        );
    } else {
        let fork_tag = if is_fork { " [fork]" } else { "" };
        if chain_id > 0 {
            eprintln!("  {:12} {} ({}){}", "Network:", network_name, chain_id, fork_tag);
        } else {
            eprintln!("  {:12} {}{}", "Network:", network_name, fork_tag);
        }
    }

    // Namespace
    if use_color {
        eprintln!("  {:12} {}", "Namespace:", namespace.style(color::MAGENTA));
    } else {
        eprintln!("  {:12} {}", "Namespace:", namespace);
    }

    // Mode
    let (mode_label, mode_style) = super::run::deployment_banner_mode(dry_run, broadcast, is_fork);
    if use_color {
        eprintln!("  {:12} {}", "Mode:", mode_label.style(mode_style));
    } else {
        eprintln!("  {:12} {}", "Mode:", mode_label);
    }

    // Senders — role name in normal color, address in gray
    if !senders.is_empty() {
        let max_role = senders.iter().map(|(r, _)| r.len()).max().unwrap_or(0);
        for (i, (role, addr)) in senders.iter().enumerate() {
            let label = if i == 0 { "Senders:" } else { "" };
            if use_color {
                eprintln!(
                    "  {:12} {:<width$}  {}",
                    label,
                    role,
                    addr.style(color::GRAY),
                    width = max_role,
                );
            } else {
                eprintln!("  {:12} {:<width$}  {}", label, role, addr, width = max_role);
            }
        }
    }

    // Components — [N] name in normal color, script/deps in gray
    for (i, entry) in plan.iter().enumerate() {
        let label = if i == 0 { "Components:" } else { "" };
        if use_color {
            let mut suffix = format!(" → {}", entry.script);
            if !entry.deps.is_empty() {
                suffix = format!("{} (depends on: [{}])", suffix, entry.deps.join(", "));
            }
            if entry.skipped {
                suffix = format!("{} (skipped)", suffix);
            }
            eprintln!(
                "  {:12} {} {} {}",
                label,
                format!("[{}]", entry.step).style(color::GRAY),
                entry.component,
                suffix.style(color::GRAY),
            );
        } else {
            let mut line = format!("[{}] {} → {}", entry.step, entry.component, entry.script);
            if !entry.deps.is_empty() {
                line = format!("{} (depends on: [{}])", line, entry.deps.join(", "));
            }
            if entry.skipped {
                line = format!("{} (skipped)", line);
            }
            eprintln!("  {:12} {}", label, line);
        }
    }

    if use_color {
        eprintln!("{}", separator.style(color::GRAY));
    } else {
        eprintln!("{separator}");
    }
}

fn print_execution_plan(compose: &ComposeFile, plan: &[PlanEntry]) {
    let [orchestration_header, plan_summary_header, plan_header] =
        format_execution_plan_header_lines(compose, plan.len());

    eprintln!("\n{orchestration_header}");
    eprintln!("{plan_summary_header}\n");
    eprintln!("{plan_header}");
    eprintln!("{}", "─".repeat(50));
    for entry in plan {
        if entry.skipped {
            eprint!(
                "{}. {} → {}",
                entry.step,
                styled(&entry.component, color::WARNING),
                styled(&entry.script, color::GREEN),
            );
            if !entry.deps.is_empty() {
                eprint!(
                    " {}",
                    styled(&format!("(depends on: [{}])", entry.deps.join(", ")), color::GRAY,),
                );
            }
            eprint!(" {}", styled("(skipped)", color::WARNING));
        } else {
            eprint!(
                "{}. {} → {}",
                entry.step,
                styled(&entry.component, color::CYAN),
                styled(&entry.script, color::GREEN),
            );
            if !entry.deps.is_empty() {
                eprint!(
                    " {}",
                    styled(&format!("(depends on: [{}])", entry.deps.join(", ")), color::GRAY,),
                );
            }
        }
        eprintln!();
        if let Some(env) =
            compose.components.get(&entry.component).and_then(|component| component.env.as_ref())
        {
            if !env.is_empty() {
                eprintln!(
                    "   {}",
                    styled(&format!("Env: {}", format_env_map(env)), color::WARNING)
                );
            }
        }
    }
    eprintln!();
}

fn print_resume_banner(start_step: usize, total: usize) {
    eprintln!("{} Resuming compose from step {} of {}", emoji::OPEN_FOLDER, start_step, total);
}

fn should_prompt_for_broadcast_confirmation(
    broadcast: bool,
    dry_run: bool,
    prompts_enabled: bool,
    executing_count: usize,
) -> bool {
    broadcast && !dry_run && prompts_enabled && executing_count > 0
}

fn should_reject_interactive_json_broadcast(
    broadcast: bool,
    dry_run: bool,
    json: bool,
    prompts_enabled: bool,
    executing_count: usize,
) -> bool {
    json && should_prompt_for_broadcast_confirmation(
        broadcast,
        dry_run,
        prompts_enabled,
        executing_count,
    )
}

// ── Result aggregation ───────────────────────────────────────────────────

/// Status of a component in compose results.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentStatus {
    Success,
    Skipped,
    Failed,
    NotExecuted,
}

impl std::fmt::Display for ComponentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ComponentStatus::Success => write!(f, "success"),
            ComponentStatus::Skipped => write!(f, "skipped"),
            ComponentStatus::Failed => write!(f, "failed"),
            ComponentStatus::NotExecuted => write!(f, "not executed"),
        }
    }
}

/// Per-component result summary.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentResultEntry {
    pub component: String,
    pub status: ComponentStatus,
    pub deployments: usize,
    pub transactions: usize,
    pub gas_used: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Aggregate totals across all components.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeTotals {
    pub deployments: usize,
    pub transactions: usize,
    pub gas_used: u64,
    pub succeeded: usize,
    pub skipped: usize,
    pub failed: usize,
    pub not_executed: usize,
}

/// Full compose result for JSON output.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ComposeOutputJson {
    group: String,
    success: bool,
    components: Vec<ComponentResultEntry>,
    totals: ComposeTotals,
}

/// Compute aggregate totals from component results.
fn compute_totals(results: &[ComponentResultEntry]) -> ComposeTotals {
    let mut totals = ComposeTotals {
        deployments: 0,
        transactions: 0,
        gas_used: 0,
        succeeded: 0,
        skipped: 0,
        failed: 0,
        not_executed: 0,
    };
    for r in results {
        totals.deployments += r.deployments;
        totals.transactions += r.transactions;
        totals.gas_used += r.gas_used;
        match r.status {
            ComponentStatus::Success => totals.succeeded += 1,
            ComponentStatus::Skipped => totals.skipped += 1,
            ComponentStatus::Failed => totals.failed += 1,
            ComponentStatus::NotExecuted => totals.not_executed += 1,
        }
    }
    totals
}

/// Apply a color style when color is enabled, plain text otherwise.
fn styled(text: &str, style: Style) -> String {
    if color::is_color_enabled() { format!("{}", text.style(style)) } else { text.to_string() }
}

/// Display compose summary in human-readable format (matches Go renderSummary).
fn display_compose_human(
    _group: &str,
    results: &[ComponentResultEntry],
    _totals: &ComposeTotals,
    completed: usize,
    total: usize,
    failed_component: &Option<String>,
) {
    let success = failed_component.is_none();

    if success {
        // Success case: no extra output needed — broadcast phase already
        // showed per-component results with tx receipts.
    } else {
        let separator = "═".repeat(70);
        eprintln!("{separator}");
        eprintln!("{}", styled(&format!("{} Orchestration failed", emoji::CROSS), color::ERROR,),);
        eprintln!("\n{} Summary:", emoji::CHART);
        if let Some(failed) = failed_component {
            eprintln!("  • Failed at step: {}", failed);
        }
        // completed count excludes the failed step (Go behavior)
        eprintln!("  • Steps completed: {}/{}", completed, total);
        // Show the error message from the failed component
        if let Some(failed) = failed_component {
            if let Some(entry) = results.iter().find(|r| &r.component == failed) {
                if let Some(ref err) = entry.error {
                    eprintln!("  • Error: {}", err);
                }
            }
        }
    }
}

/// Display compose results as JSON.
fn display_compose_json(
    group: &str,
    results: Vec<ComponentResultEntry>,
    totals: ComposeTotals,
    success: bool,
) -> anyhow::Result<()> {
    let output =
        ComposeOutputJson { group: group.to_string(), success, components: results, totals };
    output::print_json(&output)?;
    Ok(())
}

// ── Per-component config setup ───────────────────────────────────────────

/// Resolved configuration for a single compose component.
struct ComponentSetup {
    pipeline_context: PipelineContext,
    script_config: treb_forge::script::ScriptConfig,
}

/// Common parameters shared across all components in a compose run.
struct ComposeParams<'a> {
    cwd: &'a std::path::Path,
    namespace: &'a Option<String>,
    network: &'a Option<String>,
    rpc_url: &'a Option<String>,
    profile: &'a Option<String>,
    env_vars: &'a [String],
    broadcast: bool,
    slow: bool,
    legacy: bool,
    verify: bool,
    verbose: u8,
}

/// Set up environment and build pipeline config for a single component.
///
/// Called once for simulation (with the compose pipeline's ephemeral URL override)
/// and once for broadcast (with the original upstream URL).
async fn setup_component(
    name: &str,
    component: &Component,
    params: &ComposeParams<'_>,
) -> anyhow::Result<ComponentSetup> {
    // Re-inject global env vars (reset any previous component overrides).
    super::run::inject_env_vars(params.env_vars)?;
    if let Some(env_map) = &component.env {
        for (key, value) in env_map {
            unsafe { env::set_var(key, value) };
        }
    }

    let resolved = resolve_config(ResolveOpts {
        project_root: params.cwd.to_path_buf(),
        namespace: params.namespace.clone(),
        network: params.network.clone(),
        profile: params.profile.clone(),
        sender_overrides: HashMap::new(),
    })
    .with_context(|| format!("failed to resolve config for component '{}'", name))?;

    let mut effective_rpc_url = params.rpc_url.clone().or_else(|| resolved.network.clone());
    let effective_network = params.network.clone().or_else(|| resolved.network.clone());

    // Fork detection
    let is_fork = {
        let net = effective_network.as_deref();
        let mut store = treb_registry::ForkStateStore::new(&params.cwd.join(super::run::TREB_DIR));
        if store.load().is_ok() {
            if let Some(fork_entry) = net.and_then(|n| store.get_active_fork(n).cloned()) {
                effective_rpc_url = Some(fork_entry.rpc_url.clone());
                if let Some(ref net) = effective_network {
                    if let Ok(endpoints) = treb_config::resolve_rpc_endpoints(params.cwd) {
                        if let Some(endpoint) = endpoints.get(net.as_str()) {
                            if let Some(var) = super::run::extract_env_var_name(&endpoint.raw_url) {
                                unsafe { env::set_var(var, &fork_entry.rpc_url) };
                            }
                        }
                    }
                }
                true
            } else {
                false
            }
        } else {
            false
        }
    };

    // Sender resolution + v2 env var injection
    let resolved_senders = resolve_all_senders(&resolved.senders)
        .await
        .with_context(|| format!("failed to resolve senders for component '{}'", name))?;

    unsafe { env::set_var("NAMESPACE", &resolved.namespace) };
    if let Some(ref net) = effective_network {
        unsafe { env::set_var("NETWORK", net) };
    }
    unsafe {
        env::set_var("REGISTRY_FILE", solidity_registry_path(params.cwd));
        env::set_var("ADDRESSBOOK_FILE", addressbook_path(params.cwd));
    }
    let encoded_senders = encode_sender_configs(&resolved_senders);
    unsafe { env::set_var("SENDER_CONFIGS", &encoded_senders) };

    // Build ScriptConfig (all wallet keys injected)
    let mut script_config =
        build_script_config_with_senders(&resolved, &component.script, &resolved_senders)
            .with_context(|| format!("failed to build script config for '{}'", name))?;

    let sig = component.sig.as_deref().unwrap_or("run()");
    let args = component.args.clone().unwrap_or_default();
    let effective_verify = component.verify.unwrap_or(params.verify);

    script_config
        .sig(sig)
        .args(args)
        .broadcast(params.broadcast)
        .dry_run(false)
        .slow(params.slow || resolved.slow)
        .legacy(params.legacy)
        .verify(effective_verify)
        .non_interactive(true);

    if let Some(ref url) = effective_rpc_url {
        script_config.rpc_url(url);
    }

    // Resolve chain ID
    let chain_id = if let Some(ref url) = effective_rpc_url {
        let actual = super::run::resolve_rpc_url_for_chain_id(url, params.cwd);
        if let Some(u) = actual { super::run::fetch_chain_id(&u).await.unwrap_or(0) } else { 0 }
    } else {
        0
    };

    // Build sender labels
    let sender_role_names: Vec<String> = resolved_senders.keys().cloned().collect();
    let sender_labels = resolved_senders
        .iter()
        .map(|(role, sender)| (sender.broadcast_address(), role.clone()))
        .collect();
    let pipeline_config = PipelineConfig {
        script_path: component.script.clone(),
        broadcast: params.broadcast,
        namespace: resolved.namespace.clone(),
        chain_id,
        script_sig: sig.to_string(),
        script_args: Vec::new(),
        verbosity: params.verbose,
        is_fork,
        rpc_url: effective_rpc_url.clone(),
        quiet: false,
        ..Default::default()
    };

    let pipeline_context = PipelineContext {
        config: pipeline_config,
        script_path: PathBuf::from(&component.script),
        git_commit: resolve_git_commit(),
        project_root: params.cwd.to_path_buf(),
        resolved_senders,
        sender_configs: resolved.senders.clone(),
        sender_labels,
        sender_role_names,
    };

    Ok(ComponentSetup { pipeline_context, script_config })
}

// ── Command entry point ──────────────────────────────────────────────────

/// Execute a compose deployment pipeline.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    file: String,
    network: Option<String>,
    rpc_url: Option<String>,
    namespace: Option<String>,
    profile: Option<String>,
    broadcast: bool,
    dry_run: bool,
    resume: bool,
    verify: bool,
    slow: bool,
    legacy: bool,
    verbose: u8,
    dump_command: bool,
    json: bool,
    env_vars: Vec<String>,
    non_interactive: bool,
) -> anyhow::Result<()> {
    // Parse and validate the compose file.
    let compose = load_compose_file(&file)?;
    validate_compose_file(&compose)?;

    // Build execution order (topological sort).
    let order = build_execution_order(&compose)?;

    let cwd = env::current_dir().context("failed to determine current directory")?;
    let scope = super::resolve_command_scope(&cwd, namespace, network)?;
    let namespace = scope.namespace;
    let network = scope.network;

    if network.is_none() && rpc_url.is_none() {
        bail!(
            "no network configured; pass --network, set one with 'treb config set network <name>', or use --rpc-url"
        );
    }

    // ── Resume plan handling ──────────────────────────────────────────
    let compose_contents = std::fs::read_to_string(&file)
        .with_context(|| format!("failed to read compose file: {}", file))?;
    let compose_hash = compose_plan::compute_compose_hash(&compose_contents);

    let resume_plan = if resume {
        let preferred_chain_id =
            resolve_resume_chain_id(&cwd, network.as_deref(), rpc_url.as_deref()).await;
        load_resume_plan(&cwd, &file, preferred_chain_id)
    } else {
        None
    };

    let skip_set: HashSet<String> = if let Some(ref plan) = resume_plan {
        if !compose_plan::plan_matches_compose(plan, &compose_hash) && !json {
            eprintln!("Warning: compose file changed since last run; resuming anyway");
        }
        plan.components
            .iter()
            .filter(|component| component.status == PlanComponentStatus::Broadcast)
            .map(|component| component.name.clone())
            .collect()
    } else {
        HashSet::new()
    };

    let resume_broadcast_paths: HashMap<String, PathBuf> = resume_plan
        .as_ref()
        .map(|plan| {
            plan.components
                .iter()
                .filter_map(|component| {
                    component
                        .broadcast_file
                        .as_ref()
                        .map(|path| (component.name.clone(), cwd.join(path)))
                })
                .collect()
        })
        .unwrap_or_default();

    // ── Verbose resume context ────────────────────────────────────────
    if verbose > 0 && !json && resume && !skip_set.is_empty() {
        let hash_str = &compose_hash;
        let skip_count = skip_set.len().to_string();
        let kv_pairs: Vec<(&str, &str)> =
            vec![("Compose hash", hash_str), ("Skipping", &skip_count)];
        output::eprint_kv(&kv_pairs);
        eprintln!();
    }

    // Dry-run: show execution plan and exit.
    if dry_run {
        let plan = build_plan(&compose, &order, &skip_set);
        if json {
            output::print_json(&plan)?;
        } else {
            eprintln!(
                "{}",
                output::format_warning_banner(
                    "\u{1f6a7}",
                    "[DRY RUN] Showing execution plan only — no changes will be made."
                )
            );
            eprintln!();
            print_execution_plan(&compose, &plan);
        }
        return Ok(());
    }

    // ── Project initialization check ──────────────────────────────────
    super::run::ensure_initialized(&cwd)?;

    // ── Dump command: print per-component forge commands and exit ─────
    if dump_command {
        let registry = Registry::open(&cwd).context("failed to open registry")?;
        let dump_params = ComposeParams {
            cwd: &cwd,
            namespace: &namespace,
            network: &network,
            rpc_url: &rpc_url,
            profile: &profile,
            env_vars: &env_vars,
            broadcast,
            slow,
            legacy,
            verify,
            verbose,
        };
        for name in &order {
            let component = &compose.components[name];

            if skip_set.contains(name) {
                if !json {
                    println!("# {} (skipped)", name);
                }
                continue;
            }

            let mut setup = setup_component(name, component, &dump_params)
                .await
                .with_context(|| format!("failed to set up component '{}'", name))?;

            apply_deployed_libraries(
                &mut setup.script_config,
                &registry,
                &setup.pipeline_context.config.namespace,
                setup.pipeline_context.config.chain_id,
            );

            let cmd_parts = setup.script_config.to_forge_command();
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
            if !json {
                println!("# {}", name);
                println!("{}", cmd_str);
            }
        }
        return Ok(());
    }

    let prompts_enabled = !is_non_interactive(non_interactive);
    let executing_count = order.iter().filter(|name| !skip_set.contains(*name)).count();

    // ── Reject interactive broadcast in JSON mode ─────────────────────
    if should_reject_interactive_json_broadcast(
        broadcast,
        dry_run,
        json,
        prompts_enabled,
        executing_count,
    ) {
        bail!(
            "interactive broadcast confirmation is not available in JSON mode; rerun with --non-interactive"
        );
    }

    // Broadcast confirmation is handled in Phase 4 after simulation.

    // ── Open registry ─────────────────────────────────────────────────
    let mut registry = Registry::open(&cwd).context("failed to open registry")?;

    // ── Execute components in topological order ───────────────────────
    let total = order.len();
    let mut completed = skip_set.len();
    let mut component_results: Vec<ComponentResultEntry> = Vec::with_capacity(total);
    let mut failed_component: Option<String> = None;
    let resume_step = order.iter().position(|name| !skip_set.contains(name)).map(|index| index + 1);

    // Resolve config once for the banner (all components share the same
    // network/namespace/senders).
    super::run::inject_env_vars(&env_vars)?;
    let banner_resolved = resolve_config(ResolveOpts {
        project_root: cwd.clone(),
        namespace: namespace.clone(),
        network: network.clone(),
        profile: profile.clone(),
        sender_overrides: HashMap::new(),
    })
    .context("failed to resolve configuration")?;
    let banner_senders =
        resolve_all_senders(&banner_resolved.senders).await.context("failed to resolve senders")?;
    let banner_network = network.as_deref().or(banner_resolved.network.as_deref());

    // Resolve chain ID for the banner
    let banner_chain_id = if let Some(net) = banner_network {
        let actual = super::run::resolve_rpc_url_for_chain_id(net, &cwd);
        if let Some(url) = actual { super::run::fetch_chain_id(&url).await.unwrap_or(0) } else { 0 }
    } else {
        0
    };

    // Detect fork mode and resolve target RPC URL for banner/confirmation
    let (banner_is_fork, banner_rpc_url) = {
        let net = banner_network;
        let mut store = treb_registry::ForkStateStore::new(&cwd.join(super::run::TREB_DIR));
        if store.load().is_ok() {
            if let Some(fork_entry) = net.and_then(|n| store.get_active_fork(n).cloned()) {
                (true, Some(fork_entry.rpc_url))
            } else {
                let url = net.and_then(|n| super::run::resolve_rpc_url_for_chain_id(n, &cwd));
                (false, url)
            }
        } else {
            let url = net.and_then(|n| super::run::resolve_rpc_url_for_chain_id(n, &cwd));
            (false, url)
        }
    };

    // Auto-fund senders on fork
    if banner_is_fork {
        if let Some(ref rpc) = banner_rpc_url {
            let _ = treb_forge::fund_senders_on_fork(rpc, &banner_senders, 10_000).await;
        }
    }

    // Build sorted sender list for banner
    let mut banner_sender_list: Vec<(String, String)> = banner_senders
        .iter()
        .map(|(role, sender)| (role.clone(), format!("{:#x}", sender.sender_address())))
        .collect();
    banner_sender_list.sort_by(|a, b| a.0.cmp(&b.0));

    if !json {
        let plan = build_plan(&compose, &order, &skip_set);
        print_compose_banner(
            &compose,
            &plan,
            banner_network,
            banner_chain_id,
            &banner_resolved.namespace,
            banner_is_fork,
            broadcast,
            dry_run,
            &banner_sender_list,
        );
        if resume {
            if let Some(step) = resume_step {
                print_resume_banner(step, total);
            }
        }
    }

    // Track skipped components in results
    for name in &order {
        if skip_set.contains(name) {
            component_results.push(ComponentResultEntry {
                component: name.clone(),
                status: ComponentStatus::Skipped,
                deployments: 0,
                transactions: 0,
                gas_used: 0,
                error: None,
            });
        }
    }

    // Shared params for component setup
    let compose_params = ComposeParams {
        cwd: &cwd,
        namespace: &namespace,
        network: &network,
        rpc_url: &rpc_url,
        profile: &profile,
        env_vars: &env_vars,
        broadcast,
        slow,
        legacy,
        verify,
        verbose,
    };

    // ── Build SessionPipeline with all non-skipped components ─────
    use treb_forge::pipeline::{
        ScriptEntry, SessionPhase, SessionPipeline, SessionProgressCallback,
    };

    let mut session = SessionPipeline::new();
    let mut components_to_run: Vec<String> = Vec::new();
    let mut components_for_plan: Vec<(String, String)> = Vec::new();
    let mut plan_chain_id: Option<u64> = None;
    for name in &order {
        if skip_set.contains(name) {
            continue;
        }
        let component = &compose.components[name];
        let setup = setup_component(name, component, &compose_params)
            .await
            .with_context(|| format!("failed to set up component '{}'", name))?;

        plan_chain_id.get_or_insert(setup.pipeline_context.config.chain_id);
        components_for_plan.push((name.clone(), component.script.clone()));
        session.add_script(ScriptEntry {
            name: name.clone(),
            context: setup.pipeline_context,
            config: setup.script_config,
        });
        components_to_run.push(name.clone());
    }

    if resume {
        session = session.with_resume(true).with_resume_broadcast_paths(resume_broadcast_paths);
    }

    // Simulation progress spinner
    let progress_spinner = crate::ui::spinner::create_spinner("");
    progress_spinner.disable_steady_tick(); // start hidden
    if !json {
        let sp = progress_spinner.clone();
        let progress_cb: SessionProgressCallback = Box::new(move |phase| {
            let msg: Option<String> = match phase {
                SessionPhase::Compiling => Some("Compiling".into()),
                SessionPhase::Simulating(ref name) => Some(format!("Executing {name}")),
                _ => None,
            };
            if let Some(msg) = msg {
                sp.set_message(msg);
                sp.enable_steady_tick(std::time::Duration::from_millis(80));
            } else {
                sp.set_message(String::new());
                sp.disable_steady_tick();
            }
        });
        session = session.with_progress(progress_cb);
    }

    // ── Phase 2: Simulate all with shared EVM ────────────────────
    let simulated = {
        let _foundry_shell = super::run::FoundryShellGuard::suppress();
        let r = session.simulate_all(&mut registry).await;
        progress_spinner.finish_and_clear();
        r
    };

    // Reload registry — simulate_all snapshots/restores the on-disk files,
    // so the in-memory Registry is stale.
    registry = Registry::open(&cwd).context("failed to reload registry")?;

    let mut simulated = match simulated {
        Ok(s) => s,
        Err((partial_results, failed_name, err)) => {
            for sr in &partial_results {
                if components_to_run.contains(&sr.name) {
                    completed += 1;
                    component_results.push(ComponentResultEntry {
                        component: sr.name.clone(),
                        status: ComponentStatus::Success,
                        deployments: sr.result.deployments.len(),
                        transactions: sr.result.transactions.len(),
                        gas_used: sr.result.gas_used,
                        error: None,
                    });
                }
            }
            let full = format!("{:#}", err);
            if !json {
                eprintln!("{full}");
            }
            let error_msg = full
                .lines()
                .rev()
                .find(|l| l.contains("[Revert]"))
                .map(|l| {
                    l.trim()
                        .trim_start_matches(|c: char| "└─├│ ← ".contains(c))
                        .trim()
                        .trim_start_matches("[Revert]")
                        .trim()
                        .to_string()
                })
                .unwrap_or_else(|| format!("{err}"));
            if !json {
                eprintln!(
                    "{}",
                    styled(&format!("{} Failed: {}", emoji::CROSS, error_msg), color::RED)
                );
            }
            component_results.push(ComponentResultEntry {
                component: failed_name.clone(),
                status: ComponentStatus::Failed,
                deployments: 0,
                transactions: 0,
                gas_used: 0,
                error: Some(error_msg),
            });
            failed_component = Some(failed_name.clone());
            let failed_idx = components_to_run.iter().position(|n| n == &failed_name).unwrap_or(0);
            for name in components_to_run.iter().skip(failed_idx + 1) {
                component_results.push(ComponentResultEntry {
                    component: name.clone(),
                    status: ComponentStatus::NotExecuted,
                    deployments: 0,
                    transactions: 0,
                    gas_used: 0,
                    error: None,
                });
            }

            // Jump to display results
            let totals = compute_totals(&component_results);
            let _success = false;
            if !json {
                display_compose_human(
                    &compose.group,
                    &component_results,
                    &totals,
                    completed,
                    total,
                    &failed_component,
                );
            }
            let failure_error = failed_component.as_ref().map(|failed| {
                anyhow::anyhow!(
                    "compose failed: component '{}' failed ({}/{} completed)",
                    failed,
                    completed,
                    total
                )
            });
            if let Some(err) = failure_error {
                return Err(err);
            }
            return Ok(());
        }
    };

    // ── Phase 3: Display combined simulation results ────────────
    if !json {
        let sim_results: Vec<_> = simulated.results().collect();
        let total_txs: usize = sim_results.iter().map(|(_, r)| r.transactions.len()).sum();
        let total_deps: usize = sim_results.iter().map(|(_, r)| r.deployments.len()).sum();

        if total_txs > 0 {
            let use_color = crate::ui::color::is_color_enabled();
            let sim_count = sim_results.len();
            let header = format!(
                "\n{} transaction{} across {} component{}:",
                total_txs,
                if total_txs == 1 { "" } else { "s" },
                sim_count,
                if sim_count == 1 { "" } else { "s" },
            );
            if use_color {
                eprintln!("{}", header.style(crate::ui::color::BOLD));
            } else {
                eprintln!("{header}");
            }

            let mut global_idx = 0usize;
            for (name, result) in &sim_results {
                if result.transactions.is_empty() {
                    continue;
                }
                if use_color {
                    eprintln!(
                        "\n  {} {}",
                        "──".style(crate::ui::color::GRAY),
                        name.style(crate::ui::color::CYAN),
                    );
                } else {
                    eprintln!("\n  ── {}", name);
                }

                for rt in &result.transactions {
                    let sender_label = super::run::tx_sender_label(rt);
                    if use_color {
                        eprintln!(
                            "  {} {}",
                            format!("{global_idx}:").style(crate::ui::color::GRAY),
                            format!("from={sender_label}").style(crate::ui::color::CYAN),
                        );
                    } else {
                        eprintln!("  {global_idx}: from={sender_label}");
                    }

                    if let Some(ref trace) = rt.trace {
                        for line in trace.lines() {
                            eprintln!("  {line}");
                        }
                    } else {
                        for op in &rt.transaction.operations {
                            let target = crate::output::truncate_address(&op.target);
                            let line = if op.method.is_empty() || op.method.starts_with("0x") {
                                format!("      {} {}", op.operation_type, target)
                            } else {
                                format!("      {} {}.{}()", op.operation_type, target, op.method)
                            };
                            if use_color {
                                eprintln!("{}", line.style(crate::ui::color::MUTED));
                            } else {
                                eprintln!("{line}");
                            }
                        }
                    }
                    global_idx += 1;
                }
            }
            eprintln!();
        }

        // Show collisions
        let total_collisions: usize = sim_results.iter().map(|(_, r)| r.collisions.len()).sum();
        if total_collisions > 0 {
            let use_color = crate::ui::color::is_color_enabled();
            eprintln!();
            for (_, result) in &sim_results {
                for c in &result.collisions {
                    let line = format!(
                        "  {} {} at {}",
                        emoji::WARNING,
                        c.contract_name,
                        c.existing_address,
                    );
                    if use_color {
                        eprintln!("{}", line.style(crate::ui::color::YELLOW));
                    } else {
                        eprintln!("{line}");
                    }
                }
            }
            eprintln!(
                "  {} collision(s) — contract(s) already deployed at predicted address",
                total_collisions,
            );
        }

        // Deployment summary
        if total_deps > 0 {
            let use_color = crate::ui::color::is_color_enabled();
            eprintln!("\n{} Deployments:", crate::ui::emoji::PACKAGE);
            for (_, result) in &sim_results {
                for rd in &result.deployments {
                    let d = &rd.deployment;
                    let mut name = d.contract_name.clone();
                    if !d.label.is_empty() {
                        name = format!("{name}:{}", d.label);
                    }
                    if use_color {
                        eprintln!(
                            "  {} at {}",
                            name.style(crate::ui::color::CYAN),
                            d.address.style(crate::ui::color::GREEN),
                        );
                    } else {
                        eprintln!("  {} at {}", name, d.address);
                    }
                }
            }
        }

        eprintln!(
            "\n{} Simulation complete: {} transaction(s), {} deployment(s) across {} component(s)",
            emoji::CHECK_MARK,
            total_txs,
            total_deps,
            sim_results.len()
        );
        eprintln!();
    }

    // ── Phase 4: Broadcast confirmation + broadcast ──────────────
    let total_txs: usize = simulated.results().map(|(_, r)| r.transactions.len()).sum();
    let should_broadcast = broadcast && total_txs > 0;

    let script_results = if should_broadcast {
        // Confirmation prompt (interactive mode only)
        if prompts_enabled && !json {
            let network_name = network.as_deref().or(banner_network).unwrap_or("unknown");
            let fork_tag = if banner_is_fork { " [fork]" } else { "" };
            let url_suffix =
                banner_rpc_url.as_deref().map(|u| format!(" ({u})")).unwrap_or_default();
            eprintln!(
                "\n  {} {}{}{}\n",
                styled("Target:", color::BOLD),
                network_name,
                fork_tag,
                styled(&url_suffix, color::GRAY),
            );

            let confirmed = crate::ui::prompt::confirm("Broadcast these transactions?", false);
            if !confirmed {
                eprintln!("Broadcast cancelled.");
                return Ok(());
            }
        }
        if !json {
            let network_label = network.as_deref().or(banner_network).unwrap_or("network");
            eprintln!("\n{}", styled(&format!("Broadcasting to {network_label}..."), color::CYAN),);
        }

        let compose_plan_state = Arc::new(Mutex::new(resume_plan.clone().unwrap_or_else(|| {
            compose_plan::create_plan(
                &file,
                &compose_hash,
                plan_chain_id.unwrap_or(banner_chain_id),
                &components_for_plan,
            )
        })));
        {
            let plan = compose_plan_state
                .lock()
                .map_err(|_| anyhow::anyhow!("compose plan lock poisoned"))?;
            compose_plan::save_plan(&cwd, &plan)?;
        }

        // Pre-broadcast fork snapshot (if in fork mode)
        if banner_is_fork {
            let treb_dir = cwd.join(super::run::TREB_DIR);
            let mut store = treb_registry::ForkStateStore::new(&treb_dir);
            if store.load().is_ok() && store.is_fork_mode_active() {
                let source = treb_core::types::fork::ForkRunSource::Compose {
                    file: file.clone(),
                    group: compose.group.clone(),
                    components: components_to_run.clone(),
                };
                super::fork::snapshot_fork_before_broadcast(&treb_dir, &mut store, source)
                    .await
                    .ok(); // best-effort
            }
        }

        // Broadcast with inline output via BroadcastDisplay
        let broadcast_display = crate::ui::broadcast_display::BroadcastDisplay::new(json);
        let cwd_for_plan = cwd.clone();
        let compose_plan_state_for_updates = Arc::clone(&compose_plan_state);
        simulated.set_on_script_complete(Box::new(
            move |name: &str, context: &PipelineContext, result: &PipelineResult| {
                let (broadcast_file, queued_file) = if let Some(broadcast_file) =
                    result.transactions.iter().find_map(|tx| tx.transaction.broadcast_file.clone())
                {
                    let queued_file = queued_file_for_broadcast_path(&broadcast_file);
                    (Some(broadcast_file), queued_file)
                } else if result.transactions.is_empty() {
                    (None, None)
                } else {
                    let (_, broadcast_path, _) =
                        pipeline::broadcast_writer::compute_broadcast_paths(
                            &context.project_root,
                            &context.config.script_path,
                            context.config.chain_id,
                            &context.config.script_sig,
                        );
                    let broadcast_file = pipeline::broadcast_writer::relative_broadcast_path(
                        &context.project_root,
                        &broadcast_path,
                    );
                    let queued_file = queued_file_for_broadcast_path(&broadcast_file);
                    (Some(broadcast_file), queued_file)
                };
                let mut plan = compose_plan_state_for_updates.lock().map_err(|_| {
                    treb_core::error::TrebError::Forge("compose plan lock poisoned".into())
                })?;
                compose_plan::update_component(
                    &mut plan,
                    name,
                    PlanComponentStatus::Broadcast,
                    broadcast_file,
                    queued_file,
                );
                compose_plan::save_plan(&cwd_for_plan, &plan).map(|_| ())
            },
        ));
        if !json {
            broadcast_display.start_spinner("Broadcasting");
            simulated.set_on_action_complete(broadcast_display.build_callback());
        }

        let broadcast_result = {
            let _foundry_shell = super::run::FoundryShellGuard::suppress();
            let r = simulated.broadcast_all(&mut registry).await;
            broadcast_display.stop();
            r
        };

        match broadcast_result {
            Ok(results) => {
                let plan = compose_plan_state
                    .lock()
                    .map_err(|_| anyhow::anyhow!("compose plan lock poisoned"))?;
                compose_plan::save_plan(&cwd, &plan)?;
                compose_plan::save_plan_archive(&cwd, &plan)?;
                results
            }
            Err((partial_results, failed_name, err)) => {
                if let Ok(mut plan) = compose_plan_state.lock() {
                    compose_plan::update_component(
                        &mut plan,
                        &failed_name,
                        PlanComponentStatus::Failed,
                        None,
                        None,
                    );
                    let _ = compose_plan::save_plan(&cwd, &plan);
                }
                for sr in &partial_results {
                    if components_to_run.contains(&sr.name) && sr.name != failed_name {
                        completed += 1;
                        component_results.push(ComponentResultEntry {
                            component: sr.name.clone(),
                            status: ComponentStatus::Success,
                            deployments: sr.result.deployments.len(),
                            transactions: sr.result.transactions.len(),
                            gas_used: sr.result.gas_used,
                            error: None,
                        });
                    }
                }
                let error_msg = format!("{err}");
                if !json {
                    eprintln!(
                        "  {} {}",
                        styled(emoji::CROSS, color::RED),
                        styled(&format!("{failed_name}: {error_msg}"), color::RED),
                    );
                }
                component_results.push(ComponentResultEntry {
                    component: failed_name.clone(),
                    status: ComponentStatus::Failed,
                    deployments: 0,
                    transactions: 0,
                    gas_used: 0,
                    error: Some(error_msg),
                });
                failed_component = Some(failed_name.clone());
                let failed_idx =
                    components_to_run.iter().position(|n| n == &failed_name).unwrap_or(0);
                for name in components_to_run.iter().skip(failed_idx + 1) {
                    component_results.push(ComponentResultEntry {
                        component: name.clone(),
                        status: ComponentStatus::NotExecuted,
                        deployments: 0,
                        transactions: 0,
                        gas_used: 0,
                        error: None,
                    });
                }
                Vec::new()
            }
        }
    } else {
        simulated.into_results()
    };

    // Update fork run snapshot counts after broadcast
    if should_broadcast && banner_is_fork && !script_results.is_empty() {
        let treb_dir = cwd.join(super::run::TREB_DIR);
        let mut store = treb_registry::ForkStateStore::new(&treb_dir);
        if store.load().is_ok() && store.is_fork_mode_active() {
            let total_deps: usize =
                script_results.iter().map(|sr| sr.result.deployments.len()).sum();
            let total_txs: usize =
                script_results.iter().map(|sr| sr.result.transactions.len()).sum();
            if let Some(last) = store.data_mut().run_snapshots.last_mut() {
                last.deployment_count = total_deps;
                last.transaction_count = total_txs;
            }
            let _ = store.save();
        }
    }

    // Record successful results and display broadcast output
    if !script_results.is_empty() {
        for sr in &script_results {
            completed += 1;
            component_results.push(ComponentResultEntry {
                component: sr.name.clone(),
                status: ComponentStatus::Success,
                deployments: sr.result.deployments.len(),
                transactions: sr.result.transactions.len(),
                gas_used: sr.result.gas_used,
                error: None,
            });
        }

        // Per-component broadcast results were already shown inline
        // by the on_action_complete callback. No duplicate summary needed.
    }

    // ── Display results ──────────────────────────────────────────────
    let totals = compute_totals(&component_results);
    let success = failed_component.is_none();
    let failure_error = failed_component.as_ref().map(|failed| {
        anyhow::anyhow!(
            "compose failed: component '{}' failed ({}/{} completed)",
            failed,
            completed,
            total
        )
    });

    if !json {
        display_compose_human(
            &compose.group,
            &component_results,
            &totals,
            completed,
            total,
            &failed_component,
        );
    } else if success {
        // In JSON mode, execution failures bubble up to the top-level JSON
        // error wrapper instead of mixing a result payload with stderr errors.
        display_compose_json(&compose.group, component_results, totals, success)?;
    }

    if let Some(err) = failure_error {
        return Err(err);
    }

    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::run as run_cmd;
    use std::{
        ffi::OsString,
        sync::{LazyLock, Mutex, MutexGuard},
    };

    fn env_lock() -> MutexGuard<'static, ()> {
        static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
        ENV_LOCK.lock().unwrap()
    }

    struct EnvVarGuard {
        key: &'static str,
        old: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let old = std::env::var_os(key);
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, old }
        }

        fn unset(key: &'static str) -> Self {
            let old = std::env::var_os(key);
            unsafe {
                std::env::remove_var(key);
            }
            Self { key, old }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.old {
                Some(value) => unsafe {
                    std::env::set_var(self.key, value);
                },
                None => unsafe {
                    std::env::remove_var(self.key);
                },
            }
        }
    }

    #[test]
    fn deserialize_minimal_compose_file() {
        let yaml = r#"
group: my-deployment
components:
  token:
    script: script/DeployToken.s.sol
"#;
        let compose: ComposeFile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(compose.group, "my-deployment");
        assert_eq!(compose.components.len(), 1);
        let token = &compose.components["token"];
        assert_eq!(token.script, "script/DeployToken.s.sol");
        assert!(token.deps.is_none());
        assert!(token.env.is_none());
        assert!(token.sig.is_none());
        assert!(token.args.is_none());
        assert!(token.verify.is_none());
    }

    #[test]
    fn deserialize_full_compose_file() {
        let yaml = r#"
group: full-deploy
components:
  libraries:
    script: script/DeployLibs.s.sol
    sig: "deploy(uint256)"
    args:
      - "42"
    verify: true
  core:
    script: script/DeployCore.s.sol
    deps:
      - libraries
    env:
      DEPLOYER_KEY: "0xabc"
      SALT: "0x01"
  periphery:
    script: script/DeployPeriphery.s.sol
    deps:
      - libraries
      - core
    verify: false
"#;
        let compose: ComposeFile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(compose.group, "full-deploy");
        assert_eq!(compose.components.len(), 3);

        let libs = &compose.components["libraries"];
        assert_eq!(libs.script, "script/DeployLibs.s.sol");
        assert!(libs.deps.is_none());
        assert!(libs.env.is_none());
        assert_eq!(libs.sig.as_deref(), Some("deploy(uint256)"));
        assert_eq!(libs.args.as_ref().unwrap(), &vec!["42".to_string()]);
        assert_eq!(libs.verify, Some(true));

        let core = &compose.components["core"];
        assert_eq!(core.deps.as_ref().unwrap(), &vec!["libraries".to_string()]);
        let env = core.env.as_ref().unwrap();
        assert_eq!(env.get("DEPLOYER_KEY").unwrap(), "0xabc");
        assert_eq!(env.get("SALT").unwrap(), "0x01");
        assert!(core.sig.is_none());
        assert!(core.args.is_none());
        assert!(core.verify.is_none());

        let periphery = &compose.components["periphery"];
        assert_eq!(
            periphery.deps.as_ref().unwrap(),
            &vec!["libraries".to_string(), "core".to_string()]
        );
        assert_eq!(periphery.verify, Some(false));
    }

    #[test]
    fn optional_fields_deserialize_as_none() {
        let yaml = r#"
group: test
components:
  a:
    script: script/A.s.sol
"#;
        let compose: ComposeFile = serde_yaml::from_str(yaml).unwrap();
        let a = &compose.components["a"];
        assert!(a.deps.is_none(), "deps should be None, not Some(vec![])");
        assert!(a.env.is_none(), "env should be None, not Some(map)");
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let yaml = r#"
group: test
extra_field: ignored
components:
  a:
    script: script/A.s.sol
    unknown_option: true
"#;
        // serde_yaml with default settings ignores unknown fields
        let result = serde_yaml::from_str::<ComposeFile>(yaml);
        assert!(result.is_ok(), "unknown fields should be ignored: {:?}", result.err());
    }

    // ── Validation tests ────────────────────────────────────────────────

    #[test]
    fn validate_valid_compose_file() {
        let yaml = r#"
group: my-deploy
components:
  libs:
    script: script/DeployLibs.s.sol
  core:
    script: script/DeployCore.s.sol
    deps:
      - libs
"#;
        let compose: ComposeFile = serde_yaml::from_str(yaml).unwrap();
        assert!(validate_compose_file(&compose).is_ok());
    }

    #[test]
    fn validate_empty_group_fails() {
        let yaml = r#"
group: ""
components:
  a:
    script: script/A.s.sol
"#;
        let compose: ComposeFile = serde_yaml::from_str(yaml).unwrap();
        let err = validate_compose_file(&compose).unwrap_err();
        assert!(
            err.to_string().contains("group name is required"),
            "expected empty group error, got: {}",
            err
        );
    }

    #[test]
    fn validate_empty_components_fails() {
        let yaml = r#"
group: test
components: {}
"#;
        let compose: ComposeFile = serde_yaml::from_str(yaml).unwrap();
        let err = validate_compose_file(&compose).unwrap_err();
        assert!(
            err.to_string().contains("at least one component is required"),
            "expected empty components error, got: {}",
            err
        );
    }

    #[test]
    fn validate_empty_script_fails() {
        let yaml = r#"
group: test
components:
  bad:
    script: ""
"#;
        let compose: ComposeFile = serde_yaml::from_str(yaml).unwrap();
        let err = validate_compose_file(&compose).unwrap_err();
        assert!(
            err.to_string().contains("component 'bad'")
                && err.to_string().contains("must specify a script"),
            "expected empty script error for 'bad', got: {}",
            err
        );
    }

    #[test]
    fn validate_unknown_dep_fails() {
        let yaml = r#"
group: test
components:
  a:
    script: script/A.s.sol
    deps:
      - nonexistent
"#;
        let compose: ComposeFile = serde_yaml::from_str(yaml).unwrap();
        let err = validate_compose_file(&compose).unwrap_err();
        assert!(
            err.to_string()
                .contains("component 'a' depends on non-existent component 'nonexistent'"),
            "expected unknown dep error, got: {}",
            err
        );
    }

    #[test]
    fn validate_self_dep_fails() {
        let yaml = r#"
group: test
components:
  a:
    script: script/A.s.sol
    deps:
      - a
"#;
        let compose: ComposeFile = serde_yaml::from_str(yaml).unwrap();
        let err = validate_compose_file(&compose).unwrap_err();
        assert!(
            err.to_string().contains("component 'a' cannot depend on itself"),
            "expected self-dep error, got: {}",
            err
        );
    }

    #[test]
    fn validate_invalid_component_name_fails() {
        let yaml = r#"
group: test
components:
  "bad name":
    script: script/A.s.sol
"#;
        let compose: ComposeFile = serde_yaml::from_str(yaml).unwrap();
        let err = validate_compose_file(&compose).unwrap_err();
        assert!(
            err.to_string().contains("component 'bad name'")
                && err.to_string().contains("invalid name"),
            "expected invalid name error, got: {}",
            err
        );
    }

    #[test]
    fn validate_component_name_with_hyphens_and_underscores() {
        let yaml = r#"
group: test
components:
  my-component_v2:
    script: script/A.s.sol
"#;
        let compose: ComposeFile = serde_yaml::from_str(yaml).unwrap();
        assert!(validate_compose_file(&compose).is_ok());
    }

    // ── Loading tests ───────────────────────────────────────────────────

    #[test]
    fn load_missing_file_fails() {
        let err = load_compose_file("/nonexistent/path/compose.yaml").unwrap_err();
        assert!(
            err.to_string().contains("compose file not found"),
            "expected file not found error, got: {}",
            err
        );
    }

    #[test]
    fn load_malformed_yaml_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.yaml");
        std::fs::write(&path, "not: [valid: yaml: {{").unwrap();
        let err = load_compose_file(path.to_str().unwrap()).unwrap_err();
        assert!(
            format!("{:#}", err).contains("failed to parse compose file"),
            "expected parse error, got: {:#}",
            err
        );
    }

    #[test]
    fn load_valid_file_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deploy.yaml");
        std::fs::write(&path, "group: test\ncomponents:\n  a:\n    script: script/A.s.sol\n")
            .unwrap();
        let compose = load_compose_file(path.to_str().unwrap()).unwrap();
        assert_eq!(compose.group, "test");
        assert_eq!(compose.components.len(), 1);
    }

    // ── Topological sort tests ─────────────────────────────────────────

    fn make_component(script: &str, deps: Option<Vec<&str>>) -> Component {
        Component {
            script: script.to_string(),
            deps: deps.map(|d| d.into_iter().map(String::from).collect()),
            env: None,
            sig: None,
            args: None,
            verify: None,
        }
    }

    fn make_compose(components: Vec<(&str, Component)>) -> ComposeFile {
        ComposeFile {
            group: "test".to_string(),
            components: components
                .into_iter()
                .map(|(name, comp)| (name.to_string(), comp))
                .collect(),
        }
    }

    #[test]
    fn topo_single_component() {
        let compose = make_compose(vec![("token", make_component("script/Token.s.sol", None))]);
        let order = build_execution_order(&compose).unwrap();
        assert_eq!(order, vec!["token"]);
    }

    #[test]
    fn topo_independent_components_alphabetical() {
        let compose = make_compose(vec![
            ("charlie", make_component("script/C.s.sol", None)),
            ("alpha", make_component("script/A.s.sol", None)),
            ("bravo", make_component("script/B.s.sol", None)),
        ]);
        let order = build_execution_order(&compose).unwrap();
        assert_eq!(order, vec!["alpha", "bravo", "charlie"]);
    }

    #[test]
    fn topo_linear_chain() {
        let compose = make_compose(vec![
            ("a", make_component("script/A.s.sol", None)),
            ("b", make_component("script/B.s.sol", Some(vec!["a"]))),
            ("c", make_component("script/C.s.sol", Some(vec!["b"]))),
        ]);
        let order = build_execution_order(&compose).unwrap();
        assert_eq!(order, vec!["a", "b", "c"]);
    }

    #[test]
    fn topo_diamond_dependency() {
        // a -> b, a -> c, b -> d, c -> d
        let compose = make_compose(vec![
            ("a", make_component("script/A.s.sol", None)),
            ("b", make_component("script/B.s.sol", Some(vec!["a"]))),
            ("c", make_component("script/C.s.sol", Some(vec!["a"]))),
            ("d", make_component("script/D.s.sol", Some(vec!["b", "c"]))),
        ]);
        let order = build_execution_order(&compose).unwrap();

        // 'a' must be first, 'd' must be last, b and c in between (alphabetically: b, c)
        assert_eq!(order[0], "a");
        assert_eq!(order[3], "d");

        let b_pos = order.iter().position(|x| x == "b").unwrap();
        let c_pos = order.iter().position(|x| x == "c").unwrap();
        assert!(b_pos < order.iter().position(|x| x == "d").unwrap());
        assert!(c_pos < order.iter().position(|x| x == "d").unwrap());
    }

    #[test]
    fn topo_deps_before_dependents() {
        let compose = make_compose(vec![
            ("libs", make_component("script/Libs.s.sol", None)),
            ("core", make_component("script/Core.s.sol", Some(vec!["libs"]))),
            ("periphery", make_component("script/Periphery.s.sol", Some(vec!["libs", "core"]))),
        ]);
        let order = build_execution_order(&compose).unwrap();

        let libs_pos = order.iter().position(|x| x == "libs").unwrap();
        let core_pos = order.iter().position(|x| x == "core").unwrap();
        let periphery_pos = order.iter().position(|x| x == "periphery").unwrap();

        assert!(libs_pos < core_pos);
        assert!(libs_pos < periphery_pos);
        assert!(core_pos < periphery_pos);
    }

    #[test]
    fn topo_direct_cycle_detected() {
        let compose = make_compose(vec![
            ("a", make_component("script/A.s.sol", Some(vec!["b"]))),
            ("b", make_component("script/B.s.sol", Some(vec!["a"]))),
        ]);
        let err = build_execution_order(&compose).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("circular dependency detected"), "expected cycle error, got: {}", msg);
        // Should name at least one component
        assert!(
            msg.contains("a") || msg.contains("b"),
            "cycle error should name involved components, got: {}",
            msg
        );
    }

    #[test]
    fn topo_indirect_cycle_detected() {
        let compose = make_compose(vec![
            ("a", make_component("script/A.s.sol", Some(vec!["c"]))),
            ("b", make_component("script/B.s.sol", Some(vec!["a"]))),
            ("c", make_component("script/C.s.sol", Some(vec!["b"]))),
        ]);
        let err = build_execution_order(&compose).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("circular dependency detected"), "expected cycle error, got: {}", msg);
    }

    #[test]
    fn topo_cycle_with_some_independent_nodes() {
        // 'x' is independent, 'a' <-> 'b' forms a cycle
        let compose = make_compose(vec![
            ("a", make_component("script/A.s.sol", Some(vec!["b"]))),
            ("b", make_component("script/B.s.sol", Some(vec!["a"]))),
            ("x", make_component("script/X.s.sol", None)),
        ]);
        let err = build_execution_order(&compose).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("circular dependency detected"));
        // Should mention the cycling components, not 'x'
        assert!(msg.contains("a") && msg.contains("b"));
    }

    // ── Plan builder tests ─────────────────────────────────────────────

    #[test]
    fn build_plan_creates_correct_entries() {
        let compose = make_compose(vec![
            ("libs", make_component("script/Libs.s.sol", None)),
            ("core", make_component("script/Core.s.sol", Some(vec!["libs"]))),
        ]);
        let order = build_execution_order(&compose).unwrap();
        let plan = build_plan(&compose, &order, &HashSet::new());

        assert_eq!(plan.len(), 2);

        assert_eq!(plan[0].step, 1);
        assert_eq!(plan[0].component, "libs");
        assert_eq!(plan[0].script, "script/Libs.s.sol");
        assert!(plan[0].deps.is_empty());
        assert!(!plan[0].skipped);

        assert_eq!(plan[1].step, 2);
        assert_eq!(plan[1].component, "core");
        assert_eq!(plan[1].script, "script/Core.s.sol");
        assert_eq!(plan[1].deps, vec!["libs"]);
        assert!(!plan[1].skipped);
    }

    #[test]
    fn plan_json_serialization() {
        let compose = make_compose(vec![
            ("a", make_component("script/A.s.sol", None)),
            ("b", make_component("script/B.s.sol", Some(vec!["a"]))),
        ]);
        let order = build_execution_order(&compose).unwrap();
        let plan = build_plan(&compose, &order, &HashSet::new());

        let json = serde_json::to_string_pretty(&plan).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(parsed.is_array());
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["step"], 1);
        assert_eq!(arr[0]["component"], "a");
        assert_eq!(arr[1]["step"], 2);
        assert_eq!(arr[1]["deps"][0], "a");
    }

    #[test]
    fn execution_plan_headers_use_stage_style_when_color_enabled() {
        let _lock = env_lock();
        let _no_color = EnvVarGuard::unset("NO_COLOR");
        let _term = EnvVarGuard::set("TERM", "xterm-256color");
        owo_colors::set_override(true);
        color::color_enabled(false);

        let compose =
            make_compose(vec![("registry", make_component("script/Registry.s.sol", None))]);
        let headers = format_execution_plan_header_lines(&compose, 1);
        let expected =
            ["🎯 Orchestrating test", "📋 Execution plan: 1 components", "📋 Execution Plan:"];

        for (header, plain_text) in headers.iter().zip(expected) {
            assert!(
                header.starts_with("\x1b["),
                "header should be fully stage-styled, got: {header:?}"
            );
            assert_eq!(crate::ui::terminal::strip_ansi_codes(header), plain_text);
        }

        owo_colors::set_override(false);
        color::color_enabled(true);
    }

    // ── Env var and verify override tests ─────────────────────────────

    #[test]
    fn component_env_overrides_global_env() {
        // Inject global env var
        let global = vec!["TREB_COMPOSE_TEST_KEY=global_value".to_string()];
        run_cmd::inject_env_vars(&global).unwrap();
        assert_eq!(env::var("TREB_COMPOSE_TEST_KEY").unwrap(), "global_value");

        // Component env overrides it
        let mut comp_env = HashMap::new();
        comp_env.insert("TREB_COMPOSE_TEST_KEY".to_string(), "component_value".to_string());
        for (key, value) in &comp_env {
            unsafe { env::set_var(key, value) };
        }
        assert_eq!(env::var("TREB_COMPOSE_TEST_KEY").unwrap(), "component_value");

        // Re-injecting global restores original value
        run_cmd::inject_env_vars(&global).unwrap();
        assert_eq!(env::var("TREB_COMPOSE_TEST_KEY").unwrap(), "global_value");

        // Cleanup
        unsafe { env::remove_var("TREB_COMPOSE_TEST_KEY") };
    }

    #[test]
    fn component_verify_overrides_global() {
        let global_verify = true;

        // Component with explicit verify=false overrides global
        let component = Component {
            script: "script/A.s.sol".to_string(),
            deps: None,
            env: None,
            sig: None,
            args: None,
            verify: Some(false),
        };
        assert!(!component.verify.unwrap_or(global_verify));
    }

    #[test]
    fn global_verify_used_when_component_none() {
        let global_verify = true;

        // Component without verify uses global
        let component = Component {
            script: "script/A.s.sol".to_string(),
            deps: None,
            env: None,
            sig: None,
            args: None,
            verify: None,
        };
        assert!(component.verify.unwrap_or(global_verify));
    }

    #[test]
    fn parse_env_var_reusable_from_compose() {
        // Verify that parse_env_var is accessible from compose module
        let (key, value) = run_cmd::parse_env_var("MY_KEY=my_value").unwrap();
        assert_eq!(key, "MY_KEY");
        assert_eq!(value, "my_value");
    }

    // ── Resume plan tests ────────────────────────────────────────────

    #[test]
    fn compose_hash_deterministic() {
        let content = "group: test\ncomponents:\n  a:\n    script: A.s.sol\n";
        let hash1 = compose_plan::compute_compose_hash(content);
        let hash2 = compose_plan::compute_compose_hash(content);
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 16); // 16 hex chars
    }

    #[test]
    fn compose_hash_changes_on_different_content() {
        let hash1 = compose_plan::compute_compose_hash("content A");
        let hash2 = compose_plan::compute_compose_hash("content B");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn queued_file_for_broadcast_path_appends_queued_suffix() {
        let queued =
            queued_file_for_broadcast_path("broadcast/Deploy.s.sol/1/run-latest.json").unwrap();
        assert_eq!(queued, "broadcast/Deploy.s.sol/1/run-latest.queued.json");

        let archived =
            queued_file_for_broadcast_path("broadcast/Deploy.s.sol/1/run-1234567890.json").unwrap();
        assert_eq!(archived, "broadcast/Deploy.s.sol/1/run-1234567890.queued.json");
    }

    #[test]
    fn load_resume_plan_uses_single_plan_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let plan = compose_plan::create_plan(
            "compose/simple.yaml",
            "hash123",
            1,
            &[("registry".into(), "script/DeployRegistry.s.sol".into())],
        );
        compose_plan::save_plan(dir.path(), &plan).unwrap();

        let loaded = load_resume_plan(dir.path(), "compose/simple.yaml", None).unwrap();
        assert_eq!(loaded.chain_id, 1);
        assert_eq!(loaded.compose_hash, "hash123");
    }

    #[test]
    fn load_resume_plan_prefers_matching_chain() {
        let dir = tempfile::tempdir().unwrap();
        let first = compose_plan::create_plan(
            "compose/simple.yaml",
            "hash-one",
            1,
            &[("registry".into(), "script/DeployRegistry.s.sol".into())],
        );
        let second = compose_plan::create_plan(
            "compose/simple.yaml",
            "hash-two",
            2,
            &[("registry".into(), "script/DeployRegistry.s.sol".into())],
        );
        compose_plan::save_plan(dir.path(), &first).unwrap();
        compose_plan::save_plan(dir.path(), &second).unwrap();

        let loaded = load_resume_plan(dir.path(), "compose/simple.yaml", Some(2)).unwrap();
        assert_eq!(loaded.chain_id, 2);
        assert_eq!(loaded.compose_hash, "hash-two");
    }

    #[test]
    fn plan_with_skip_set_marks_skipped() {
        let compose = make_compose(vec![
            ("a", make_component("script/A.s.sol", None)),
            ("b", make_component("script/B.s.sol", Some(vec!["a"]))),
            ("c", make_component("script/C.s.sol", Some(vec!["a"]))),
        ]);
        let order = build_execution_order(&compose).unwrap();

        let mut skip_set = HashSet::new();
        skip_set.insert("a".to_string());

        let plan = build_plan(&compose, &order, &skip_set);

        assert_eq!(plan.len(), 3);
        assert!(plan[0].skipped, "'a' should be skipped");
        assert!(!plan[1].skipped, "'b' should not be skipped");
        assert!(!plan[2].skipped, "'c' should not be skipped");
    }

    #[test]
    fn plan_json_skipped_field_only_when_true() {
        let compose = make_compose(vec![
            ("a", make_component("script/A.s.sol", None)),
            ("b", make_component("script/B.s.sol", Some(vec!["a"]))),
        ]);
        let order = build_execution_order(&compose).unwrap();

        let mut skip_set = HashSet::new();
        skip_set.insert("a".to_string());

        let plan = build_plan(&compose, &order, &skip_set);
        let json = serde_json::to_string_pretty(&plan).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        let arr = parsed.as_array().unwrap();
        // 'a' should have skipped: true
        assert_eq!(arr[0]["skipped"], true);
        // 'b' should not have skipped field (skip_serializing_if)
        assert!(arr[1].get("skipped").is_none());
    }

    // ── Result aggregation tests ─────────────────────────────────────

    #[test]
    fn compute_totals_all_success() {
        let results = vec![
            ComponentResultEntry {
                component: "a".to_string(),
                status: ComponentStatus::Success,
                deployments: 2,
                transactions: 3,
                gas_used: 100_000,
                error: None,
            },
            ComponentResultEntry {
                component: "b".to_string(),
                status: ComponentStatus::Success,
                deployments: 1,
                transactions: 1,
                gas_used: 50_000,
                error: None,
            },
        ];
        let totals = compute_totals(&results);
        assert_eq!(totals.deployments, 3);
        assert_eq!(totals.transactions, 4);
        assert_eq!(totals.gas_used, 150_000);
        assert_eq!(totals.succeeded, 2);
        assert_eq!(totals.skipped, 0);
        assert_eq!(totals.failed, 0);
        assert_eq!(totals.not_executed, 0);
    }

    #[test]
    fn compute_totals_mixed_statuses() {
        let results = vec![
            ComponentResultEntry {
                component: "a".to_string(),
                status: ComponentStatus::Skipped,
                deployments: 0,
                transactions: 0,
                gas_used: 0,
                error: None,
            },
            ComponentResultEntry {
                component: "b".to_string(),
                status: ComponentStatus::Success,
                deployments: 3,
                transactions: 2,
                gas_used: 200_000,
                error: None,
            },
            ComponentResultEntry {
                component: "c".to_string(),
                status: ComponentStatus::Failed,
                deployments: 0,
                transactions: 0,
                gas_used: 0,
                error: Some("script reverted".to_string()),
            },
            ComponentResultEntry {
                component: "d".to_string(),
                status: ComponentStatus::NotExecuted,
                deployments: 0,
                transactions: 0,
                gas_used: 0,
                error: None,
            },
        ];
        let totals = compute_totals(&results);
        assert_eq!(totals.deployments, 3);
        assert_eq!(totals.transactions, 2);
        assert_eq!(totals.gas_used, 200_000);
        assert_eq!(totals.succeeded, 1);
        assert_eq!(totals.skipped, 1);
        assert_eq!(totals.failed, 1);
        assert_eq!(totals.not_executed, 1);
    }

    #[test]
    fn compute_totals_empty_results() {
        let results: Vec<ComponentResultEntry> = vec![];
        let totals = compute_totals(&results);
        assert_eq!(totals.deployments, 0);
        assert_eq!(totals.transactions, 0);
        assert_eq!(totals.gas_used, 0);
        assert_eq!(totals.succeeded, 0);
        assert_eq!(totals.skipped, 0);
        assert_eq!(totals.failed, 0);
        assert_eq!(totals.not_executed, 0);
    }

    #[test]
    fn compose_json_output_all_success() {
        let results = vec![
            ComponentResultEntry {
                component: "libs".to_string(),
                status: ComponentStatus::Success,
                deployments: 2,
                transactions: 1,
                gas_used: 100_000,
                error: None,
            },
            ComponentResultEntry {
                component: "core".to_string(),
                status: ComponentStatus::Success,
                deployments: 1,
                transactions: 2,
                gas_used: 75_000,
                error: None,
            },
        ];
        let totals = compute_totals(&results);
        let output = ComposeOutputJson {
            group: "my-deploy".to_string(),
            success: true,
            components: results,
            totals,
        };
        let json_str = serde_json::to_string_pretty(&output).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed["group"], "my-deploy");
        assert_eq!(parsed["success"], true);

        let components = parsed["components"].as_array().unwrap();
        assert_eq!(components.len(), 2);
        assert_eq!(components[0]["component"], "libs");
        assert_eq!(components[0]["status"], "success");
        assert_eq!(components[0]["deployments"], 2);
        assert_eq!(components[0]["transactions"], 1);
        assert_eq!(components[0]["gasUsed"], 100_000);
        assert!(components[0].get("error").is_none());

        assert_eq!(components[1]["component"], "core");
        assert_eq!(components[1]["status"], "success");
        assert_eq!(components[1]["deployments"], 1);
        assert_eq!(components[1]["transactions"], 2);
        assert_eq!(components[1]["gasUsed"], 75_000);

        assert_eq!(parsed["totals"]["deployments"], 3);
        assert_eq!(parsed["totals"]["transactions"], 3);
        assert_eq!(parsed["totals"]["gasUsed"], 175_000);
        assert_eq!(parsed["totals"]["succeeded"], 2);
        assert_eq!(parsed["totals"]["skipped"], 0);
        assert_eq!(parsed["totals"]["failed"], 0);
        assert_eq!(parsed["totals"]["notExecuted"], 0);
    }

    #[test]
    fn compose_json_output_with_failure() {
        let results = vec![
            ComponentResultEntry {
                component: "a".to_string(),
                status: ComponentStatus::Success,
                deployments: 1,
                transactions: 1,
                gas_used: 50_000,
                error: None,
            },
            ComponentResultEntry {
                component: "b".to_string(),
                status: ComponentStatus::Failed,
                deployments: 0,
                transactions: 0,
                gas_used: 0,
                error: Some("script reverted".to_string()),
            },
            ComponentResultEntry {
                component: "c".to_string(),
                status: ComponentStatus::NotExecuted,
                deployments: 0,
                transactions: 0,
                gas_used: 0,
                error: None,
            },
        ];
        let totals = compute_totals(&results);
        let output = ComposeOutputJson {
            group: "test-deploy".to_string(),
            success: false,
            components: results,
            totals,
        };
        let json_str = serde_json::to_string_pretty(&output).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed["success"], false);

        let components = parsed["components"].as_array().unwrap();
        assert_eq!(components[1]["status"], "failed");
        assert_eq!(components[1]["error"], "script reverted");
        assert_eq!(components[1]["gasUsed"], 0);
        assert_eq!(components[2]["status"], "not_executed");
        assert_eq!(components[2]["gasUsed"], 0);
        assert!(components[2].get("error").is_none());

        assert_eq!(parsed["totals"]["succeeded"], 1);
        assert_eq!(parsed["totals"]["failed"], 1);
        assert_eq!(parsed["totals"]["notExecuted"], 1);
        assert_eq!(parsed["totals"]["gasUsed"], 50_000);
    }

    #[test]
    fn compose_json_output_with_skipped() {
        let results = vec![
            ComponentResultEntry {
                component: "a".to_string(),
                status: ComponentStatus::Skipped,
                deployments: 0,
                transactions: 0,
                gas_used: 0,
                error: None,
            },
            ComponentResultEntry {
                component: "b".to_string(),
                status: ComponentStatus::Success,
                deployments: 5,
                transactions: 3,
                gas_used: 300_000,
                error: None,
            },
        ];
        let totals = compute_totals(&results);
        let output = ComposeOutputJson {
            group: "resume-deploy".to_string(),
            success: true,
            components: results,
            totals,
        };
        let json_str = serde_json::to_string_pretty(&output).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        let components = parsed["components"].as_array().unwrap();
        assert_eq!(components[0]["status"], "skipped");
        assert_eq!(components[0]["deployments"], 0);
        assert_eq!(components[0]["gasUsed"], 0);

        assert_eq!(parsed["totals"]["skipped"], 1);
        assert_eq!(parsed["totals"]["succeeded"], 1);
        assert_eq!(parsed["totals"]["deployments"], 5);
        assert_eq!(parsed["totals"]["transactions"], 3);
        assert_eq!(parsed["totals"]["gasUsed"], 300_000);
    }

    #[test]
    fn component_status_display_strings() {
        assert_eq!(ComponentStatus::Success.to_string(), "success");
        assert_eq!(ComponentStatus::Skipped.to_string(), "skipped");
        assert_eq!(ComponentStatus::Failed.to_string(), "failed");
        assert_eq!(ComponentStatus::NotExecuted.to_string(), "not executed");
    }

    #[test]
    fn prompt_for_broadcast_confirmation_requires_remaining_components() {
        assert!(should_prompt_for_broadcast_confirmation(true, false, true, 1));
        assert!(!should_prompt_for_broadcast_confirmation(true, false, true, 0));
        assert!(!should_prompt_for_broadcast_confirmation(true, true, true, 1));
        assert!(!should_prompt_for_broadcast_confirmation(true, false, false, 1));
        assert!(!should_prompt_for_broadcast_confirmation(false, false, true, 1));
    }

    #[test]
    fn interactive_json_broadcast_is_not_rejected_when_resume_is_a_no_op() {
        assert!(should_reject_interactive_json_broadcast(true, false, true, true, 1));
        assert!(!should_reject_interactive_json_broadcast(true, false, true, true, 0));
        assert!(!should_reject_interactive_json_broadcast(true, false, true, false, 1));
        assert!(!should_reject_interactive_json_broadcast(true, false, false, true, 1));
        assert!(!should_reject_interactive_json_broadcast(true, true, true, true, 1));
    }

    #[test]
    fn compose_json_is_valid_json() {
        // Verify the full output is valid JSON parseable by serde
        let results = vec![ComponentResultEntry {
            component: "libs".to_string(),
            status: ComponentStatus::Success,
            deployments: 2,
            transactions: 1,
            gas_used: 80_000,
            error: None,
        }];
        let totals = compute_totals(&results);
        let output = ComposeOutputJson {
            group: "test".to_string(),
            success: true,
            components: results,
            totals,
        };
        let json_str = serde_json::to_string_pretty(&output).unwrap();
        // Re-parse to verify it's valid JSON
        let reparsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert!(reparsed.is_object());
        assert!(reparsed["components"].is_array());
        assert!(reparsed["totals"].is_object());
    }
}
