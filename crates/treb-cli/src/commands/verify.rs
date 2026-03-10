//! `treb verify` command implementation.

use std::{collections::HashMap, env, fmt, time::Duration};

use anyhow::{Context, bail};
use chrono::Utc;
use owo_colors::{OwoColorize, Style};
use serde::Serialize;
use treb_core::types::{VerificationStatus, VerifierStatus, contract_display_name};
use treb_registry::Registry;
use treb_verify::VerifyOpts;

use crate::{
    commands::resolve::resolve_deployment,
    output,
    ui::{
        color, emoji,
        interactive::is_non_interactive,
        selector::{fuzzy_select_deployment_id, multiselect_deployments},
    },
};

/// Per-verifier JSON result in the verifiers breakdown.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VerifierResultJson {
    status: String,
    url: String,
    reason: String,
}

/// JSON output for a single verification result.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VerifyOutputJson {
    deployment_id: String,
    contract_name: String,
    address: String,
    chain_id: u64,
    verifier: String,
    status: String,
    explorer_url: String,
    reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    verified_at: Option<String>,
    verifiers: HashMap<String, VerifierResultJson>,
}

#[derive(Debug)]
struct RenderedVerifyFailure;

impl fmt::Display for RenderedVerifyFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("verification failed")
    }
}

impl std::error::Error for RenderedVerifyFailure {}

pub fn is_rendered_verify_failure(err: &anyhow::Error) -> bool {
    err.downcast_ref::<RenderedVerifyFailure>().is_some()
}

/// Resolve the API key for a verifier.
///
/// If an explicit key was provided via `--verifier-api-key`, it takes precedence.
/// Otherwise, checks the standard environment variable for the verifier:
///   - etherscan  -> ETHERSCAN_API_KEY
///   - blockscout -> BLOCKSCOUT_API_KEY
///   - sourcify   -> None (keyless)
fn resolve_api_key(verifier: &str, explicit_key: &Option<String>) -> Option<String> {
    if explicit_key.is_some() {
        return explicit_key.clone();
    }
    match verifier {
        "etherscan" => env::var("ETHERSCAN_API_KEY").ok(),
        "blockscout" => env::var("BLOCKSCOUT_API_KEY").ok(),
        _ => None,
    }
}

/// Compute aggregate verification status from per-verifier results.
///
/// All VERIFIED -> Verified, all FAILED -> Failed, mixed -> Partial.
fn aggregate_status(
    verifier_results: &std::collections::HashMap<String, VerifierStatus>,
) -> VerificationStatus {
    if verifier_results.is_empty() {
        return VerificationStatus::Unverified;
    }
    let all_verified = verifier_results.values().all(|v| v.status == "VERIFIED");
    let all_failed = verifier_results.values().all(|v| v.status == "FAILED");
    if all_verified {
        VerificationStatus::Verified
    } else if all_failed {
        VerificationStatus::Failed
    } else {
        VerificationStatus::Partial
    }
}

/// Convert internal verifier status map to JSON output map.
fn verifier_results_json(
    verifiers: &HashMap<String, VerifierStatus>,
) -> HashMap<String, VerifierResultJson> {
    verifiers
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                VerifierResultJson {
                    status: v.status.clone(),
                    url: v.url.clone(),
                    reason: v.reason.clone(),
                },
            )
        })
        .collect()
}

/// Conditionally apply an owo-colors [`Style`] to text.
///
/// Returns the styled string when color is enabled, plain text otherwise.
fn styled(text: &str, style: Style) -> String {
    if color::is_color_enabled() { format!("{}", text.style(style)) } else { text.to_string() }
}

/// Title-case a verifier name (e.g., "etherscan" → "Etherscan").
fn title_case(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

/// Map a `VerificationStatus` to its Go-matching emoji icon for batch output.
fn get_status_icon(status: &VerificationStatus) -> &'static str {
    match status {
        VerificationStatus::Verified => emoji::REFRESH,
        VerificationStatus::Failed => emoji::WARNING,
        VerificationStatus::Partial => emoji::REPEAT,
        VerificationStatus::Unverified => emoji::HOURGLASS,
    }
}

