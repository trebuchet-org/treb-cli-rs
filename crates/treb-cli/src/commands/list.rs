//! `treb list` command implementation.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    env,
    fmt::{self, Write as _},
    path::Path,
};

use alloy_chains::Chain;
use anyhow::{Context, bail};
use owo_colors::OwoColorize;
use serde::Serialize;
use treb_core::types::{Deployment, DeploymentType};
use treb_registry::Registry;

use crate::{
    output,
    ui::{badge, color, emoji},
};

/// Filter criteria for deployments. All specified filters are combined with AND logic.
pub struct DeploymentFilters {
    pub network: Option<String>,
    pub namespace: Option<String>,
    pub deployment_type: Option<String>,
    pub tag: Option<String>,
    pub contract: Option<String>,
    pub label: Option<String>,
    pub fork: bool,
    pub no_fork: bool,
}

/// Filter a slice of deployments by the given criteria.
///
/// All specified filters are combined with AND logic — a deployment must match
/// every active filter to be included in the result.
pub fn filter_deployments<'a>(
    deployments: &[&'a Deployment],
    filters: &DeploymentFilters,
) -> Vec<&'a Deployment> {
    deployments
        .iter()
        .copied()
        .filter(|d| {
            if let Some(ref ns) = filters.namespace {
                if !d.namespace.eq_ignore_ascii_case(ns) {
                    return false;
                }
            }

            if let Some(ref network) = filters.network {
                if !network_matches(d.chain_id, network) {
                    return false;
                }
            }

            if let Some(ref dtype) = filters.deployment_type {
                if !d.deployment_type.to_string().eq_ignore_ascii_case(dtype) {
                    return false;
                }
            }

            if let Some(ref tag) = filters.tag {
                match &d.tags {
                    Some(tags) => {
                        if !tags.iter().any(|t| t == tag) {
                            return false;
                        }
                    }
                    None => return false,
                }
            }

            if let Some(ref contract) = filters.contract {
                if !d.contract_name.eq_ignore_ascii_case(contract) {
                    return false;
                }
            }

            if let Some(ref label) = filters.label {
                if d.label != *label {
                    return false;
                }
            }

            if filters.fork && !d.namespace.starts_with("fork/") {
                return false;
            }

            if filters.no_fork && d.namespace.starts_with("fork/") {
                return false;
            }

            true
        })
        .collect()
}

fn resolve_chain_id(network: &str) -> Option<u64> {
    network.parse::<u64>().ok().or_else(|| network.parse::<Chain>().ok().map(|chain| chain.id()))
}

fn network_matches(chain_id: u64, network: &str) -> bool {
    resolve_chain_id(network).is_some_and(|resolved_chain_id| resolved_chain_id == chain_id)
}

// ---------------------------------------------------------------------------
// Deployment grouping: namespace > chain_id > display category
// ---------------------------------------------------------------------------

/// Display category for deployment grouping in the list command.
///
/// Separates singletons that serve as proxy implementations into a distinct
/// IMPLEMENTATIONS group, matching Go CLI categorization logic.
/// Display order: Proxy (0) → Implementation (1) → Singleton (2) → Library (3)
/// → Unknown (4).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum DisplayCategory {
    Proxy,
    Implementation,
    Singleton,
    Library,
    Unknown,
}

impl fmt::Display for DisplayCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Proxy => write!(f, "PROXIES"),
            Self::Implementation => write!(f, "IMPLEMENTATIONS"),
            Self::Singleton => write!(f, "SINGLETONS"),
            Self::Library => write!(f, "LIBRARIES"),
            Self::Unknown => write!(f, "UNKNOWN"),
        }
    }
}

impl DisplayCategory {
    /// Returns the color style for this display category.
    pub fn style(&self) -> owo_colors::Style {
        match self {
            Self::Proxy => color::TYPE_PROXY,
            Self::Implementation => color::TYPE_SINGLETON,
            Self::Singleton => color::TYPE_SINGLETON,
            Self::Library => color::TYPE_LIBRARY,
            Self::Unknown => color::TYPE_UNKNOWN,
        }
    }
}

/// A group of deployments sharing the same display category within a chain.
pub struct TypeGroup<'a> {
    pub category: DisplayCategory,
    pub deployments: Vec<&'a Deployment>,
}

/// Deployments organized by namespace > chain_id > display category.
///
/// - Namespace keys sort alphabetically (BTreeMap).
/// - Chain IDs sort numerically (BTreeMap<u64, …>).
/// - Type groups are in fixed display order: Proxy, Implementation, Singleton, Library, Unknown.
/// - Deployments within each type group are sorted by contract name.
pub type GroupedDeployments<'a> = BTreeMap<String, BTreeMap<u64, Vec<TypeGroup<'a>>>>;

/// Result of the list command query, holding filtered deployments and context
/// needed for display and JSON output.
#[allow(dead_code)]
pub struct ListResult<'a> {
    /// Filtered deployments matching the query criteria.
    pub deployments: Vec<&'a Deployment>,
    /// Other namespaces with deployment counts (populated when filtered results
    /// are empty and a namespace filter was applied).
    pub other_namespaces: BTreeMap<String, usize>,
    /// Chain ID to network name mapping for display in chain headers.
    pub network_names: BTreeMap<u64, String>,
    /// Set of deployment IDs in fork namespaces.
    pub fork_deployment_ids: HashSet<String>,
}

/// Returns the sort key for the fixed display order of display categories.
fn type_sort_key(cat: &DisplayCategory) -> u8 {
    match cat {
        DisplayCategory::Proxy => 0,
        DisplayCategory::Implementation => 1,
        DisplayCategory::Singleton => 2,
        DisplayCategory::Library => 3,
        DisplayCategory::Unknown => 4,
    }
}

type ImplementationKey = (String, u64, String);

fn collect_implementation_keys(deployments: &[&Deployment]) -> HashSet<ImplementationKey> {
    deployments
        .iter()
        .filter(|d| d.deployment_type == DeploymentType::Proxy)
        .filter_map(|d| {
            d.proxy_info
                .as_ref()
                .map(|pi| (d.namespace.clone(), d.chain_id, pi.implementation.to_lowercase()))
        })
        .collect()
}

/// Build a map of other namespaces with their deployment counts while
/// preserving the active query context except for namespace. Used for the
/// namespace discovery hint.
fn build_other_namespaces(
    all_deployments: &[&Deployment],
    filters: &DeploymentFilters,
    exclude_namespace: &str,
) -> BTreeMap<String, usize> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let context_filters = DeploymentFilters {
        network: filters.network.clone(),
        namespace: None,
        deployment_type: filters.deployment_type.clone(),
        tag: filters.tag.clone(),
        contract: filters.contract.clone(),
        label: filters.label.clone(),
        fork: filters.fork,
        no_fork: filters.no_fork,
    };

    for d in filter_deployments(all_deployments, &context_filters) {
        if !d.namespace.eq_ignore_ascii_case(exclude_namespace) {
            *counts.entry(d.namespace.clone()).or_insert(0) += 1;
        }
    }
    counts
}

/// Collect deployment IDs from fork namespaces (namespace starts with "fork/").
fn collect_fork_deployment_ids(deployments: &[&Deployment]) -> HashSet<String> {
    deployments.iter().filter(|d| d.namespace.starts_with("fork/")).map(|d| d.id.clone()).collect()
}

/// Load deployment IDs from a snapshot file (for fork-only detection).
fn load_fork_snapshot_ids(path: &Path) -> Option<HashSet<String>> {
    let content = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    let map = value.as_object()?;
    Some(map.keys().cloned().collect())
}

