//! `treb show` command implementation.

use std::{collections::HashMap, env, fmt::Write as _};

use anyhow::{Context, bail};
use owo_colors::{OwoColorize, Style};
use treb_core::types::{Deployment, VerificationStatus, VerifierStatus, contract_display_name};
use treb_registry::Registry;

use crate::{
    commands::{
        list::{DeploymentFilters, filter_deployments},
        resolve::resolve_deployment_in_scope,
    },
    output,
    ui::{badge, color, selector::fuzzy_select_deployment_id},
};

/// Resolved proxy implementation metadata keyed by `(namespace, chain_id, address)`.
struct ResolvedImplementation {
    deployment_id: String,
    display_name: String,
}

/// Lookup table for resolving proxy implementation addresses to deployment metadata.
type ImplLookup = HashMap<(String, u64, String), ResolvedImplementation>;

struct VerificationDisplay<'a> {
    status: VerificationStatus,
    etherscan_url: &'a str,
    verified_at: Option<&'a chrono::DateTime<chrono::Utc>>,
}

pub async fn run(
    deployment_query: Option<String>,
    namespace: Option<String>,
    network: Option<String>,
    no_fork: bool,
    json: bool,
    non_interactive: bool,
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

    let scope = super::resolve_command_scope(&cwd, namespace, network)?;

    let registry = Registry::open(&cwd).context("failed to open registry")?;
    let lookup = registry.load_lookup_index().context("failed to load lookup index")?;
    let all_deployments = registry.list_deployments();
    let impl_lookup = build_impl_lookup(&all_deployments);
    let scope = ShowScope { namespace: scope.namespace, network: scope.network, no_fork };
    let mut filters = scope.as_deployment_filters();
    filters.resolved_chain_id =
        super::resolve_chain_id_for_network(&cwd, scope.network.as_deref()).await?;
    let filtered_deployments = filter_deployments(&all_deployments, &filters);

    let query = match deployment_query {
        Some(q) => q,
        None => {
            if filtered_deployments.is_empty() {
                bail!("{}", scope.no_deployments_message());
            }

            let deployments: Vec<_> =
                filtered_deployments.iter().map(|deployment| (*deployment).clone()).collect();
            fuzzy_select_deployment_id(&deployments, non_interactive)
                .map_err(|e| anyhow::anyhow!("{e}"))?
                .ok_or_else(|| anyhow::anyhow!("no deployment selected"))?
        }
    };

    let deployment = normalize_deployment_for_show(
        resolve_deployment_in_scope(&query, &registry, &lookup, &filtered_deployments)
            .map_err(|err| scope.decorate_resolution_error(&query, err))?
            .clone(),
    );

    if json {
        let is_fork = deployment.namespace.starts_with("fork/");
        let mut wrapper = serde_json::json!({
            "deployment": deployment,
        });
        if is_fork {
            wrapper["fork"] = serde_json::json!(true);
        }
        output::print_json(&wrapper)?;
    } else {
        print_deployment_details(&deployment, &impl_lookup);
    }

    Ok(())
}

#[derive(Clone, Debug, Default)]
struct ShowScope {
    namespace: Option<String>,
    network: Option<String>,
    no_fork: bool,
}

impl ShowScope {
    fn as_deployment_filters(&self) -> DeploymentFilters {
        DeploymentFilters {
            network: self.network.clone(),
            resolved_chain_id: None,
            namespace: self.namespace.clone(),
            deployment_type: None,
            tag: None,
            contract: None,
            label: None,
            fork: false,
            no_fork: self.no_fork,
        }
    }

    fn context_suffix(&self) -> String {
        let mut context = String::new();

        if let Some(namespace) = &self.namespace {
            let _ = write!(context, " in namespace '{namespace}'");
        }

        if let Some(network) = &self.network {
            let _ = write!(context, " on network '{network}'");
        }

        if self.no_fork {
            context.push_str(" excluding fork deployments");
        }

        context
    }

    fn no_deployments_message(&self) -> String {
        format!(
            "no deployments found{}\n\nRun `treb list` to see available deployments.",
            self.context_suffix()
        )
    }