fn ordered_verifier_names(
    requested_verifiers: Option<&[String]>,
    verifiers: &HashMap<String, VerifierStatus>,
) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();

    if let Some(requested) = requested_verifiers {
        for name in requested {
            let normalized = name.to_lowercase();
            if verifiers.contains_key(&normalized) && !names.contains(&normalized) {
                names.push(normalized);
            }
        }
    }

    let mut remaining: Vec<_> =
        verifiers.keys().filter(|name| !names.contains(*name)).cloned().collect();
    remaining.sort();
    names.extend(remaining);

    names
}

fn print_verifier_status_lines(
    verifiers: &HashMap<String, VerifierStatus>,
    requested_verifiers: Option<&[String]>,
    indent: &str,
) {
    for name in ordered_verifier_names(requested_verifiers, verifiers) {
        let v = &verifiers[&name];
        let title = title_case(&name);
        if v.status == "VERIFIED" {
            eprintln!(
                "{indent}{} {} {}",
                styled(emoji::CHECK_MARK, color::VERIFIED),
                title,
                styled("Verified", color::VERIFIED),
            );
            if !v.url.is_empty() {
                eprintln!("{indent}  {}", styled(&v.url, color::MUTED));
            }
        } else {
            eprintln!(
                "{indent}{} {} {}",
                styled(emoji::CROSS_MARK, color::FAILED),
                title,
                styled("Failed", color::FAILED),
            );
            if !v.reason.is_empty() {
                eprintln!("{indent}  {}", styled(&v.reason, color::MUTED));
            }
        }
    }
}

fn print_batch_result_details(
    status: &VerificationStatus,
    verifiers: &HashMap<String, VerifierStatus>,
    requested_verifiers: &[String],
) {
    if matches!(status, VerificationStatus::Verified | VerificationStatus::Partial) {
        eprintln!(
            "    {} {}",
            styled(emoji::CHECK_MARK, color::SUCCESS),
            styled("Verification completed", color::SUCCESS),
        );
    }

    let mut printed_failure = false;
    for name in ordered_verifier_names(Some(requested_verifiers), verifiers) {
        let verifier = &verifiers[&name];
        if verifier.status != "FAILED" {
            continue;
        }

        printed_failure = true;
        let message = if verifier.reason.is_empty() {
            "Verification failed"
        } else {
            verifier.reason.as_str()
        };
        eprintln!(
            "    {} {}",
            styled(emoji::CROSS_MARK, color::FAILED),
            styled(message, color::FAILED),
        );
    }

    if !printed_failure && *status == VerificationStatus::Failed {
        eprintln!(
            "    {} {}",
            styled(emoji::CROSS_MARK, color::FAILED),
            styled("Verification failed", color::FAILED),
        );
    }
}

/// Print the `Verification Status:` section with per-verifier results.
fn print_verification_status(verifiers: &HashMap<String, VerifierStatus>) {
    if verifiers.is_empty() {
        return;
    }
    eprintln!();
    eprintln!("  Verification Status:");
    print_verifier_status_lines(verifiers, None, "    ");
}