fn resolve_network_names(
    project_root: &Path,
    deployments: &[&Deployment],
    selected_network: Option<&str>,
) -> BTreeMap<u64, String> {
    let mut network_names = BTreeMap::new();

    if let Ok(config) = treb_config::load_foundry_config(project_root) {
        for name in treb_config::rpc_endpoints(&config).into_keys() {
            if let Some(chain_id) = resolve_chain_id(&name) {
                network_names.entry(chain_id).or_insert(name);
            }
        }
    }

    if let Some(network) = selected_network {
        if let Some(chain_id) = resolve_chain_id(network) {
            if network.parse::<u64>().is_err() {
                network_names.insert(chain_id, network.to_string());
            } else if let Some(named_chain) = Chain::from_id(chain_id).named() {
                network_names.entry(chain_id).or_insert_with(|| named_chain.to_string());
            }
        }
    }

    for &deployment in deployments {
        if let Some(named_chain) = Chain::from_id(deployment.chain_id).named() {
            network_names.entry(deployment.chain_id).or_insert_with(|| named_chain.to_string());
        }
    }

    network_names
}

fn format_chain_header_label(chain_id: u64, network_names: &BTreeMap<u64, String>) -> String {
    network_names
        .get(&chain_id)
        .map(|network_name| format!("{network_name} ({chain_id})"))
        .unwrap_or_else(|| chain_id.to_string())
}

fn format_selected_network_label(
    selected_network: Option<&str>,
    network_names: &BTreeMap<u64, String>,
) -> Option<String> {
    let network = selected_network?;
    Some(
        resolve_chain_id(network)
            .map(|chain_id| format_chain_header_label(chain_id, network_names))
            .unwrap_or_else(|| network.to_string()),
    )
}

/// Format a namespace header line in Go CLI format.
///
/// Format: `   ◎ namespace:   UPPERCASE_NAMESPACE              `
/// Uses `%-12s` for `"namespace:"` and `%-30s` for the uppercase value.
/// Styled with `NS_HEADER` (black on yellow) and `NS_HEADER_BOLD` (black bold
/// on yellow) when color is enabled.
fn format_namespace_header(namespace: &str) -> String {
    let label = format!("   {} {:<12} ", emoji::CIRCLE, "namespace:");
    let value = format!("{:<30}", namespace.to_uppercase());
    if color::is_color_enabled() {
        format!("{}{}", label.style(color::NS_HEADER), value.style(color::NS_HEADER_BOLD))
    } else {
        format!("{label}{value}")
    }
}

/// Format a chain header line in Go CLI format.
///
/// Format: `├─ ⛓ chain:       network (chainid)               `
/// Uses `├─` for non-last chains and `└─` for the last chain.
/// Styled with `CHAIN_HEADER` and `CHAIN_HEADER_BOLD` when color is enabled.
fn format_chain_header(chain_label: &str, is_last: bool) -> String {
    let tree_prefix = if is_last { "└─" } else { "├─" };
    let label = format!(" {} {:<12} ", emoji::CHAIN_EMOJI, "chain:");
    let value = format!("{:<30}", chain_label);
    if color::is_color_enabled() {
        format!(
            "{}{}{}",
            tree_prefix,
            label.style(color::CHAIN_HEADER),
            value.style(color::CHAIN_HEADER_BOLD),
        )
    } else {
        format!("{tree_prefix}{label}{value}")
    }
}

/// Format the namespace discovery hint shown when the current namespace has no
/// deployments but other namespaces do.
///
/// Matches Go CLI's `renderNamespaceDiscoveryHint()` format:
/// ```text
/// No deployments found in namespace "<name>" [on <network>]
///
/// Other namespaces with deployments:
///
///   <namespace>          <count> deployment(s)
///
/// Use --namespace <name> or `treb config set namespace <name>` to switch.
/// ```
fn format_namespace_discovery_hint(
    namespace: &str,
    network: Option<&str>,
    other_namespaces: &BTreeMap<String, usize>,
) -> String {
    let mut out = String::new();

    // First line
    write!(out, "No deployments found in namespace \"{namespace}\"").unwrap();
    if let Some(net) = network {
        write!(out, " on {net}").unwrap();
    }
    writeln!(out).unwrap();

    // Other namespaces section
    if !other_namespaces.is_empty() {
        writeln!(out).unwrap();
        writeln!(out, "Other namespaces with deployments:").unwrap();
        writeln!(out).unwrap();
        for (ns, count) in other_namespaces {
            let plural = if *count == 1 { "deployment" } else { "deployments" };
            writeln!(out, "  {ns:<20}{count} {plural}").unwrap();
        }
        writeln!(out).unwrap();
        write!(out, "Use --namespace <name> or `treb config set namespace <name>` to switch.")
            .unwrap();
    }

    out
}

// ---------------------------------------------------------------------------
// Table-based deployment rendering (Go format)
// ---------------------------------------------------------------------------

/// Lookup table for resolving implementation addresses to contract display names.
type ImplNameLookup = HashMap<(String, u64, String), String>;

/// Build a lookup table mapping (namespace_lower, chain_id, address_lower) to
/// contract display names for implementation resolution.
fn build_impl_name_lookup(deployments: &[&Deployment]) -> ImplNameLookup {
    deployments
        .iter()
        .map(|d| {
            let display_name = if d.label.is_empty() {
                d.contract_name.clone()
            } else {
                format!("{}:{}", d.contract_name, d.label)
            };
            ((d.namespace.to_lowercase(), d.chain_id, d.address.to_lowercase()), display_name)
        })
        .collect()
}

/// Build the contract display string for column 0 of a deployment table row.
///
/// Format: `TypeColored(ContractName[:Label]) [fork] (first_tag)`
fn build_contract_display(
    d: &Deployment,
    category: &DisplayCategory,
    fork_deployment_ids: &HashSet<String>,
) -> String {
    let name = if d.label.is_empty() {
        d.contract_name.clone()
    } else {
        format!("{}:{}", d.contract_name, d.label)
    };

    let mut display =
        if color::is_color_enabled() { format!("{}", name.style(category.style())) } else { name };

    if fork_deployment_ids.contains(&d.id) {
        if color::is_color_enabled() {
            write!(display, " {}", "[fork]".style(color::FORK_INDICATOR)).unwrap();
        } else {
            display.push_str(" [fork]");
        }
    }

    if let Some(tags) = &d.tags {
        if let Some(first_tag) = tags.first() {
            let tag_str = format!("({first_tag})");
            if color::is_color_enabled() {
                write!(display, " {}", tag_str.style(color::TAGS)).unwrap();
            } else {
                write!(display, " {tag_str}").unwrap();
            }
        }
    }

    display
}

/// Build a 4-column table row for a deployment.
///
/// Columns: [contract_display, full_address, verification_badges, timestamp]
fn build_deployment_row(
    d: &Deployment,
    category: &DisplayCategory,
    fork_deployment_ids: &HashSet<String>,
) -> Vec<String> {
    let name = build_contract_display(d, category, fork_deployment_ids);

    let address = if color::is_color_enabled() {
        format!("{}", d.address.style(color::ADDRESS))
    } else {
        d.address.clone()
    };

    let ver_badge = if color::is_color_enabled() {
        badge::verification_badge_styled(&d.verification.verifiers)
    } else {
        badge::verification_badge(&d.verification.verifiers)
    };

    let timestamp = {
        let ts = d.created_at.format("%Y-%m-%d %H:%M:%S").to_string();
        if color::is_color_enabled() { format!("{}", ts.style(color::TIMESTAMP)) } else { ts }
    };

    vec![name, address, ver_badge, timestamp]
}