    fn decorate_resolution_error(&self, query: &str, err: anyhow::Error) -> anyhow::Error {
        if self.context_suffix().is_empty() {
            return err;
        }

        let message = err.to_string();
        if message.starts_with("no deployment found") {
            anyhow::anyhow!(
                "no deployment found matching '{query}'{}\n\nRun `treb list` to see available deployments.",
                self.context_suffix()
            )
        } else {
            err
        }
    }
}

/// Conditionally apply an owo-colors [`Style`] to text.
///
/// Returns the styled string when color is enabled, plain text otherwise.
fn styled(text: &str, style: Style) -> String {
    if color::is_color_enabled() { format!("{}", text.style(style)) } else { text.to_string() }
}

/// Print the deployment header: `Deployment: {id}` in cyan bold + 80-char `=` divider.
fn print_deployment_header(id: &str, fork_badge: Option<&str>) {
    let header = match fork_badge {
        Some(badge) => format!("Deployment: {} {}", id, styled(badge, color::FORK_BADGE)),
        None => format!("Deployment: {}", id),
    };
    println!("{}", styled(&header, color::STAGE));
    println!("{}", "=".repeat(80));
}

/// Print a plain-text section header: `\nSection Name:\n`.
fn print_section(title: &str) {
    println!("\n{}:", title);
}

/// Print a 2-space-indented key-value field: `  Key: Value`.
fn print_field(key: &str, value: &str) {
    println!("  {}: {}", key, value);
}

fn build_impl_lookup(deployments: &[&Deployment]) -> ImplLookup {
    deployments
        .iter()
        .map(|d| {
            let display_name = contract_display_name(&d.contract_name, &d.label);
            (
                (d.namespace.to_lowercase(), d.chain_id, d.address.to_lowercase()),
                ResolvedImplementation { deployment_id: d.id.clone(), display_name },
            )
        })
        .collect()
}

fn resolve_proxy_implementation<'a>(
    namespace: &str,
    chain_id: u64,
    implementation: &str,
    impl_lookup: &'a ImplLookup,
) -> Option<&'a ResolvedImplementation> {
    let key = (namespace.to_lowercase(), chain_id, implementation.to_lowercase());

    impl_lookup.get(&key)
}

fn aggregate_verification_status(
    verifiers: &HashMap<String, VerifierStatus>,
) -> VerificationStatus {
    if verifiers.is_empty() {
        return VerificationStatus::Unverified;
    }

    let all_verified =
        verifiers.values().all(|verifier| verifier.status.eq_ignore_ascii_case("VERIFIED"));
    let all_failed =
        verifiers.values().all(|verifier| verifier.status.eq_ignore_ascii_case("FAILED"));

    if all_verified {
        VerificationStatus::Verified
    } else if all_failed {
        VerificationStatus::Failed
    } else {
        VerificationStatus::Partial
    }
}

fn verification_display(d: &Deployment) -> VerificationDisplay<'_> {
    let status = if d.verification.verifiers.is_empty() {
        d.verification.status.clone()
    } else {
        aggregate_verification_status(&d.verification.verifiers)
    };

    let etherscan_url = d
        .verification
        .verifiers
        .iter()
        .find(|(name, verifier)| name.eq_ignore_ascii_case("etherscan") && !verifier.url.is_empty())
        .map(|(_, verifier)| verifier.url.as_str())
        .filter(|_| d.verification.etherscan_url.is_empty())
        .unwrap_or(&d.verification.etherscan_url);

    VerificationDisplay { status, etherscan_url, verified_at: d.verification.verified_at.as_ref() }
}

fn normalize_deployment_for_show(mut deployment: Deployment) -> Deployment {
    let (status, etherscan_url, verified_at) = {
        let verification = verification_display(&deployment);
        (
            verification.status,
            verification.etherscan_url.to_string(),
            verification.verified_at.cloned(),
        )
    };

    deployment.verification.status = status;
    deployment.verification.etherscan_url = etherscan_url;
    deployment.verification.verified_at = verified_at;

    deployment
}