/// Run the verify command.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    deployment: Option<String>,
    all: bool,
    verifiers: &[String],
    verifier_url: Option<String>,
    verifier_api_key: Option<String>,
    force: bool,
    watch: bool,
    retries: u32,
    delay: u64,
    json: bool,
) -> anyhow::Result<()> {
    // Validate all verifier values.
    for v in verifiers {
        match v.as_str() {
            "etherscan" | "sourcify" | "blockscout" => {}
            other => {
                bail!(
                    "unknown verifier '{}': expected one of etherscan, sourcify, blockscout",
                    other
                );
            }
        }
    }

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

    if all {
        return run_batch(
            verifiers,
            verifier_url,
            verifier_api_key,
            force,
            watch,
            retries,
            delay,
            json,
            &cwd,
        )
        .await;
    }

    // --- Single deployment verification ---

    let mut registry = Registry::open(&cwd).context("failed to open registry")?;
    let lookup = registry.load_lookup_index().context("failed to load lookup index")?;

    let query = match deployment {
        Some(q) => q,
        None => {
            let deployments: Vec<_> = registry.list_deployments().into_iter().cloned().collect();
            fuzzy_select_deployment_id(&deployments)
                .map_err(|e| anyhow::anyhow!("{e}"))?
                .ok_or_else(|| anyhow::anyhow!("no deployment selected"))?
        }
    };

    let resolved = resolve_deployment(&query, &registry, &lookup)?;

    // Capture fields we need after the borrow on registry is released.
    let deployment_id = resolved.id.clone();
    let contract_name = resolved.contract_name.clone();
    let label = resolved.label.clone();
    let address = resolved.address.clone();
    let chain_id = resolved.chain_id;
    let already_verified = resolved.verification.status == VerificationStatus::Verified;
    let existing_url = resolved.verification.etherscan_url.clone();
    let existing_verified_at = resolved.verification.verified_at;
    let existing_verifiers = resolved.verification.verifiers.clone();
    let display_name = contract_display_name(&contract_name, &label);

    // Skip if already verified and not forced.
    if already_verified && !force {
        if json {
            let verifier = verifiers.first().map(|s| s.as_str()).unwrap_or("etherscan");
            let out = VerifyOutputJson {
                deployment_id,
                contract_name,
                address,
                chain_id,
                verifier: verifier.to_string(),
                status: "VERIFIED".to_string(),
                explorer_url: existing_url,
                reason: String::new(),
                verified_at: existing_verified_at.map(|t| t.to_rfc3339()),
                verifiers: verifier_results_json(&existing_verifiers),
            };
            output::print_json(&out)?;
        } else {
            eprintln!(
                "{}",
                styled(
                    &format!(
                        "Contract {} is already verified. Use --force to re-verify.",
                        display_name
                    ),
                    color::WARNING,
                ),
            );
        }
        return Ok(());
    }

    // Release the borrow on `resolved` before mutating registry.
    let _ = resolved;

    // --- Multi-verifier loop ---
    let mut dep = registry.get_deployment(&deployment_id).unwrap().clone();
    let verifier_count = verifiers.len();
    let mut attempted_verifiers: HashMap<String, VerifierStatus> = HashMap::new();

    for (vi, verifier) in verifiers.iter().enumerate() {
        let resolved_key = resolve_api_key(verifier, &verifier_api_key);
        let opts = VerifyOpts {
            verifier: verifier.clone(),
            verifier_url: verifier_url.clone(),
            verifier_api_key: resolved_key.clone(),
            etherscan_api_key: resolved_key,
            rpc_url: None,
            force,
            watch,
            retries,
            delay: delay as u32,
            root: cwd.clone(),
        };

        // Re-fetch deployment reference for build_verify_args (needs &Deployment).
        let dep_ref = registry.get_deployment(&deployment_id).unwrap();
        let verify_args = match treb_verify::build_verify_args(dep_ref, &opts) {
            Ok(args) => args,
            Err(e) => {
                let reason = format!("{e:#}");
                let status = VerifierStatus {
                    status: "FAILED".to_string(),
                    url: String::new(),
                    reason: reason.clone(),
                };
                dep.verification.verifiers.insert(verifier.to_lowercase(), status.clone());
                attempted_verifiers.insert(verifier.to_lowercase(), status);
                continue;
            }
        };

        let result = verify_args.run().await;
        let explorer_url = treb_verify::explorer_url(chain_id, &address, verifier);

        match result {
            Ok(()) => {
                // Keep etherscan_url reserved for successful etherscan verification only.
                if verifier == "etherscan" && dep.verification.etherscan_url.is_empty() {
                    if let Some(ref url) = explorer_url {
                        dep.verification.etherscan_url = url.clone();
                    }
                }
                if dep.verification.verified_at.is_none() {
                    dep.verification.verified_at = Some(Utc::now());
                }
                let status = VerifierStatus {
                    status: "VERIFIED".to_string(),
                    url: explorer_url.clone().unwrap_or_default(),
                    reason: String::new(),
                };
                dep.verification.verifiers.insert(verifier.to_lowercase(), status.clone());
                attempted_verifiers.insert(verifier.to_lowercase(), status);
            }
            Err(e) => {
                let reason = format!("{e:#}");
                let status = VerifierStatus {
                    status: "FAILED".to_string(),
                    url: String::new(),
                    reason: reason.clone(),
                };
                dep.verification.verifiers.insert(verifier.to_lowercase(), status.clone());
                attempted_verifiers.insert(verifier.to_lowercase(), status);
            }
        }

        // Rate limiting delay between verifier attempts (skip after the last one).
        if vi + 1 < verifier_count && delay > 0 {
            tokio::time::sleep(Duration::from_secs(delay)).await;
        }
    }

    // Compute aggregate status and update registry once.
    let agg_status = aggregate_status(&attempted_verifiers);
    dep.verification.status = agg_status.clone();
    // verified_at already set on first successful verification above.
    // Set reason from first failed verifier, if any.
    dep.verification.reason = attempted_verifiers
        .values()
        .find(|v| v.status == "FAILED")
        .map(|v| v.reason.clone())
        .unwrap_or_default();

    registry.update_deployment(dep.clone())?;

    // Output.
    if json {
        let agg_status_str = match dep.verification.status {
            VerificationStatus::Verified => "VERIFIED",
            VerificationStatus::Failed => "FAILED",
            VerificationStatus::Partial => "PARTIAL",
            VerificationStatus::Unverified => "UNVERIFIED",
        };
        let verifier_label = verifiers.first().map(|s| s.as_str()).unwrap_or("etherscan");
        let out = VerifyOutputJson {
            deployment_id,
            contract_name,
            address,
            chain_id,
            verifier: verifier_label.to_string(),
            status: agg_status_str.to_string(),
            explorer_url: dep.verification.etherscan_url,
            reason: dep.verification.reason,
            verified_at: dep.verification.verified_at.map(|t| t.to_rfc3339()),
            verifiers: verifier_results_json(&attempted_verifiers),
        };
        output::print_json(&out)?;
    } else {
        // Print success/failure message and verification status section.
        match dep.verification.status {
            VerificationStatus::Verified => {
                eprintln!(
                    "{} {}",
                    styled(emoji::CHECK_MARK, color::SUCCESS),
                    styled("Verification completed successfully!", color::SUCCESS),
                );
            }
            VerificationStatus::Failed => {
                // Print per-error failure messages.
                for v in attempted_verifiers.values() {
                    if v.status == "FAILED" {
                        eprintln!(
                            "{} {}",
                            styled(emoji::CROSS_MARK, color::FAILED),
                            styled(&format!("Verification failed: {}", v.reason), color::FAILED,),
                        );
                    }
                }
            }
            _ => {}
        }

        // Show verification status section.
        print_verification_status(&attempted_verifiers);

        if dep.verification.status == VerificationStatus::Failed {
            return Err(RenderedVerifyFailure.into());
        }
    }

    Ok(())
}