/// Build an implementation sub-row for a proxy deployment.
///
/// Format: `└─ impl_display_name` in faint style, with empty columns 1-3.
fn build_impl_row(
    namespace: &str,
    chain_id: u64,
    impl_address: &str,
    impl_lookup: &ImplNameLookup,
) -> Vec<String> {
    let key = (namespace.to_lowercase(), chain_id, impl_address.to_lowercase());
    let impl_name =
        impl_lookup.get(&key).cloned().unwrap_or_else(|| output::truncate_address(impl_address));

    let display = format!("└─ {impl_name}");
    let col0 = if color::is_color_enabled() {
        format!("{}", display.style(color::IMPL_PREFIX))
    } else {
        display
    };

    vec![col0, String::new(), String::new(), String::new()]
}

/// Build table data for all deployments in a type group.
fn build_type_group_table(
    tg: &TypeGroup,
    namespace: &str,
    chain_id: u64,
    fork_deployment_ids: &HashSet<String>,
    impl_lookup: &ImplNameLookup,
) -> output::TableData {
    let mut table = Vec::new();
    for d in &tg.deployments {
        table.push(build_deployment_row(d, &tg.category, fork_deployment_ids));
        if let Some(ref pi) = d.proxy_info {
            table.push(build_impl_row(namespace, chain_id, &pi.implementation, impl_lookup));
        }
    }
    table
}

/// Organize a flat list of deployments into a hierarchical grouping:
/// namespace → chain_id → display category.
///
/// Singletons whose addresses match a proxy's `proxy_info.implementation`
/// address are categorized as `Implementation` instead of `Singleton`.
///
/// The output is suitable for consumption by the tree renderer.
#[cfg(test)]
fn group_deployments<'a>(deployments: &[&'a Deployment]) -> GroupedDeployments<'a> {
    let implementation_keys = collect_implementation_keys(deployments);
    group_deployments_with_implementation_keys(deployments, &implementation_keys)
}

fn group_deployments_with_implementation_keys<'a>(
    deployments: &[&'a Deployment],
    implementation_keys: &HashSet<ImplementationKey>,
) -> GroupedDeployments<'a> {
    let mut result: GroupedDeployments<'a> = BTreeMap::new();

    for &d in deployments {
        let category = match d.deployment_type {
            DeploymentType::Proxy => DisplayCategory::Proxy,
            DeploymentType::Singleton => {
                let key = (d.namespace.clone(), d.chain_id, d.address.to_lowercase());
                if implementation_keys.contains(&key) {
                    DisplayCategory::Implementation
                } else {
                    DisplayCategory::Singleton
                }
            }
            DeploymentType::Library => DisplayCategory::Library,
            DeploymentType::Unknown => DisplayCategory::Unknown,
        };

        let chain_map = result.entry(d.namespace.clone()).or_default();
        let type_groups = chain_map.entry(d.chain_id).or_default();

        if let Some(group) = type_groups.iter_mut().find(|g| g.category == category) {
            group.deployments.push(d);
        } else {
            type_groups.push(TypeGroup { category, deployments: vec![d] });
        }
    }

    // Sort type groups by fixed order and deployments by contract name
    for chain_map in result.values_mut() {
        for type_groups in chain_map.values_mut() {
            type_groups.sort_by_key(|g| type_sort_key(&g.category));
            for group in type_groups.iter_mut() {
                group.deployments.sort_by(|a, b| a.contract_name.cmp(&b.contract_name));
            }
        }
    }

    result
}

// ---------------------------------------------------------------------------
// JSON output types (Go's listJSONOutput schema)
// ---------------------------------------------------------------------------

/// A single deployment entry in the JSON output, matching Go's `listJSONEntry`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ListJsonEntry {
    id: String,
    contract_name: String,
    address: String,
    namespace: String,
    chain_id: u64,
    #[serde(skip_serializing_if = "String::is_empty")]
    label: String,
    #[serde(rename = "type")]
    deployment_type: String,
    #[serde(skip_serializing_if = "is_false")]
    fork: bool,
}

/// Top-level JSON output wrapper matching Go's `listJSONOutput`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ListJsonOutput {
    deployments: Vec<ListJsonEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    other_namespaces: Option<BTreeMap<String, usize>>,
}

fn is_false(v: &bool) -> bool {
    !v
}