fn print_deployment_details(d: &Deployment, impl_lookup: &ImplLookup) {
    // Header: Deployment: {id} [fork]
    let fork = badge::fork_badge(&d.namespace);
    print_deployment_header(&d.id, fork.as_deref());

    // Basic Information (Go: Identity + On-Chain merged)
    print_section("Basic Information");
    let display_name = contract_display_name(&d.contract_name, &d.label);
    let contract_styled = styled(&display_name, color::YELLOW);
    print_field("Contract", &contract_styled);
    print_field("Address", &d.address);
    let type_str = d.deployment_type.to_string();
    print_field("Type", &type_str);
    print_field("Namespace", &d.namespace);
    print_field("Network", &d.chain_id.to_string());
    if !d.label.is_empty() {
        let label_styled = styled(&d.label, Style::new().magenta());
        print_field("Label", &label_styled);
    }

    // Deployment Strategy (Go: Transaction)
    print_section("Deployment Strategy");
    print_field("Method", &d.deployment_strategy.method.to_string());
    if !d.deployment_strategy.factory.is_empty() {
        print_field("Factory", &d.deployment_strategy.factory);
    }
    let zero_hash = "0x0000000000000000000000000000000000000000000000000000000000000000";
    if !d.deployment_strategy.salt.is_empty() && d.deployment_strategy.salt != zero_hash {
        print_field("Salt", &d.deployment_strategy.salt);
    }
    if !d.deployment_strategy.entropy.is_empty() {
        print_field("Entropy", &d.deployment_strategy.entropy);
    }
    if !d.deployment_strategy.init_code_hash.is_empty() {
        print_field("InitCodeHash", &d.deployment_strategy.init_code_hash);
    }

    // Proxy Information (only for proxy deployments)
    if let Some(ref proxy) = d.proxy_info {
        print_section("Proxy Information");
        print_field("Type", &proxy.proxy_type);
        let resolved_implementation = resolve_proxy_implementation(
            &d.namespace,
            d.chain_id,
            &proxy.implementation,
            impl_lookup,
        );
        let implementation = match resolved_implementation {
            Some(resolved) => {
                let styled_name = styled(&resolved.display_name, Style::new().yellow().bold());
                format!("{styled_name} at {}", proxy.implementation)
            }
            None => proxy.implementation.clone(),
        };
        print_field("Implementation", &implementation);
        if let Some(resolved) = resolved_implementation {
            print_field("Implementation ID", &styled(&resolved.deployment_id, color::CYAN));
        }
        if !proxy.admin.is_empty() {
            print_field("Admin", &proxy.admin);
        }
        if !proxy.history.is_empty() {
            println!("  Upgrade History:");
            for (i, upgrade) in proxy.history.iter().enumerate() {
                println!(
                    "    {}. {} (upgraded at {})",
                    i + 1,
                    upgrade.implementation_id,
                    upgrade.upgraded_at.format("%Y-%m-%d %H:%M:%S"),
                );
            }
        }
    }

    // Artifact Information (Go: Artifact)
    print_section("Artifact Information");
    print_field("Path", &d.artifact.path);
    print_field("Compiler", &d.artifact.compiler_version);
    if !d.artifact.bytecode_hash.is_empty() {
        print_field("BytecodeHash", &d.artifact.bytecode_hash);
    }
    if !d.artifact.script_path.is_empty() {
        print_field("Script", &d.artifact.script_path);
    }
    if !d.artifact.git_commit.is_empty() {
        print_field("GitCommit", &d.artifact.git_commit);
    }

    // Verification Status (Go: Verification)
    print_section("Verification Status");
    let verification = verification_display(d);
    let status_str = verification.status.to_string();
    let status_style = match verification.status {
        VerificationStatus::Verified => color::VERIFIED,
        _ => color::NOT_VERIFIED,
    };
    print_field("Status", &styled(&status_str, status_style));
    if !verification.etherscan_url.is_empty() {
        print_field("Etherscan", verification.etherscan_url);
    }
    if let Some(verified_at) = verification.verified_at {
        print_field("Verified At", &verified_at.format("%Y-%m-%d %H:%M:%S").to_string());
    }

    // Tags (only when present)
    if let Some(ref tags) = d.tags {
        if !tags.is_empty() {
            print_section("Tags");
            for tag in tags {
                println!("  - {}", tag);
            }
        }
    }

    // Timestamps
    print_section("Timestamps");
    print_field("Created", &d.created_at.format("%Y-%m-%d %H:%M:%S").to_string());
    print_field("Updated", &d.updated_at.format("%Y-%m-%d %H:%M:%S").to_string());
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use treb_core::types::{
        ArtifactInfo, DeploymentMethod, DeploymentStrategy, DeploymentType, VerificationInfo,
    };

    fn sample_deployment() -> Deployment {
        Deployment {
            id: "mainnet/1/Counter:v1".into(),
            namespace: "mainnet".into(),
            chain_id: 1,
            contract_name: "Counter".into(),
            label: "v1".into(),
            address: "0x1234567890abcdef1234567890abcdef12345678".into(),
            deployment_type: DeploymentType::Singleton,
            execution: None,
            transaction_id: String::new(),
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
                bytecode_hash: String::new(),
                script_path: String::new(),
                git_commit: String::new(),
            },
            verification: VerificationInfo {
                status: VerificationStatus::Unverified,
                etherscan_url: String::new(),
                verified_at: None,
                reason: String::new(),
                verifiers: HashMap::new(),
            },
            tags: None,
            created_at: chrono::Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap(),
            updated_at: chrono::Utc.with_ymd_and_hms(2026, 3, 2, 12, 0, 0).unwrap(),
        }
    }

    #[test]
    fn verification_display_derives_stale_aggregate_fields_from_verifiers() {
        let mut deployment = sample_deployment();
        deployment.verification.verifiers.insert(
            "etherscan".into(),
            VerifierStatus {
                status: "VERIFIED".into(),
                url: "https://etherscan.io/address/0x123".into(),
                reason: String::new(),
            },
        );
        deployment.verification.verifiers.insert(
            "sourcify".into(),
            VerifierStatus {
                status: "FAILED".into(),
                url: String::new(),
                reason: "not found".into(),
            },
        );

        let display = verification_display(&deployment);

        assert_eq!(display.status, VerificationStatus::Partial);
        assert_eq!(display.etherscan_url, "https://etherscan.io/address/0x123");
        assert!(display.verified_at.is_none());
    }

    #[test]
    fn verification_display_keeps_top_level_fields_without_verifiers() {
        let mut deployment = sample_deployment();
        let verified_at = chrono::Utc.with_ymd_and_hms(2026, 3, 3, 12, 0, 0).unwrap();
        deployment.verification.status = VerificationStatus::Verified;
        deployment.verification.etherscan_url = "https://etherscan.io/address/0xabc".into();
        deployment.verification.verified_at = Some(verified_at);

        let display = verification_display(&deployment);

        assert_eq!(display.status, VerificationStatus::Verified);
        assert_eq!(display.etherscan_url, "https://etherscan.io/address/0xabc");
        assert_eq!(display.verified_at, Some(&verified_at));
    }

    #[test]
    fn normalize_deployment_for_show_updates_stale_verification_fields() {
        let mut deployment = sample_deployment();
        deployment.verification.verifiers.insert(
            "etherscan".into(),
            VerifierStatus {
                status: "VERIFIED".into(),
                url: "https://etherscan.io/address/0x123".into(),
                reason: String::new(),
            },
        );
        deployment.verification.verifiers.insert(
            "sourcify".into(),
            VerifierStatus {
                status: "FAILED".into(),
                url: String::new(),
                reason: "not found".into(),
            },
        );

        let normalized = normalize_deployment_for_show(deployment);

        assert_eq!(normalized.verification.status, VerificationStatus::Partial);
        assert_eq!(normalized.verification.etherscan_url, "https://etherscan.io/address/0x123");
        assert!(normalized.verification.verified_at.is_none());
    }

    #[test]
    fn show_scope_context_suffix_orders_filters() {
        let scope = ShowScope {
            namespace: Some("mainnet".into()),
            network: Some("42220".into()),
            no_fork: true,
        };

        assert_eq!(
            scope.context_suffix(),
            " in namespace 'mainnet' on network '42220' excluding fork deployments"
        );
    }

    #[test]
    fn show_scope_rewrites_no_match_errors_with_filter_context() {
        let scope = ShowScope {
            namespace: Some("mainnet".into()),
            network: Some("42220".into()),
            no_fork: false,
        };

        let error = scope.decorate_resolution_error(
            "Counter",
            anyhow::anyhow!("no deployment found matching 'Counter'\n\nRun `treb list` to see available deployments."),
        );

        assert_eq!(
            error.to_string(),
            "no deployment found matching 'Counter' in namespace 'mainnet' on network '42220'\n\nRun `treb list` to see available deployments."
        );
    }
}
