//! `treb compose` command implementation.
//!
//! YAML-based multi-step deployment orchestration that executes multiple
//! Forge scripts in dependency order.

use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    env,
    hash::{DefaultHasher, Hash, Hasher},
    io::{self, BufRead, IsTerminal, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use treb_config::{ResolveOpts, resolve_config};
use treb_forge::{
    pipeline::{PipelineConfig, PipelineContext, RunPipeline, resolve_git_commit},
    script::build_script_config_with_senders,
    sender::resolve_all_senders,
};
use treb_registry::Registry;

use crate::output;

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

// ── Resume state tracking ────────────────────────────────────────────────

/// State file for tracking compose execution progress.
const COMPOSE_STATE_FILE: &str = ".treb/compose-state.json";

/// Persistent state for resume support.
#[derive(Debug, Serialize, Deserialize)]
pub struct ComposeState {
    /// Hash of the compose file contents at the start of the run.
    pub compose_hash: String,
    /// Names of components that completed successfully.
    pub completed: Vec<String>,
}

/// Compute a hash of file contents for change detection.
fn compute_file_hash(contents: &str) -> String {
    let mut hasher = DefaultHasher::new();
    contents.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Load the compose state file, returning `None` if it doesn't exist.
pub fn load_compose_state() -> anyhow::Result<Option<ComposeState>> {
    let path = Path::new(COMPOSE_STATE_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read compose state file: {}", path.display()))?;
    let state: ComposeState = serde_json::from_str(&contents)
        .with_context(|| format!("failed to parse compose state file: {}", path.display()))?;
    Ok(Some(state))
}

/// Save the compose state file.
fn save_compose_state(state: &ComposeState) -> anyhow::Result<()> {
    let path = Path::new(COMPOSE_STATE_FILE);
    let contents = serde_json::to_string_pretty(state)?;
    std::fs::write(path, contents)
        .with_context(|| format!("failed to write compose state file: {}", path.display()))?;
    Ok(())
}

/// Delete the compose state file if it exists.
fn delete_compose_state() {
    let path = Path::new(COMPOSE_STATE_FILE);
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
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

/// Display the dry-run plan in human-readable format.
fn print_dry_run_plan(compose: &ComposeFile, plan: &[PlanEntry]) {
    eprintln!("\nOrchestrating {}", compose.group);
    eprintln!("Execution plan: {} components\n", plan.len());
    eprintln!("Execution Plan:");
    eprintln!("{}", "─".repeat(50));
    for entry in plan {
        eprint!("{}. {} → {}", entry.step, entry.component, entry.script);
        if !entry.deps.is_empty() {
            eprint!(" (depends on: [{}])", entry.deps.join(", "));
        }
        if entry.skipped {
            eprint!(" (skipped)");
        }
        eprintln!();
    }
    eprintln!();
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

/// Display compose results in human-readable format.
fn display_compose_human(group: &str, results: &[ComponentResultEntry], totals: &ComposeTotals) {
    println!();
    println!("Compose results: {}", group);
    println!();

    for r in results {
        match r.status {
            ComponentStatus::Success => {
                println!(
                    "  {} — {} deployment{}, {} transaction{}",
                    r.component,
                    r.deployments,
                    if r.deployments == 1 { "" } else { "s" },
                    r.transactions,
                    if r.transactions == 1 { "" } else { "s" },
                );
            }
            ComponentStatus::Skipped => {
                println!("  {} (skipped)", r.component);
            }
            ComponentStatus::Failed => {
                println!(
                    "  {} (failed): {}",
                    r.component,
                    r.error.as_deref().unwrap_or("unknown error")
                );
            }
            ComponentStatus::NotExecuted => {
                println!("  {} (not executed)", r.component);
            }
        }
    }

    println!();
    println!(
        "Totals: {} deployment{}, {} transaction{} | {} succeeded, {} skipped, {} failed, {} not executed",
        totals.deployments,
        if totals.deployments == 1 { "" } else { "s" },
        totals.transactions,
        if totals.transactions == 1 { "" } else { "s" },
        totals.succeeded,
        totals.skipped,
        totals.failed,
        totals.not_executed,
    );
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
    verbose: bool,
    json: bool,
    env_vars: Vec<String>,
    non_interactive: bool,
) -> anyhow::Result<()> {
    // Parse and validate the compose file.
    let compose = load_compose_file(&file)?;
    validate_compose_file(&compose)?;

    // Build execution order (topological sort).
    let order = build_execution_order(&compose)?;

    // ── Resume state handling ─────────────────────────────────────────
    let compose_contents = std::fs::read_to_string(&file)
        .with_context(|| format!("failed to read compose file: {}", file))?;
    let compose_hash = compute_file_hash(&compose_contents);

    let skip_set: HashSet<String> = if resume {
        if let Some(state) = load_compose_state()? {
            // Warn if compose file changed since the state was saved.
            if state.compose_hash != compose_hash {
                eprintln!("Warning: compose file has changed since the last run; resuming anyway");
            }
            state.completed.into_iter().collect()
        } else {
            HashSet::new()
        }
    } else {
        // Fresh start: clear any existing state file.
        delete_compose_state();
        HashSet::new()
    };

    // Dry-run: show execution plan and exit.
    if dry_run {
        let plan = build_plan(&compose, &order, &skip_set);
        if json {
            output::print_json(&plan)?;
        } else {
            eprintln!("{}", output::format_warning_banner("\u{1f6a7}", "[DRY RUN] Showing execution plan only — no changes will be made."));
            eprintln!();
            print_dry_run_plan(&compose, &plan);
        }
        return Ok(());
    }

    // ── Project initialization check ──────────────────────────────────
    let cwd = env::current_dir().context("failed to determine current directory")?;
    super::run::ensure_initialized(&cwd)?;

    // ── Broadcast confirmation (once before first component) ──────────
    if broadcast && !non_interactive {
        let is_tty = io::stdin().is_terminal();
        if is_tty {
            let executing_count = order.iter().filter(|n| !skip_set.contains(*n)).count();
            let count_str = format!("{}", executing_count);
            let mut kv_pairs: Vec<(&str, &str)> = vec![
                ("Components", &count_str),
                ("Compose", &compose.group),
            ];
            let ns_ref;
            if let Some(ref ns) = namespace {
                ns_ref = ns.clone();
                kv_pairs.push(("Namespace", &ns_ref));
            }
            let net_ref;
            if let Some(ref net) = network {
                net_ref = net.clone();
                kv_pairs.push(("Network", &net_ref));
            }
            eprintln!("About to broadcast to the network.");
            output::eprint_kv(&kv_pairs);
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

    // ── Open registry ─────────────────────────────────────────────────
    let mut registry = Registry::open(&cwd).context("failed to open registry")?;

    // ── Initialize state tracking ─────────────────────────────────────
    let mut state = ComposeState {
        compose_hash: compose_hash.clone(),
        completed: skip_set.iter().cloned().collect(),
    };

    // ── Execute components in topological order ───────────────────────
    let total = order.len();
    let mut completed = skip_set.len();
    let mut component_results: Vec<ComponentResultEntry> = Vec::with_capacity(total);
    let mut failed_component: Option<String> = None;

    if !json {
        output::print_stage("\u{1f680}", &format!("Orchestrating {} ({} components)", compose.group, total));
    }

    for (i, name) in order.iter().enumerate() {
        // Skip already-completed components (resume mode).
        if skip_set.contains(name) {
            if !json {
                output::print_stage("\u{23ed}\u{fe0f}", &format!("[{}/{}] Skipping '{}' (already completed)", i + 1, total, name));
            }
            component_results.push(ComponentResultEntry {
                component: name.clone(),
                status: ComponentStatus::Skipped,
                deployments: 0,
                transactions: 0,
                gas_used: 0,
                error: None,
            });
            continue;
        }

        // Mark remaining components as not-executed if a previous component failed.
        if failed_component.is_some() {
            component_results.push(ComponentResultEntry {
                component: name.clone(),
                status: ComponentStatus::NotExecuted,
                deployments: 0,
                transactions: 0,
                gas_used: 0,
                error: None,
            });
            continue;
        }

        let component = &compose.components[name];

        if !json {
            output::print_stage("\u{1f528}", &format!("[{}/{}] Executing '{}'...", i + 1, total, name));
        }

        // Re-inject global env vars (reset any previous component overrides).
        super::run::inject_env_vars(&env_vars)?;

        // Inject per-component env vars (override global for same keys).
        if let Some(env_map) = &component.env {
            for (key, value) in env_map {
                // SAFETY: single-threaded CLI code; no concurrent env access.
                unsafe { env::set_var(key, value) };
            }
        }

        // Config resolution with global flags.
        let resolved = resolve_config(ResolveOpts {
            project_root: cwd.clone(),
            namespace: namespace.clone(),
            network: network.clone(),
            profile: profile.clone(),
            sender_overrides: HashMap::new(),
        })
        .with_context(|| format!("failed to resolve config for component '{}'", name))?;

        let effective_rpc_url = rpc_url.clone().or_else(|| resolved.network.clone());

        // Sender resolution.
        let resolved_senders = resolve_all_senders(&resolved.senders)
            .await
            .with_context(|| format!("failed to resolve senders for component '{}'", name))?;

        // Build ScriptConfig.
        let mut script_config =
            build_script_config_with_senders(&resolved, &component.script, &resolved_senders)
                .with_context(|| {
                    format!("failed to build script config for component '{}'", name)
                })?;

        let sig = component.sig.as_deref().unwrap_or("run()");
        let args = component.args.clone().unwrap_or_default();
        let effective_verify = component.verify.unwrap_or(verify);

        script_config
            .sig(sig)
            .args(args)
            .broadcast(broadcast)
            .dry_run(false)
            .slow(slow || resolved.slow)
            .legacy(legacy)
            .verify(effective_verify)
            .non_interactive(true); // Already prompted above

        if let Some(ref url) = effective_rpc_url {
            script_config.rpc_url(url);
        }

        // Verbose per-component context.
        if verbose && !json {
            eprintln!("  Script: {}", component.script);
            eprintln!("  Namespace: {}", resolved.namespace);
            if let Some(ref url) = effective_rpc_url {
                eprintln!("  RPC: {}", url);
            }
        }

        // Build pipeline context.
        let pipeline_config = PipelineConfig {
            script_path: component.script.clone(),
            dry_run: false,
            namespace: resolved.namespace.clone(),
            script_sig: sig.to_string(),
            script_args: Vec::new(),
            ..Default::default()
        };

        let git_commit = resolve_git_commit();

        let pipeline_context = PipelineContext {
            config: pipeline_config,
            script_path: PathBuf::from(&component.script),
            git_commit,
            project_root: cwd.clone(),
            deployer_sender: None,
        };

        // Execute pipeline.
        let pipeline = RunPipeline::new(pipeline_context).with_script_config(script_config);

        match pipeline.execute(&mut registry).await {
            Ok(result) => {
                completed += 1;
                if !json {
                    output::print_stage("\u{2705}", &format!("[{}/{}] '{}' completed", i + 1, total, name));
                }

                component_results.push(ComponentResultEntry {
                    component: name.clone(),
                    status: ComponentStatus::Success,
                    deployments: result.deployments.len(),
                    transactions: result.transactions.len(),
                    gas_used: result.gas_used,
                    error: None,
                });

                // Update state file after each successful component.
                state.completed.push(name.clone());
                save_compose_state(&state)?;
            }
            Err(e) => {
                let error_msg = format!("{}", e);
                eprintln!(
                    "Component '{}' failed ({}/{} completed): {}",
                    name, completed, total, error_msg
                );

                component_results.push(ComponentResultEntry {
                    component: name.clone(),
                    status: ComponentStatus::Failed,
                    deployments: 0,
                    transactions: 0,
                    gas_used: 0,
                    error: Some(error_msg),
                });

                failed_component = Some(name.clone());
            }
        }
    }

    // ── Display results ──────────────────────────────────────────────
    let totals = compute_totals(&component_results);
    let success = failed_component.is_none();

    if !json {
        if success {
            output::print_stage("\u{2705}", "Orchestration complete.");
        } else {
            output::print_stage("\u{274c}", "Orchestration failed.");
        }
    }

    if json {
        display_compose_json(&compose.group, component_results, totals, success)?;
    } else {
        display_compose_human(&compose.group, &component_results, &totals);
    }

    // Full successful completion: delete the state file.
    if success {
        delete_compose_state();
    }

    if let Some(ref failed) = failed_component {
        bail!("compose failed: component '{}' failed ({}/{} completed)", failed, completed, total);
    }

    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::run as run_cmd;

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
        assert_eq!(component.verify.unwrap_or(global_verify), false);
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
        assert_eq!(component.verify.unwrap_or(global_verify), true);
    }

    #[test]
    fn parse_env_var_reusable_from_compose() {
        // Verify that parse_env_var is accessible from compose module
        let (key, value) = run_cmd::parse_env_var("MY_KEY=my_value").unwrap();
        assert_eq!(key, "MY_KEY");
        assert_eq!(value, "my_value");
    }

    // ── Resume state tests ───────────────────────────────────────────

    #[test]
    fn compute_file_hash_deterministic() {
        let content = "group: test\ncomponents:\n  a:\n    script: A.s.sol\n";
        let hash1 = compute_file_hash(content);
        let hash2 = compute_file_hash(content);
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 16); // 16 hex chars
    }

    #[test]
    fn compute_file_hash_changes_on_different_content() {
        let hash1 = compute_file_hash("content A");
        let hash2 = compute_file_hash("content B");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn compose_state_serialization_roundtrip() {
        let state = ComposeState {
            compose_hash: "abc123".to_string(),
            completed: vec!["libs".to_string(), "core".to_string()],
        };
        let json = serde_json::to_string_pretty(&state).unwrap();
        let parsed: ComposeState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.compose_hash, "abc123");
        assert_eq!(parsed.completed, vec!["libs", "core"]);
    }

    #[test]
    fn compose_state_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join(".treb");
        std::fs::create_dir_all(&state_path).unwrap();
        let state_file = state_path.join("compose-state.json");

        let state = ComposeState {
            compose_hash: "test_hash".to_string(),
            completed: vec!["alpha".to_string(), "bravo".to_string()],
        };

        // Write state
        let contents = serde_json::to_string_pretty(&state).unwrap();
        std::fs::write(&state_file, &contents).unwrap();

        // Read state back
        let loaded: ComposeState =
            serde_json::from_str(&std::fs::read_to_string(&state_file).unwrap()).unwrap();
        assert_eq!(loaded.compose_hash, "test_hash");
        assert_eq!(loaded.completed, vec!["alpha", "bravo"]);
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

    #[test]
    fn delete_compose_state_no_error_when_missing() {
        // Should not panic or error when file doesn't exist
        delete_compose_state();
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