/// Batch verification: verify all unverified (or all with --force) deployments.
#[allow(clippy::too_many_arguments)]
async fn run_batch(
    verifiers: &[String],
    verifier_url: Option<String>,
    verifier_api_key: Option<String>,
    force: bool,
    watch: bool,
    retries: u32,
    delay: u64,
    json: bool,
    cwd: &std::path::Path,
) -> anyhow::Result<()> {
    let mut registry = Registry::open(cwd).context("failed to open registry")?;

    let all_deployments: Vec<_> = registry.list_deployments().into_iter().cloned().collect();
    let candidate_deployments: Vec<_> = if force {
        all_deployments
    } else {
        all_deployments
            .into_iter()
            .filter(|d| d.verification.status != VerificationStatus::Verified)
            .collect()
    };

    if candidate_deployments.is_empty() {
        if json {
            output::print_json(&Vec::<VerifyOutputJson>::new())?;
        } else if force {
            eprintln!("{}", styled("No deployed contracts found to verify.", color::WARNING),);
        } else {
            eprintln!(
                "{}",
                styled(
                    "No unverified deployed contracts found. Use --force to re-verify all contracts.",
                    color::WARNING,
                ),
            );
        }
        return Ok(());
    }

    // Interactive selection is only available when the shared non-interactive
    // mode checks say prompts are safe to render; otherwise verify all
    // candidates.
    let candidate_ids: Vec<String> = if is_non_interactive(false) {
        candidate_deployments.iter().map(|d| d.id.clone()).collect()
    } else {
        multiselect_deployments(&candidate_deployments, "Select deployments to verify")
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .into_iter()
            .map(|d| d.id.clone())
            .collect()
    };

    if candidate_ids.is_empty() {
        if json {
            output::print_json(&Vec::<VerifyOutputJson>::new())?;
        } else if force {
            eprintln!("{}", styled("No deployed contracts found to verify.", color::WARNING),);
        } else {
            eprintln!(
                "{}",
                styled(
                    "No unverified deployed contracts found. Use --force to re-verify all contracts.",
                    color::WARNING,
                ),
            );
        }
        return Ok(());
    }

    let total = candidate_ids.len();
    let mut results: Vec<VerifyOutputJson> = Vec::new();
    let mut success_count: usize = 0;

    if !json {
        if force {
            eprintln!(
                "{}",
                styled(
                    &format!(
                        "Found {} deployed contracts to verify (including verified ones with --force):",
                        total,
                    ),
                    color::STAGE,
                ),
            );
        } else {
            eprintln!(
                "{}",
                styled(
                    &format!("Found {} unverified deployed contracts to verify:", total),
                    color::STAGE,
                ),
            );
        }
    }

    for (i, dep_id) in candidate_ids.iter().enumerate() {
        let dep_snapshot = registry.get_deployment(dep_id).unwrap().clone();
        let contract_name = dep_snapshot.contract_name.clone();
        let display_name = contract_display_name(&dep_snapshot.contract_name, &dep_snapshot.label);
        let address = dep_snapshot.address.clone();
        let chain_id = dep_snapshot.chain_id;
        let namespace = dep_snapshot.namespace.clone();

        let mut dep_owned = dep_snapshot;
        let verifier_count = verifiers.len();
        let mut attempted_verifiers: HashMap<String, VerifierStatus> = HashMap::new();

        for (vi, verifier) in verifiers.iter().enumerate() {
            let resolved_key = resolve_api_key(verifier, &verifier_api_key);
            let opts = VerifyOpts {
                verifier: verifier.clone(),
                verifier_url: verifier_url.clone(),
                verifier_api_key: resolved_key.clone(),
                etherscan_api_key: resolved_key,
                rpc_url: None,
                force,
                watch,
                retries,
                delay: delay as u32,
                root: cwd.to_path_buf(),
            };

            let dep_ref = registry.get_deployment(dep_id).unwrap();
            let verify_args = match treb_verify::build_verify_args(dep_ref, &opts) {
                Ok(args) => args,
                Err(e) => {
                    let reason = format!("{e:#}");
                    let status = VerifierStatus {
                        status: "FAILED".to_string(),
                        url: String::new(),
                        reason: reason.clone(),
                    };
                    dep_owned
                        .verification
                        .verifiers
                        .insert(verifier.to_lowercase(), status.clone());
                    attempted_verifiers.insert(verifier.to_lowercase(), status);
                    continue;
                }
            };

            let result = verify_args.run().await;
            let explorer_url = treb_verify::explorer_url(chain_id, &address, verifier);

            match result {
                Ok(()) => {
                    // Keep etherscan_url reserved for successful etherscan verification only.
                    if verifier == "etherscan" && dep_owned.verification.etherscan_url.is_empty() {
                        if let Some(ref url) = explorer_url {
                            dep_owned.verification.etherscan_url = url.clone();
                        }
                    }
                    if dep_owned.verification.verified_at.is_none() {
                        dep_owned.verification.verified_at = Some(Utc::now());
                    }
                    let status = VerifierStatus {
                        status: "VERIFIED".to_string(),
                        url: explorer_url.clone().unwrap_or_default(),
                        reason: String::new(),
                    };
                    dep_owned
                        .verification
                        .verifiers
                        .insert(verifier.to_lowercase(), status.clone());
                    attempted_verifiers.insert(verifier.to_lowercase(), status);
                }
                Err(e) => {
                    let reason = format!("{e:#}");
                    let status = VerifierStatus {
                        status: "FAILED".to_string(),
                        url: String::new(),
                        reason: reason.clone(),
                    };
                    dep_owned
                        .verification
                        .verifiers
                        .insert(verifier.to_lowercase(), status.clone());
                    attempted_verifiers.insert(verifier.to_lowercase(), status);
                }
            }

            // Rate limiting delay between verifier attempts (skip after the last one).
            if vi + 1 < verifier_count && delay > 0 {
                tokio::time::sleep(Duration::from_secs(delay)).await;
            }
        }

        // Compute aggregate status and update registry once per deployment.
        let agg_status = aggregate_status(&attempted_verifiers);
        dep_owned.verification.status = agg_status.clone();
        // verified_at already set on first successful verification above.
        dep_owned.verification.reason = attempted_verifiers
            .values()
            .find(|v| v.status == "FAILED")
            .map(|v| v.reason.clone())
            .unwrap_or_default();

        registry.update_deployment(dep_owned.clone())?;

        // Print per-result output in Go-matching format.
        if !json {
            let location = format!("chain:{}/{}/{}", chain_id, namespace, display_name);
            let icon = get_status_icon(&agg_status);
            eprintln!("  {} {}", icon, location);
            print_batch_result_details(&agg_status, &attempted_verifiers, verifiers);

            // Blank line between results, not after last.
            if i + 1 < total {
                eprintln!();
            }
        }

        if agg_status == VerificationStatus::Verified {
            success_count += 1;
        }

        let agg_status_str = match dep_owned.verification.status {
            VerificationStatus::Verified => "VERIFIED",
            VerificationStatus::Failed => "FAILED",
            VerificationStatus::Partial => "PARTIAL",
            VerificationStatus::Unverified => "UNVERIFIED",
        };

        results.push(VerifyOutputJson {
            deployment_id: dep_id.clone(),
            contract_name,
            address,
            chain_id,
            verifier: verifiers.first().map(|s| s.as_str()).unwrap_or("etherscan").to_string(),
            status: agg_status_str.to_string(),
            explorer_url: dep_owned.verification.etherscan_url.clone(),
            reason: dep_owned.verification.reason.clone(),
            verified_at: dep_owned.verification.verified_at.map(|t| t.to_rfc3339()),
            verifiers: verifier_results_json(&attempted_verifiers),
        });

        // Rate limiting delay between deployments (skip after the last one).
        if i + 1 < total && delay > 0 {
            tokio::time::sleep(Duration::from_secs(delay)).await;
        }
    }

    // Output results.
    if json {
        output::print_json(&results)?;
    } else {
        eprintln!("\nVerification complete: {}/{} successful", success_count, total);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Mutex, MutexGuard, OnceLock},
    };

    use treb_core::types::{VerificationStatus, VerifierStatus};

    use crate::ui::emoji;

    use super::{aggregate_status, get_status_icon, resolve_api_key};

    fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().expect("env test lock poisoned")
    }

    struct EnvVarGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let guard = Self::capture(key);
            // SAFETY: Serialized by env_lock(), so no concurrent env mutation in these tests.
            unsafe { std::env::set_var(key, value) };
            guard
        }

        fn unset(key: &'static str) -> Self {
            let guard = Self::capture(key);
            // SAFETY: Serialized by env_lock(), so no concurrent env mutation in these tests.
            unsafe { std::env::remove_var(key) };
            guard
        }

        fn capture(key: &'static str) -> Self {
            Self { key, original: std::env::var(key).ok() }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => {
                    // SAFETY: Serialized by env_lock(), so no concurrent env mutation in these
                    // tests.
                    unsafe { std::env::set_var(self.key, value) };
                }
                None => {
                    // SAFETY: Serialized by env_lock(), so no concurrent env mutation in these
                    // tests.
                    unsafe { std::env::remove_var(self.key) };
                }
            }
        }
    }

    #[test]
    fn aggregate_status_all_verified() {
        let mut verifiers = HashMap::new();
        verifiers.insert(
            "etherscan".to_string(),
            VerifierStatus {
                status: "VERIFIED".to_string(),
                url: String::new(),
                reason: String::new(),
            },
        );
        verifiers.insert(
            "sourcify".to_string(),
            VerifierStatus {
                status: "VERIFIED".to_string(),
                url: String::new(),
                reason: String::new(),
            },
        );
        verifiers.insert(
            "blockscout".to_string(),
            VerifierStatus {
                status: "VERIFIED".to_string(),
                url: String::new(),
                reason: String::new(),
            },
        );
        assert_eq!(aggregate_status(&verifiers), VerificationStatus::Verified);
    }

    #[test]
    fn aggregate_status_all_failed() {
        let mut verifiers = HashMap::new();
        verifiers.insert(
            "etherscan".to_string(),
            VerifierStatus {
                status: "FAILED".to_string(),
                url: String::new(),
                reason: "timeout".to_string(),
            },
        );
        verifiers.insert(
            "sourcify".to_string(),
            VerifierStatus {
                status: "FAILED".to_string(),
                url: String::new(),
                reason: "not found".to_string(),
            },
        );
        assert_eq!(aggregate_status(&verifiers), VerificationStatus::Failed);
    }

    #[test]
    fn aggregate_status_mixed_returns_partial() {
        let mut verifiers = HashMap::new();
        verifiers.insert(
            "etherscan".to_string(),
            VerifierStatus {
                status: "VERIFIED".to_string(),
                url: String::new(),
                reason: String::new(),
            },
        );
        verifiers.insert(
            "sourcify".to_string(),
            VerifierStatus {
                status: "FAILED".to_string(),
                url: String::new(),
                reason: "error".to_string(),
            },
        );
        assert_eq!(aggregate_status(&verifiers), VerificationStatus::Partial);
    }

    #[test]
    fn aggregate_status_empty_returns_unverified() {
        let verifiers = HashMap::new();
        assert_eq!(aggregate_status(&verifiers), VerificationStatus::Unverified);
    }

    #[test]
    fn aggregate_status_single_verified() {
        let mut verifiers = HashMap::new();
        verifiers.insert(
            "etherscan".to_string(),
            VerifierStatus {
                status: "VERIFIED".to_string(),
                url: String::new(),
                reason: String::new(),
            },
        );
        assert_eq!(aggregate_status(&verifiers), VerificationStatus::Verified);
    }

    #[test]
    fn aggregate_status_single_failed() {
        let mut verifiers = HashMap::new();
        verifiers.insert(
            "etherscan".to_string(),
            VerifierStatus {
                status: "FAILED".to_string(),
                url: String::new(),
                reason: "err".to_string(),
            },
        );
        assert_eq!(aggregate_status(&verifiers), VerificationStatus::Failed);
    }

    // --- resolve_api_key tests ---

    #[test]
    fn resolve_api_key_explicit_overrides_env_for_etherscan() {
        let _lock = env_lock();
        let _guard = EnvVarGuard::set("ETHERSCAN_API_KEY", "env-key");
        let explicit = Some("explicit-key".to_string());
        let result = resolve_api_key("etherscan", &explicit);
        assert_eq!(result, Some("explicit-key".to_string()));
    }

    #[test]
    fn resolve_api_key_explicit_overrides_env_for_blockscout() {
        let _lock = env_lock();
        let _guard = EnvVarGuard::set("BLOCKSCOUT_API_KEY", "env-key");
        let explicit = Some("explicit-key".to_string());
        let result = resolve_api_key("blockscout", &explicit);
        assert_eq!(result, Some("explicit-key".to_string()));
    }

    #[test]
    fn resolve_api_key_explicit_overrides_for_sourcify() {
        let explicit = Some("explicit-key".to_string());
        let result = resolve_api_key("sourcify", &explicit);
        assert_eq!(result, Some("explicit-key".to_string()));
    }

    #[test]
    fn resolve_api_key_etherscan_from_env() {
        let _lock = env_lock();
        let _guard = EnvVarGuard::set("ETHERSCAN_API_KEY", "eth-env-key");
        let result = resolve_api_key("etherscan", &None);
        assert_eq!(result, Some("eth-env-key".to_string()));
    }

    #[test]
    fn resolve_api_key_blockscout_from_env() {
        let _lock = env_lock();
        let _guard = EnvVarGuard::set("BLOCKSCOUT_API_KEY", "block-env-key");
        let result = resolve_api_key("blockscout", &None);
        assert_eq!(result, Some("block-env-key".to_string()));
    }

    #[test]
    fn resolve_api_key_sourcify_requires_no_key() {
        let result = resolve_api_key("sourcify", &None);
        assert_eq!(result, None);
    }

    #[test]
    fn resolve_api_key_etherscan_no_env_returns_none() {
        let _lock = env_lock();
        let _guard = EnvVarGuard::unset("ETHERSCAN_API_KEY");
        let result = resolve_api_key("etherscan", &None);
        assert_eq!(result, None);
    }

    #[test]
    fn resolve_api_key_blockscout_no_env_returns_none() {
        let _lock = env_lock();
        let _guard = EnvVarGuard::unset("BLOCKSCOUT_API_KEY");
        let result = resolve_api_key("blockscout", &None);
        assert_eq!(result, None);
    }

    #[test]
    fn resolve_api_key_unknown_verifier_returns_none() {
        let result = resolve_api_key("unknown", &None);
        assert_eq!(result, None);
    }

    // --- get_status_icon tests ---

    #[test]
    fn status_icon_verified_is_refresh() {
        assert_eq!(get_status_icon(&VerificationStatus::Verified), emoji::REFRESH);
    }

    #[test]
    fn status_icon_failed_is_warning() {
        assert_eq!(get_status_icon(&VerificationStatus::Failed), emoji::WARNING);
    }

    #[test]
    fn status_icon_partial_is_repeat() {
        assert_eq!(get_status_icon(&VerificationStatus::Partial), emoji::REPEAT);
    }

    #[test]
    fn status_icon_unverified_is_hourglass() {
        assert_eq!(get_status_icon(&VerificationStatus::Unverified), emoji::HOURGLASS);
    }
}