fn build_list_json_output(result: &ListResult) -> ListJsonOutput {
    let entries: Vec<ListJsonEntry> = result
        .deployments
        .iter()
        .map(|d| ListJsonEntry {
            id: d.id.clone(),
            contract_name: d.contract_name.clone(),
            address: d.address.clone(),
            namespace: d.namespace.clone(),
            chain_id: d.chain_id,
            label: d.label.clone(),
            deployment_type: d.deployment_type.to_string(),
            fork: result.fork_deployment_ids.contains(&d.id),
        })
        .collect();

    let other_namespaces = if entries.is_empty() && !result.other_namespaces.is_empty() {
        Some(result.other_namespaces.clone())
    } else {
        None
    };

    ListJsonOutput { deployments: entries, other_namespaces }
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    network: Option<String>,
    namespace: Option<String>,
    deployment_type: Option<String>,
    tag: Option<String>,
    contract: Option<String>,
    label: Option<String>,
    fork: bool,
    no_fork: bool,
    json: bool,
) -> anyhow::Result<()> {
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
    let all_deployments = registry.list_deployments();

    let filters = DeploymentFilters {
        network,
        namespace,
        deployment_type,
        tag,
        contract,
        label,
        fork,
        no_fork,
    };

    // Compute fork deployment IDs: namespace-based + snapshot diff when fork mode is active
    let mut fork_deployment_ids = collect_fork_deployment_ids(&all_deployments);
    let fork_mode_active;
    {
        let treb_dir = cwd.join(".treb");
        let mut store = treb_registry::ForkStateStore::new(&treb_dir);
        fork_mode_active = store.load().is_ok() && store.is_fork_mode_active();
        if fork_mode_active {
            if let Some(ref snap_dir) = store.data().snapshot_dir {
                let snap_path =
                    std::path::PathBuf::from(snap_dir).join(treb_registry::DEPLOYMENTS_FILE);
                if let Some(snapshot_ids) = load_fork_snapshot_ids(&snap_path) {
                    for d in &all_deployments {
                        if !snapshot_ids.contains(&d.id) {
                            fork_deployment_ids.insert(d.id.clone());
                        }
                    }
                }
            }
        }
    }

    let implementation_keys = collect_implementation_keys(&all_deployments);

    // When --fork is passed and fork mode is active, use snapshot-diff filtering
    // instead of the namespace-prefix filter in filter_deployments.
    let filtered = if fork && fork_mode_active {
        let base_filters = DeploymentFilters {
            network: filters.network.clone(),
            namespace: filters.namespace.clone(),
            deployment_type: filters.deployment_type.clone(),
            tag: filters.tag.clone(),
            contract: filters.contract.clone(),
            label: filters.label.clone(),
            fork: false,    // skip namespace-based fork filter
            no_fork: false,
        };
        filter_deployments(&all_deployments, &base_filters)
            .into_iter()
            .filter(|d| fork_deployment_ids.contains(&d.id))
            .collect()
    } else {
        filter_deployments(&all_deployments, &filters)
    };

    // Build other_namespaces when filtered is empty and namespace filter is set
    let other_namespaces = if filtered.is_empty() {
        if let Some(ref ns) = filters.namespace {
            build_other_namespaces(&all_deployments, &filters, ns)
        } else {
            BTreeMap::new()
        }
    } else {
        BTreeMap::new()
    };
    let network_names = resolve_network_names(&cwd, &filtered, filters.network.as_deref());
    let selected_network_label =
        format_selected_network_label(filters.network.as_deref(), &network_names);

    let result =
        ListResult { deployments: filtered, other_namespaces, network_names, fork_deployment_ids };

    if json {
        let json_output = build_list_json_output(&result);
        output::print_json(&json_output)?;
    } else if result.deployments.is_empty() {
        if let Some(ref ns) = filters.namespace {
            if result.other_namespaces.is_empty() {
                println!("No deployments found");
            } else {
                let hint = format_namespace_discovery_hint(
                    ns,
                    selected_network_label.as_deref(),
                    &result.other_namespaces,
                );
                print!("{hint}");
            }
        } else {
            println!("No deployments found");
        }
    } else {
        let grouped =
            group_deployments_with_implementation_keys(&result.deployments, &implementation_keys);
        let impl_lookup = build_impl_name_lookup(&all_deployments);

        // First pass: collect all table data for global column width calculation
        let mut all_tables: Vec<output::TableData> = Vec::new();
        for (namespace, chains) in &grouped {
            for (&chain_id, type_groups) in chains {
                for tg in type_groups {
                    all_tables.push(build_type_group_table(
                        tg,
                        namespace,
                        chain_id,
                        &result.fork_deployment_ids,
                        &impl_lookup,
                    ));
                }
            }
        }

        let widths = output::calculate_column_widths(&all_tables);

        // Second pass: render with headers and table data
        let mut table_idx = 0;
        for (namespace, chains) in &grouped {
            println!("{}", format_namespace_header(namespace));

            let chain_count = chains.len();
            for (chain_idx, (chain_id, type_groups)) in chains.iter().enumerate() {
                let is_last_chain = chain_idx == chain_count - 1;
                let chain_label = format_chain_header_label(*chain_id, &result.network_names);

                println!("{}", format_chain_header(&chain_label, is_last_chain));

                let cont_prefix = if is_last_chain { "  " } else { "│ " };

                // Blank continuation line after chain header
                println!("{cont_prefix}");

                for (tg_idx, tg) in type_groups.iter().enumerate() {
                    // Blank line between type sections
                    if tg_idx > 0 {
                        println!("{cont_prefix}");
                    }

                    // Type section header
                    if color::is_color_enabled() {
                        println!(
                            "{cont_prefix}{}",
                            tg.category.to_string().style(color::SECTION_HEADER)
                        );
                    } else {
                        println!("{cont_prefix}{}", tg.category);
                    }

                    // Deployment table rows
                    let rendered = output::render_table_with_widths(
                        &all_tables[table_idx],
                        &widths,
                        cont_prefix,
                    );
                    if !rendered.is_empty() {
                        println!("{rendered}");
                    }
                    table_idx += 1;
                }

                if is_last_chain {
                    println!();
                } else {
                    println!("{cont_prefix}");
                }
            }
        }

        println!("Total deployments: {}", result.deployments.len());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use std::{
        collections::HashMap,
        sync::{Mutex, MutexGuard, OnceLock},
    };
    use tempfile::TempDir;
    use treb_core::types::{
        ArtifactInfo, DeploymentMethod, DeploymentStrategy, DeploymentType, ProxyInfo,
        VerificationInfo, VerificationStatus, VerifierStatus,
    };

    fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().expect("env test lock poisoned")
    }

    struct EnvVarGuard {
        key: &'static str,
        old: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let old = std::env::var(key).ok();
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, old }
        }

        fn unset(key: &'static str) -> Self {
            let old = std::env::var(key).ok();
            unsafe {
                std::env::remove_var(key);
            }
            Self { key, old }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.old {
                    Some(value) => std::env::set_var(self.key, value),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    fn make_deployment(
        id: &str,
        namespace: &str,
        chain_id: u64,
        contract_name: &str,
        label: &str,
        dtype: DeploymentType,
        tags: Option<Vec<String>>,
    ) -> Deployment {
        Deployment {
            id: id.into(),
            namespace: namespace.into(),
            chain_id,
            contract_name: contract_name.into(),
            label: label.into(),
            address: format!("0x{:040x}", chain_id),
            deployment_type: dtype,
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
                path: "out/Contract.json".into(),
                compiler_version: "0.8.24".into(),
                bytecode_hash: "0xabc".into(),
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
            tags,
            created_at: Utc.with_ymd_and_hms(2025, 1, 15, 10, 30, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2025, 1, 15, 10, 30, 0).unwrap(),
        }
    }

    fn no_filters() -> DeploymentFilters {
        DeploymentFilters {
            network: None,
            namespace: None,
            deployment_type: None,
            tag: None,
            contract: None,
            label: None,
            fork: false,
            no_fork: false,
        }
    }

    fn sample_deployments() -> Vec<Deployment> {
        vec![
            make_deployment(
                "mainnet/42220/FPMM:v3.0.0",
                "mainnet",
                42220,
                "FPMM",
                "v3.0.0",
                DeploymentType::Singleton,
                Some(vec!["core".into()]),
            ),
            make_deployment(
                "mainnet/42220/FPMMFactory:v3.0.0",
                "mainnet",
                42220,
                "FPMMFactory",
                "v3.0.0",
                DeploymentType::Singleton,
                None,
            ),
            make_deployment(
                "testnet/11155111/Counter:v1",
                "testnet",
                11155111,
                "Counter",
                "v1",
                DeploymentType::Library,
                Some(vec!["test".into(), "core".into()]),
            ),
            make_deployment(
                "fork/42220/FPMM:dev",
                "fork/42220",
                42220,
                "FPMM",
                "dev",
                DeploymentType::Proxy,
                None,
            ),
        ]
    }

    #[test]
    fn no_filters_returns_all() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let result = filter_deployments(&refs, &no_filters());
        assert_eq!(result.len(), 4);
    }

    #[test]
    fn filter_by_namespace() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.namespace = Some("mainnet".into());
        let result = filter_deployments(&refs, &filters);
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|d| d.namespace == "mainnet"));
    }

    #[test]
    fn filter_by_network_chain_id() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.network = Some("11155111".into());
        let result = filter_deployments(&refs, &filters);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].chain_id, 11155111);
    }

    #[test]
    fn filter_by_named_network() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.network = Some("sepolia".into());
        let result = filter_deployments(&refs, &filters);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].chain_id, 11155111);
    }

    #[test]
    fn filter_by_deployment_type() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.deployment_type = Some("SINGLETON".into());
        let result = filter_deployments(&refs, &filters);
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|d| d.deployment_type == DeploymentType::Singleton));
    }

    #[test]
    fn filter_by_deployment_type_case_insensitive() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.deployment_type = Some("proxy".into());
        let result = filter_deployments(&refs, &filters);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].deployment_type, DeploymentType::Proxy);
    }

    #[test]
    fn filter_by_tag() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.tag = Some("core".into());
        let result = filter_deployments(&refs, &filters);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn filter_by_tag_no_match() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.tag = Some("nonexistent".into());
        let result = filter_deployments(&refs, &filters);
        assert!(result.is_empty());
    }

    #[test]
    fn filter_by_contract_name() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.contract = Some("fpmm".into());
        let result = filter_deployments(&refs, &filters);
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|d| d.contract_name.eq_ignore_ascii_case("fpmm")));
    }

    #[test]
    fn filter_by_label() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.label = Some("v3.0.0".into());
        let result = filter_deployments(&refs, &filters);
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|d| d.label == "v3.0.0"));
    }

    #[test]
    fn filter_fork_only() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.fork = true;
        let result = filter_deployments(&refs, &filters);
        assert_eq!(result.len(), 1);
        assert!(result[0].namespace.starts_with("fork/"));
    }

    #[test]
    fn filter_no_fork() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.no_fork = true;
        let result = filter_deployments(&refs, &filters);
        assert_eq!(result.len(), 3);
        assert!(result.iter().all(|d| !d.namespace.starts_with("fork/")));
    }

    #[test]
    fn combined_filters_and_logic() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.namespace = Some("mainnet".into());
        filters.contract = Some("FPMM".into());
        let result = filter_deployments(&refs, &filters);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "mainnet/42220/FPMM:v3.0.0");
    }

    #[test]
    fn combined_filters_empty_result() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let mut filters = no_filters();
        filters.namespace = Some("mainnet".into());
        filters.deployment_type = Some("LIBRARY".into());
        let result = filter_deployments(&refs, &filters);
        assert!(result.is_empty());
    }

    #[test]
    fn empty_deployments_returns_empty() {
        let refs: Vec<&Deployment> = vec![];
        let result = filter_deployments(&refs, &no_filters());
        assert!(result.is_empty());
    }

    // -----------------------------------------------------------------------
    // Grouping tests
    // -----------------------------------------------------------------------

    #[test]
    fn group_empty_returns_empty_btreemap() {
        let refs: Vec<&Deployment> = vec![];
        let grouped = group_deployments(&refs);
        assert!(grouped.is_empty());
    }

    #[test]
    fn group_single_namespace_chain_mixed_types() {
        // AC: 2 singletons + 1 proxy, all mainnet/42220
        let deployments = [
            make_deployment(
                "mainnet/42220/FPMM",
                "mainnet",
                42220,
                "FPMM",
                "",
                DeploymentType::Singleton,
                None,
            ),
            make_deployment(
                "mainnet/42220/FPMMFactory",
                "mainnet",
                42220,
                "FPMMFactory",
                "",
                DeploymentType::Singleton,
                None,
            ),
            make_deployment(
                "mainnet/42220/TransparentUpgradeableProxy",
                "mainnet",
                42220,
                "TransparentUpgradeableProxy",
                "",
                DeploymentType::Proxy,
                None,
            ),
        ];
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let grouped = group_deployments(&refs);

        // One namespace
        assert_eq!(grouped.len(), 1);
        assert!(grouped.contains_key("mainnet"));

        // One chain
        let chains = &grouped["mainnet"];
        assert_eq!(chains.len(), 1);
        assert!(chains.contains_key(&42220));

        // Two type groups: Proxy first (sort key 0), then Singleton (sort key 2)
        let type_groups = &chains[&42220];
        assert_eq!(type_groups.len(), 2);
        assert_eq!(type_groups[0].category, DisplayCategory::Proxy);
        assert_eq!(type_groups[0].deployments.len(), 1);
        assert_eq!(type_groups[0].deployments[0].contract_name, "TransparentUpgradeableProxy");
        assert_eq!(type_groups[1].category, DisplayCategory::Singleton);
        assert_eq!(type_groups[1].deployments.len(), 2);
        assert_eq!(type_groups[1].deployments[0].contract_name, "FPMM");
        assert_eq!(type_groups[1].deployments[1].contract_name, "FPMMFactory");
    }

    #[test]
    fn group_multiple_namespaces_sorted_alphabetically() {
        let deployments = [
            make_deployment("z-ns/1/A", "z-ns", 1, "A", "", DeploymentType::Singleton, None),
            make_deployment("a-ns/1/B", "a-ns", 1, "B", "", DeploymentType::Singleton, None),
            make_deployment("m-ns/1/C", "m-ns", 1, "C", "", DeploymentType::Singleton, None),
        ];
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let grouped = group_deployments(&refs);

        let namespaces: Vec<&String> = grouped.keys().collect();
        assert_eq!(namespaces, vec!["a-ns", "m-ns", "z-ns"]);
    }

    #[test]
    fn group_chain_ids_sorted_numerically() {
        let deployments = [
            make_deployment("ns/999/A", "ns", 999, "A", "", DeploymentType::Singleton, None),
            make_deployment("ns/1/B", "ns", 1, "B", "", DeploymentType::Singleton, None),
            make_deployment("ns/42/C", "ns", 42, "C", "", DeploymentType::Singleton, None),
        ];
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let grouped = group_deployments(&refs);

        let chain_ids: Vec<u64> = grouped["ns"].keys().copied().collect();
        assert_eq!(chain_ids, vec![1, 42, 999]);
    }

    #[test]
    fn group_type_order_proxy_singleton_library() {
        let deployments = [
            make_deployment("ns/1/Lib", "ns", 1, "Lib", "", DeploymentType::Library, None),
            make_deployment("ns/1/Sing", "ns", 1, "Sing", "", DeploymentType::Singleton, None),
            make_deployment("ns/1/Prox", "ns", 1, "Prox", "", DeploymentType::Proxy, None),
        ];
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let grouped = group_deployments(&refs);

        let type_groups = &grouped["ns"][&1];
        assert_eq!(type_groups.len(), 3);
        assert_eq!(type_groups[0].category, DisplayCategory::Proxy);
        assert_eq!(type_groups[1].category, DisplayCategory::Singleton);
        assert_eq!(type_groups[2].category, DisplayCategory::Library);
    }

    #[test]
    fn group_deployments_sorted_by_contract_name() {
        let deployments = [
            make_deployment("ns/1/Zeta", "ns", 1, "Zeta", "", DeploymentType::Singleton, None),
            make_deployment("ns/1/Alpha", "ns", 1, "Alpha", "", DeploymentType::Singleton, None),
            make_deployment("ns/1/Mid", "ns", 1, "Mid", "", DeploymentType::Singleton, None),
        ];
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let grouped = group_deployments(&refs);

        let names: Vec<&str> =
            grouped["ns"][&1][0].deployments.iter().map(|d| d.contract_name.as_str()).collect();
        assert_eq!(names, vec!["Alpha", "Mid", "Zeta"]);
    }

    #[test]
    fn group_mixed_namespaces_chains_and_types() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let grouped = group_deployments(&refs);

        // 3 namespaces: fork/42220, mainnet, testnet
        assert_eq!(grouped.len(), 3);
        let namespaces: Vec<&String> = grouped.keys().collect();
        assert_eq!(namespaces, vec!["fork/42220", "mainnet", "testnet"]);

        // mainnet has 1 chain (42220) with 1 type group (Singleton, 2 entries)
        let mainnet_chains = &grouped["mainnet"];
        assert_eq!(mainnet_chains.len(), 1);
        let mainnet_types = &mainnet_chains[&42220];
        assert_eq!(mainnet_types.len(), 1);
        assert_eq!(mainnet_types[0].category, DisplayCategory::Singleton);
        assert_eq!(mainnet_types[0].deployments.len(), 2);

        // testnet has 1 chain (11155111) with 1 type group (Library, 1 entry)
        let testnet_types = &grouped["testnet"][&11155111];
        assert_eq!(testnet_types.len(), 1);
        assert_eq!(testnet_types[0].category, DisplayCategory::Library);

        // fork/42220 has 1 chain (42220) with 1 type group (Proxy, 1 entry)
        let fork_types = &grouped["fork/42220"][&42220];
        assert_eq!(fork_types.len(), 1);
        assert_eq!(fork_types[0].category, DisplayCategory::Proxy);
    }

    #[test]
    fn group_implementation_separation() {
        // A proxy whose proxy_info.implementation matches a singleton's address
        // should cause that singleton to be categorized as Implementation.
        let impl_address = "0x959597fD009876e6f53EbdB2F1c1Bc3f994579dF";
        let mut proxy =
            make_deployment("ns/1/MyProxy", "ns", 1, "MyProxy", "", DeploymentType::Proxy, None);
        proxy.address = "0x22A81Fc75b0d5F7cac19cABa9F0c3719b3897F03".into();
        proxy.proxy_info = Some(ProxyInfo {
            proxy_type: "UUPS".into(),
            implementation: impl_address.into(),
            admin: String::new(),
            history: vec![],
        });

        let mut impl_singleton =
            make_deployment("ns/1/MyImpl", "ns", 1, "MyImpl", "", DeploymentType::Singleton, None);
        impl_singleton.address = impl_address.into();

        let regular_singleton = make_deployment(
            "ns/1/RegularContract",
            "ns",
            1,
            "RegularContract",
            "",
            DeploymentType::Singleton,
            None,
        );

        let deployments = [proxy, impl_singleton, regular_singleton];
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let grouped = group_deployments(&refs);

        // Should have 3 groups: Proxy, Implementation, Singleton
        let type_groups = &grouped["ns"][&1];
        assert_eq!(type_groups.len(), 3, "expected 3 groups: Proxy, Implementation, Singleton");
        assert_eq!(type_groups[0].category, DisplayCategory::Proxy);
        assert_eq!(type_groups[0].deployments.len(), 1);
        assert_eq!(type_groups[0].deployments[0].contract_name, "MyProxy");

        assert_eq!(type_groups[1].category, DisplayCategory::Implementation);
        assert_eq!(type_groups[1].deployments.len(), 1);
        assert_eq!(type_groups[1].deployments[0].contract_name, "MyImpl");

        assert_eq!(type_groups[2].category, DisplayCategory::Singleton);
        assert_eq!(type_groups[2].deployments.len(), 1);
        assert_eq!(type_groups[2].deployments[0].contract_name, "RegularContract");
    }

    #[test]
    fn group_implementation_case_insensitive_address_match() {
        // Implementation address matching should be case-insensitive (checksummed vs lowercase)
        let mut proxy =
            make_deployment("ns/1/MyProxy", "ns", 1, "MyProxy", "", DeploymentType::Proxy, None);
        proxy.proxy_info = Some(ProxyInfo {
            proxy_type: "UUPS".into(),
            implementation: "0xAbCdEf0123456789AbCdEf0123456789AbCdEf01".into(),
            admin: String::new(),
            history: vec![],
        });

        let mut impl_singleton =
            make_deployment("ns/1/MyImpl", "ns", 1, "MyImpl", "", DeploymentType::Singleton, None);
        impl_singleton.address = "0xabcdef0123456789abcdef0123456789abcdef01".into();

        let deployments = [proxy, impl_singleton];
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let grouped = group_deployments(&refs);

        let type_groups = &grouped["ns"][&1];
        assert_eq!(type_groups.len(), 2);
        assert_eq!(type_groups[0].category, DisplayCategory::Proxy);
        assert_eq!(type_groups[1].category, DisplayCategory::Implementation);
    }

    #[test]
    fn filtered_implementation_keeps_category_when_proxy_is_filtered_out() {
        let impl_address = "0x959597fD009876e6f53EbdB2F1c1Bc3f994579dF";
        let mut proxy =
            make_deployment("ns/1/MyProxy", "ns", 1, "MyProxy", "", DeploymentType::Proxy, None);
        proxy.proxy_info = Some(ProxyInfo {
            proxy_type: "UUPS".into(),
            implementation: impl_address.into(),
            admin: String::new(),
            history: vec![],
        });

        let mut implementation =
            make_deployment("ns/1/MyImpl", "ns", 1, "MyImpl", "", DeploymentType::Singleton, None);
        implementation.address = impl_address.into();

        let deployments = [proxy, implementation];
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let implementation_keys = collect_implementation_keys(&refs);

        let mut filters = no_filters();
        filters.contract = Some("MyImpl".into());
        let filtered = filter_deployments(&refs, &filters);
        let grouped = group_deployments_with_implementation_keys(&filtered, &implementation_keys);

        let type_groups = &grouped["ns"][&1];
        assert_eq!(type_groups.len(), 1);
        assert_eq!(type_groups[0].category, DisplayCategory::Implementation);
        assert_eq!(type_groups[0].deployments[0].contract_name, "MyImpl");
    }

    #[test]
    fn group_unknown_deployments_stay_unknown() {
        let deployments = [make_deployment(
            "ns/1/Mystery",
            "ns",
            1,
            "Mystery",
            "",
            DeploymentType::Unknown,
            None,
        )];
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let grouped = group_deployments(&refs);

        let type_groups = &grouped["ns"][&1];
        assert_eq!(type_groups.len(), 1);
        assert_eq!(type_groups[0].category, DisplayCategory::Unknown);
        assert_eq!(type_groups[0].deployments[0].contract_name, "Mystery");
    }

    // -----------------------------------------------------------------------
    // Table row rendering tests
    // -----------------------------------------------------------------------

    #[test]
    fn deployment_row_proxy_has_implementation_row() {
        let _lock = env_lock();
        let _no_color = EnvVarGuard::set("NO_COLOR", "1");
        color::color_enabled(false);

        let mut d = make_deployment(
            "mainnet/42220/TransparentUpgradeableProxy:FPMMFactory",
            "mainnet",
            42220,
            "TransparentUpgradeableProxy",
            "FPMMFactory",
            DeploymentType::Proxy,
            None,
        );
        d.address = "0x22A81Fc75b0d5F7cac19cABa9F0c3719b3897F03".into();
        d.proxy_info = Some(ProxyInfo {
            proxy_type: "UUPS".into(),
            implementation: "0x959597fD009876e6f53EbdB2F1c1Bc3f994579dF".into(),
            admin: String::new(),
            history: vec![],
        });

        let fork_ids = HashSet::new();
        let impl_lookup = ImplNameLookup::new();
        let tg = TypeGroup { category: DisplayCategory::Proxy, deployments: vec![&d] };
        let table = build_type_group_table(&tg, "mainnet", 42220, &fork_ids, &impl_lookup);

        assert_eq!(table.len(), 2, "proxy deployment should produce 2 table rows");
        assert!(
            table[0][0].contains("TransparentUpgradeableProxy:FPMMFactory"),
            "first row col0 should contain contract name:label"
        );
        assert!(
            table[0][1].contains("0x22A81Fc75b0d5F7cac19cABa9F0c3719b3897F03"),
            "first row col1 should contain full address"
        );
        assert!(
            table[0][2].contains("UNVERIFIED"),
            "first row col2 should contain verification badge"
        );
        assert!(table[1][0].contains("└─"), "second row should be an implementation row");
    }

    #[test]
    fn implementation_row_uses_lookup_built_from_full_registry_context() {
        let _lock = env_lock();
        let _no_color = EnvVarGuard::set("NO_COLOR", "1");
        color::color_enabled(false);

        let mut proxy = make_deployment(
            "mainnet/42220/TransparentUpgradeableProxy:FPMMFactory",
            "mainnet",
            42220,
            "TransparentUpgradeableProxy",
            "FPMMFactory",
            DeploymentType::Proxy,
            None,
        );
        proxy.address = "0x22A81Fc75b0d5F7cac19cABa9F0c3719b3897F03".into();
        proxy.proxy_info = Some(ProxyInfo {
            proxy_type: "Transparent".into(),
            implementation: "0x959597fD009876e6f53EbdB2F1c1Bc3f994579dF".into(),
            admin: String::new(),
            history: vec![],
        });

        let mut implementation = make_deployment(
            "mainnet/42220/FPMMFactory:v3.0.0",
            "mainnet",
            42220,
            "FPMMFactory",
            "v3.0.0",
            DeploymentType::Singleton,
            None,
        );
        implementation.address = "0x959597fD009876e6f53EbdB2F1c1Bc3f994579dF".into();

        let deployments = [proxy, implementation];
        let full_refs: Vec<&Deployment> = deployments.iter().collect();
        let fork_ids = HashSet::new();
        let impl_lookup = build_impl_name_lookup(&full_refs);
        let tg = TypeGroup { category: DisplayCategory::Proxy, deployments: vec![&deployments[0]] };
        let table = build_type_group_table(&tg, "mainnet", 42220, &fork_ids, &impl_lookup);

        assert_eq!(table.len(), 2);
        assert!(
            table[1][0].contains("└─ FPMMFactory:v3.0.0"),
            "implementation row should resolve from full lookup: {:?}",
            table[1][0]
        );
    }

    #[test]
    fn deployment_row_non_proxy_has_no_impl_row() {
        let _lock = env_lock();
        let _no_color = EnvVarGuard::set("NO_COLOR", "1");
        color::color_enabled(false);

        let d = make_deployment(
            "mainnet/42220/FPMM",
            "mainnet",
            42220,
            "FPMM",
            "",
            DeploymentType::Singleton,
            None,
        );

        let fork_ids = HashSet::new();
        let impl_lookup = ImplNameLookup::new();
        let tg = TypeGroup { category: DisplayCategory::Singleton, deployments: vec![&d] };
        let table = build_type_group_table(&tg, "mainnet", 42220, &fork_ids, &impl_lookup);

        assert_eq!(table.len(), 1, "non-proxy deployment should produce 1 table row");
    }

    #[test]
    fn contract_display_includes_fork_badge() {
        let _lock = env_lock();
        let _no_color = EnvVarGuard::set("NO_COLOR", "1");
        color::color_enabled(false);

        let d = make_deployment(
            "fork/42220/FPMM:dev",
            "fork/42220",
            42220,
            "FPMM",
            "dev",
            DeploymentType::Proxy,
            None,
        );
        let mut fork_ids = HashSet::new();
        fork_ids.insert(d.id.clone());

        let display = build_contract_display(&d, &DisplayCategory::Proxy, &fork_ids);
        assert!(display.contains("[fork]"), "fork deployment display should contain [fork] badge");
    }

    #[test]
    fn deployment_row_uses_styled_badges_when_color_enabled() {
        let _lock = env_lock();
        let _no_color = EnvVarGuard::unset("NO_COLOR");
        let _term = EnvVarGuard::set("TERM", "xterm-256color");
        owo_colors::set_override(true);
        color::color_enabled(false);

        let mut d = make_deployment(
            "fork/42220/FPMM:dev",
            "fork/42220",
            42220,
            "FPMM",
            "dev",
            DeploymentType::Proxy,
            None,
        );
        d.verification.verifiers.insert(
            "etherscan".into(),
            VerifierStatus { status: "VERIFIED".into(), url: String::new(), reason: String::new() },
        );

        let mut fork_ids = HashSet::new();
        fork_ids.insert(d.id.clone());
        let row = build_deployment_row(&d, &DisplayCategory::Proxy, &fork_ids);

        assert!(
            row[0].contains('\x1b'),
            "styled contract name should contain ANSI codes: {:?}",
            row[0]
        );
        assert!(
            row[2].contains("e[✔︎]"),
            "styled verification should contain Go-format verifier text"
        );
        assert!(row[0].contains("[fork]"), "styled row should include the fork badge text");
    }

    // -----------------------------------------------------------------------
    // Other-namespaces and fork deployment ID tests
    // -----------------------------------------------------------------------

    #[test]
    fn build_other_namespaces_excludes_specified() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let other = build_other_namespaces(&refs, &no_filters(), "mainnet");

        assert!(!other.contains_key("mainnet"));
        assert_eq!(other["testnet"], 1);
        assert_eq!(other["fork/42220"], 1);
    }

    #[test]
    fn build_other_namespaces_empty_when_all_match() {
        let deployments =
            [make_deployment("ns/1/A", "ns", 1, "A", "", DeploymentType::Singleton, None)];
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let other = build_other_namespaces(&refs, &no_filters(), "ns");

        assert!(other.is_empty());
    }

    #[test]
    fn build_other_namespaces_counts_per_namespace() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let other = build_other_namespaces(&refs, &no_filters(), "nonexistent");

        assert_eq!(other["mainnet"], 2);
        assert_eq!(other["testnet"], 1);
        assert_eq!(other["fork/42220"], 1);
    }

    #[test]
    fn build_other_namespaces_case_insensitive_exclude() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let other = build_other_namespaces(&refs, &no_filters(), "MAINNET");

        assert!(!other.contains_key("mainnet"));
        assert_eq!(other.len(), 2);
    }

    #[test]
    fn build_other_namespaces_preserves_network_filter() {
        let deployments = [
            make_deployment("default/1/A", "default", 1, "A", "", DeploymentType::Singleton, None),
            make_deployment(
                "production/1/B",
                "production",
                1,
                "B",
                "",
                DeploymentType::Singleton,
                None,
            ),
            make_deployment(
                "sandbox/11155111/C",
                "sandbox",
                11155111,
                "C",
                "",
                DeploymentType::Singleton,
                None,
            ),
            make_deployment(
                "staging/11155111/D",
                "staging",
                11155111,
                "D",
                "",
                DeploymentType::Singleton,
                None,
            ),
        ];
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let filters = DeploymentFilters {
            network: Some("mainnet".into()),
            namespace: Some("staging".into()),
            deployment_type: None,
            tag: None,
            contract: None,
            label: None,
            fork: false,
            no_fork: false,
        };

        let other = build_other_namespaces(&refs, &filters, "staging");

        assert_eq!(other["default"], 1);
        assert_eq!(other["production"], 1);
        assert!(!other.contains_key("sandbox"));
        assert!(!other.contains_key("staging"));
    }

    #[test]
    fn resolve_network_names_uses_config_aliases_in_chain_labels() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("foundry.toml"),
            "[profile.default]\n\n[rpc_endpoints]\nethlive = \"https://example.invalid\"\n",
        )
        .unwrap();
        let deployments = [make_deployment(
            "default/1/A",
            "default",
            1,
            "A",
            "",
            DeploymentType::Singleton,
            None,
        )];
        let refs: Vec<&Deployment> = deployments.iter().collect();

        let network_names = resolve_network_names(tmp.path(), &refs, None);

        assert_eq!(network_names.get(&1).map(String::as_str), Some("ethlive"));
        assert_eq!(format_chain_header_label(1, &network_names), "ethlive (1)");
    }

    #[test]
    fn resolve_network_names_adds_named_chain_for_numeric_selected_network() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("foundry.toml"), "[profile.default]\n").unwrap();
        let refs: Vec<&Deployment> = Vec::new();

        let network_names = resolve_network_names(tmp.path(), &refs, Some("42220"));

        assert_eq!(network_names.get(&42220).map(String::as_str), Some("celo"));
        assert_eq!(
            format_selected_network_label(Some("42220"), &network_names).as_deref(),
            Some("celo (42220)")
        );
    }

    #[test]
    fn collect_fork_ids_finds_fork_namespaces() {
        let deployments = sample_deployments();
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let fork_ids = collect_fork_deployment_ids(&refs);

        assert_eq!(fork_ids.len(), 1);
        assert!(fork_ids.contains("fork/42220/FPMM:dev"));
    }

    #[test]
    fn collect_fork_ids_empty_for_non_fork() {
        let deployments =
            [make_deployment("ns/1/A", "ns", 1, "A", "", DeploymentType::Singleton, None)];
        let refs: Vec<&Deployment> = deployments.iter().collect();
        let fork_ids = collect_fork_deployment_ids(&refs);

        assert!(fork_ids.is_empty());
    }

    // -----------------------------------------------------------------------
    // Namespace discovery hint formatting tests
    // -----------------------------------------------------------------------

    #[test]
    fn hint_without_network_and_with_other_namespaces() {
        let mut other = BTreeMap::new();
        other.insert("default".into(), 3usize);
        other.insert("production".into(), 1usize);

        let hint = format_namespace_discovery_hint("staging", None, &other);

        assert!(
            hint.starts_with("No deployments found in namespace \"staging\"\n"),
            "hint should start with namespace message, got: {hint:?}"
        );
        assert!(hint.contains("Other namespaces with deployments:"));
        assert!(hint.contains("  default             3 deployments"));
        assert!(hint.contains("  production          1 deployment\n"));
        assert!(
            hint.contains(
                "Use --namespace <name> or `treb config set namespace <name>` to switch."
            )
        );
    }

    #[test]
    fn hint_with_network_filter() {
        let other = BTreeMap::new();
        let hint = format_namespace_discovery_hint("staging", Some("42220"), &other);

        assert!(
            hint.starts_with("No deployments found in namespace \"staging\" on 42220\n"),
            "hint should include network filter, got: {hint:?}"
        );
        // No other namespaces, so no extra sections
        assert!(!hint.contains("Other namespaces"));
    }

    #[test]
    fn hint_singular_deployment_count() {
        let mut other = BTreeMap::new();
        other.insert("default".into(), 1usize);

        let hint = format_namespace_discovery_hint("staging", None, &other);

        assert!(hint.contains("1 deployment\n"), "count=1 should use singular 'deployment'");
        assert!(!hint.contains("1 deployments"), "count=1 should NOT use plural 'deployments'");
    }

    #[test]
    fn hint_plural_deployment_count() {
        let mut other = BTreeMap::new();
        other.insert("default".into(), 5usize);

        let hint = format_namespace_discovery_hint("staging", None, &other);

        assert!(hint.contains("5 deployments"), "count>1 should use plural 'deployments'");
    }

    #[test]
    fn hint_namespaces_sorted_alphabetically() {
        let mut other = BTreeMap::new();
        other.insert("z-ns".into(), 1usize);
        other.insert("a-ns".into(), 2usize);
        other.insert("m-ns".into(), 3usize);

        let hint = format_namespace_discovery_hint("staging", None, &other);

        let lines: Vec<&str> = hint.lines().collect();
        // Find the namespace lines (indented with 2 spaces)
        let ns_lines: Vec<&str> = lines.iter().filter(|l| l.starts_with("  ")).copied().collect();
        assert_eq!(ns_lines.len(), 3);
        assert!(ns_lines[0].contains("a-ns"), "first should be a-ns");
        assert!(ns_lines[1].contains("m-ns"), "second should be m-ns");
        assert!(ns_lines[2].contains("z-ns"), "third should be z-ns");
    }

    #[test]
    fn hint_empty_other_namespaces_shows_only_first_line() {
        let other = BTreeMap::new();
        let hint = format_namespace_discovery_hint("staging", None, &other);

        assert_eq!(
            hint, "No deployments found in namespace \"staging\"\n",
            "empty other_namespaces should produce only the first line"
        );
    }

    // -- build_list_json_output tests --

    #[test]
    fn json_output_wraps_deployments_in_object() {
        let d = make_deployment("ns/1/A", "ns", 1, "A", "", DeploymentType::Singleton, None);
        let result = ListResult {
            deployments: vec![&d],
            other_namespaces: BTreeMap::new(),
            network_names: BTreeMap::new(),
            fork_deployment_ids: HashSet::new(),
        };

        let output = build_list_json_output(&result);
        let json = serde_json::to_value(&output).unwrap();

        assert!(json.is_object(), "top-level should be an object");
        assert!(json.get("deployments").unwrap().is_array(), "should have deployments array");
        assert_eq!(json["deployments"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn json_output_entry_has_exactly_go_fields() {
        let d = make_deployment("ns/1/A", "ns", 1, "A", "lbl", DeploymentType::Proxy, None);
        let result = ListResult {
            deployments: vec![&d],
            other_namespaces: BTreeMap::new(),
            network_names: BTreeMap::new(),
            fork_deployment_ids: HashSet::new(),
        };

        let output = build_list_json_output(&result);
        let json = serde_json::to_value(&output).unwrap();
        let entry = &json["deployments"][0];

        assert_eq!(entry["id"], "ns/1/A");
        assert_eq!(entry["contractName"], "A");
        assert_eq!(entry["address"], format!("0x{:040x}", 1));
        assert_eq!(entry["namespace"], "ns");
        assert_eq!(entry["chainId"], 1);
        assert_eq!(entry["label"], "lbl");
        assert_eq!(entry["type"], "PROXY");
        // fork should be omitted (false)
        assert!(entry.get("fork").is_none(), "fork should be omitted when false");
    }

    #[test]
    fn json_output_label_omitted_when_empty() {
        let d = make_deployment("ns/1/A", "ns", 1, "A", "", DeploymentType::Singleton, None);
        let result = ListResult {
            deployments: vec![&d],
            other_namespaces: BTreeMap::new(),
            network_names: BTreeMap::new(),
            fork_deployment_ids: HashSet::new(),
        };

        let output = build_list_json_output(&result);
        let json = serde_json::to_value(&output).unwrap();
        let entry = &json["deployments"][0];

        assert!(entry.get("label").is_none(), "label should be omitted when empty");
    }

    #[test]
    fn json_output_fork_true_for_fork_deployment() {
        let d = make_deployment(
            "fork/42220/A",
            "fork/42220",
            42220,
            "A",
            "",
            DeploymentType::Singleton,
            None,
        );
        let mut fork_ids = HashSet::new();
        fork_ids.insert("fork/42220/A".to_string());

        let result = ListResult {
            deployments: vec![&d],
            other_namespaces: BTreeMap::new(),
            network_names: BTreeMap::new(),
            fork_deployment_ids: fork_ids,
        };

        let output = build_list_json_output(&result);
        let json = serde_json::to_value(&output).unwrap();
        let entry = &json["deployments"][0];

        assert_eq!(entry["fork"], true, "fork should be true for fork deployments");
    }

    #[test]
    fn json_output_other_namespaces_included_when_empty_deployments() {
        let mut other_ns = BTreeMap::new();
        other_ns.insert("production".into(), 3usize);
        other_ns.insert("staging".into(), 1usize);

        let result = ListResult {
            deployments: vec![],
            other_namespaces: other_ns,
            network_names: BTreeMap::new(),
            fork_deployment_ids: HashSet::new(),
        };

        let output = build_list_json_output(&result);
        let json = serde_json::to_value(&output).unwrap();

        assert!(json["deployments"].as_array().unwrap().is_empty());
        let other = json.get("otherNamespaces").expect("otherNamespaces should be present");
        assert_eq!(other["production"], 3);
        assert_eq!(other["staging"], 1);
    }

    #[test]
    fn json_output_other_namespaces_omitted_when_deployments_exist() {
        let d = make_deployment("ns/1/A", "ns", 1, "A", "", DeploymentType::Singleton, None);
        let mut other_ns = BTreeMap::new();
        other_ns.insert("production".into(), 3usize);

        let result = ListResult {
            deployments: vec![&d],
            other_namespaces: other_ns,
            network_names: BTreeMap::new(),
            fork_deployment_ids: HashSet::new(),
        };

        let output = build_list_json_output(&result);
        let json = serde_json::to_value(&output).unwrap();

        assert!(
            json.get("otherNamespaces").is_none(),
            "otherNamespaces should be omitted when deployments exist"
        );
    }

    #[test]
    fn json_output_other_namespaces_omitted_when_empty() {
        let result = ListResult {
            deployments: vec![],
            other_namespaces: BTreeMap::new(),
            network_names: BTreeMap::new(),
            fork_deployment_ids: HashSet::new(),
        };

        let output = build_list_json_output(&result);
        let json = serde_json::to_value(&output).unwrap();

        assert!(
            json.get("otherNamespaces").is_none(),
            "otherNamespaces should be omitted when empty"
        );
    }
}
