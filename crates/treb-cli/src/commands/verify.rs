//! `treb verify` command implementation.

use std::{env, time::Duration};

use anyhow::{Context, bail};
use chrono::Utc;
use console::Term;
use owo_colors::{OwoColorize, Style};
use serde::Serialize;
use treb_core::types::{VerificationStatus, VerifierStatus};
use treb_registry::Registry;
use treb_verify::VerifyOpts;

use crate::{
    commands::resolve::resolve_deployment,
    output,
    ui::badge,
    ui::color,
    ui::selector::{fuzzy_select_deployment_id, multiselect_deployments},
};

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
    verified_at: Option<String>,
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
fn aggregate_status(verifier_results: &std::collections::HashMap<String, VerifierStatus>) -> VerificationStatus {
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

/// Conditionally apply an owo-colors [`Style`] to text.
///
/// Returns the styled string when color is enabled, plain text otherwise.
fn styled(text: &str, style: Style) -> String {
    if color::is_color_enabled() {
        format!("{}", text.style(style))
    } else {
        text.to_string()
    }
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
    let address = resolved.address.clone();
    let chain_id = resolved.chain_id;
    let already_verified = resolved.verification.status == VerificationStatus::Verified;
    let existing_url = resolved.verification.etherscan_url.clone();
    let existing_verified_at = resolved.verification.verified_at;

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
            };
            output::print_json(&out)?;
        } else {
            eprintln!(
                "  {} {} is already verified — use --force to re-verify",
                styled("\u{2713}", color::SUCCESS),
                contract_name,
            );
        }
        return Ok(());
    }

    // Release the borrow on `resolved` before mutating registry.
    let _ = resolved;

    // --- Multi-verifier loop ---
    if !json {
        output::print_stage(
            "\u{1f50d}",
            &format!("Verifying {} ({})", contract_name, output::truncate_address(&address)),
        );
    }

    let mut dep = registry.get_deployment(&deployment_id).unwrap().clone();
    let verifier_count = verifiers.len();

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
                dep.verification.verifiers.insert(
                    verifier.to_lowercase(),
                    VerifierStatus {
                        status: "FAILED".to_string(),
                        url: String::new(),
                        reason: reason.clone(),
                    },
                );
                if !json {
                    eprintln!(
                        "  {}: {}",
                        verifier,
                        styled("FAILED", color::FAILED),
                    );
                    eprintln!("    {}", styled(&reason, color::MUTED));
                }
                continue;
            }
        };

        let result = verify_args.run().await;
        let explorer_url = treb_verify::explorer_url(chain_id, &address, verifier);

        match result {
            Ok(()) => {
                // Set etherscan_url from first successful verification only.
                if dep.verification.etherscan_url.is_empty() {
                    if let Some(ref url) = explorer_url {
                        dep.verification.etherscan_url = url.clone();
                    }
                }
                if dep.verification.verified_at.is_none() {
                    dep.verification.verified_at = Some(Utc::now());
                }
                dep.verification.verifiers.insert(
                    verifier.to_lowercase(),
                    VerifierStatus {
                        status: "VERIFIED".to_string(),
                        url: explorer_url.clone().unwrap_or_default(),
                        reason: String::new(),
                    },
                );
                if !json {
                    eprintln!(
                        "  {}: {}",
                        verifier,
                        styled("VERIFIED", color::VERIFIED),
                    );
                    if let Some(ref url) = explorer_url {
                        eprintln!("    {}", styled(url, color::MUTED));
                    }
                }
            }
            Err(e) => {
                let reason = format!("{e:#}");
                dep.verification.verifiers.insert(
                    verifier.to_lowercase(),
                    VerifierStatus {
                        status: "FAILED".to_string(),
                        url: String::new(),
                        reason: reason.clone(),
                    },
                );
                if !json {
                    eprintln!(
                        "  {}: {}",
                        verifier,
                        styled("FAILED", color::FAILED),
                    );
                    eprintln!("    {}", styled(&reason, color::MUTED));
                }
            }
        }

        // Rate limiting delay between verifier attempts (skip after the last one).
        if vi + 1 < verifier_count && delay > 0 {
            tokio::time::sleep(Duration::from_secs(delay)).await;
        }
    }

    // Compute aggregate status and update registry once.
    let agg_status = aggregate_status(&dep.verification.verifiers);
    dep.verification.status = agg_status.clone();
    // verified_at already set on first successful verification above.
    // Set reason from first failed verifier, if any.
    dep.verification.reason = dep
        .verification
        .verifiers
        .values()
        .find(|v| v.status == "FAILED")
        .map(|v| v.reason.clone())
        .unwrap_or_default();

    registry.update_deployment(dep.clone())?;

    // Show verification badge summary on stdout.
    if !json {
        let ver_badge = if color::is_color_enabled() {
            badge::verification_badge_styled(&dep.verification.verifiers)
        } else {
            badge::verification_badge(&dep.verification.verifiers)
        };
        println!("  {}", ver_badge);
    }

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
        };
        output::print_json(&out)?;
    } else if dep.verification.status == VerificationStatus::Failed {
        bail!(
            "verification failed for {}",
            contract_name
        );
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

    // Collect candidate deployments as owned values to avoid borrow conflicts.
    let candidate_deployments: Vec<_> = registry
        .list_deployments()
        .into_iter()
        .filter(|d| force || d.verification.status != VerificationStatus::Verified)
        .cloned()
        .collect();

    if candidate_deployments.is_empty() {
        if json {
            output::print_json(&Vec::<VerifyOutputJson>::new())?;
        } else {
            eprintln!("No unverified deployments found.");
        }
        return Ok(());
    }

    // In TTY mode, let the user select a subset interactively; non-TTY verifies all.
    let candidate_ids: Vec<String> = if Term::stdout().is_term() {
        multiselect_deployments(&candidate_deployments, "Select deployments to verify")
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .into_iter()
            .map(|d| d.id.clone())
            .collect()
    } else {
        candidate_deployments.iter().map(|d| d.id.clone()).collect()
    };

    if candidate_ids.is_empty() {
        if json {
            output::print_json(&Vec::<VerifyOutputJson>::new())?;
        } else {
            eprintln!("No deployments selected.");
        }
        return Ok(());
    }

    let total = candidate_ids.len();
    let mut results: Vec<VerifyOutputJson> = Vec::new();
    let mut ver_badges: Vec<String> = Vec::new();

    for (i, dep_id) in candidate_ids.iter().enumerate() {
        let dep_snapshot = registry.get_deployment(dep_id).unwrap().clone();
        let contract_name = dep_snapshot.contract_name.clone();
        let address = dep_snapshot.address.clone();
        let chain_id = dep_snapshot.chain_id;

        if !json {
            let action = if dep_snapshot.verification.status == VerificationStatus::Verified {
                "Re-verifying"
            } else {
                "Verifying"
            };
            output::print_stage(
                "\u{1f50d}",
                &format!(
                    "[{}/{}] {} {} ({})",
                    i + 1,
                    total,
                    action,
                    contract_name,
                    output::truncate_address(&address),
                ),
            );
        }

        let mut dep_owned = dep_snapshot;
        let verifier_count = verifiers.len();

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
                    dep_owned.verification.verifiers.insert(
                        verifier.to_lowercase(),
                        VerifierStatus {
                            status: "FAILED".to_string(),
                            url: String::new(),
                            reason: reason.clone(),
                        },
                    );
                    if !json {
                        eprintln!(
                            "  {}: {}",
                            verifier,
                            styled("FAILED", color::FAILED),
                        );
                        eprintln!("    {}", styled(&reason, color::MUTED));
                    }
                    continue;
                }
            };

            let result = verify_args.run().await;
            let explorer_url = treb_verify::explorer_url(chain_id, &address, verifier);

            match result {
                Ok(()) => {
                    // Set etherscan_url from first successful verification only.
                    if dep_owned.verification.etherscan_url.is_empty() {
                        if let Some(ref url) = explorer_url {
                            dep_owned.verification.etherscan_url = url.clone();
                        }
                    }
                    if dep_owned.verification.verified_at.is_none() {
                        dep_owned.verification.verified_at = Some(Utc::now());
                    }
                    dep_owned.verification.verifiers.insert(
                        verifier.to_lowercase(),
                        VerifierStatus {
                            status: "VERIFIED".to_string(),
                            url: explorer_url.clone().unwrap_or_default(),
                            reason: String::new(),
                        },
                    );
                    if !json {
                        eprintln!(
                            "  {}: {}",
                            verifier,
                            styled("VERIFIED", color::VERIFIED),
                        );
                        if let Some(ref url) = explorer_url {
                            eprintln!("    {}", styled(url, color::MUTED));
                        }
                    }
                }
                Err(e) => {
                    let reason = format!("{e:#}");
                    dep_owned.verification.verifiers.insert(
                        verifier.to_lowercase(),
                        VerifierStatus {
                            status: "FAILED".to_string(),
                            url: String::new(),
                            reason: reason.clone(),
                        },
                    );
                    if !json {
                        eprintln!(
                            "  {}: {}",
                            verifier,
                            styled("FAILED", color::FAILED),
                        );
                        eprintln!("    {}", styled(&reason, color::MUTED));
                    }
                }
            }

            // Rate limiting delay between verifier attempts (skip after the last one).
            if vi + 1 < verifier_count && delay > 0 {
                tokio::time::sleep(Duration::from_secs(delay)).await;
            }
        }

        // Compute aggregate status and update registry once per deployment.
        let agg_status = aggregate_status(&dep_owned.verification.verifiers);
        dep_owned.verification.status = agg_status.clone();
        // verified_at already set on first successful verification above.
        dep_owned.verification.reason = dep_owned
            .verification
            .verifiers
            .values()
            .find(|v| v.status == "FAILED")
            .map(|v| v.reason.clone())
            .unwrap_or_default();

        registry.update_deployment(dep_owned.clone())?;

        // Compute verification badge for summary table.
        let ver_badge = if color::is_color_enabled() {
            badge::verification_badge_styled(&dep_owned.verification.verifiers)
        } else {
            badge::verification_badge(&dep_owned.verification.verifiers)
        };
        ver_badges.push(ver_badge);

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
            explorer_url: dep_owned.verification.etherscan_url,
            reason: dep_owned.verification.reason,
            verified_at: dep_owned.verification.verified_at.map(|t| t.to_rfc3339()),
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
        // Print summary table.
        let mut table = output::build_table(&["Contract", "Address", "Status", "Verifiers"]);
        for (r, ver_badge) in results.iter().zip(ver_badges.iter()) {
            let status_styled = match r.status.as_str() {
                "VERIFIED" => styled("VERIFIED", color::VERIFIED),
                "FAILED" => styled("FAILED", color::FAILED),
                "PARTIAL" => styled("PARTIAL", color::WARNING),
                _ => r.status.clone(),
            };
            table.add_row(vec![
                r.contract_name.clone(),
                output::truncate_address(&r.address),
                status_styled,
                ver_badge.clone(),
            ]);
        }
        output::print_table(&table);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use treb_core::types::{VerificationStatus, VerifierStatus};

    use super::{aggregate_status, resolve_api_key};

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
                    // SAFETY: Serialized by env_lock(), so no concurrent env mutation in these tests.
                    unsafe { std::env::set_var(self.key, value) };
                }
                None => {
                    // SAFETY: Serialized by env_lock(), so no concurrent env mutation in these tests.
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
            VerifierStatus { status: "VERIFIED".to_string(), url: String::new(), reason: String::new() },
        );
        verifiers.insert(
            "sourcify".to_string(),
            VerifierStatus { status: "VERIFIED".to_string(), url: String::new(), reason: String::new() },
        );
        verifiers.insert(
            "blockscout".to_string(),
            VerifierStatus { status: "VERIFIED".to_string(), url: String::new(), reason: String::new() },
        );
        assert_eq!(aggregate_status(&verifiers), VerificationStatus::Verified);
    }

    #[test]
    fn aggregate_status_all_failed() {
        let mut verifiers = HashMap::new();
        verifiers.insert(
            "etherscan".to_string(),
            VerifierStatus { status: "FAILED".to_string(), url: String::new(), reason: "timeout".to_string() },
        );
        verifiers.insert(
            "sourcify".to_string(),
            VerifierStatus { status: "FAILED".to_string(), url: String::new(), reason: "not found".to_string() },
        );
        assert_eq!(aggregate_status(&verifiers), VerificationStatus::Failed);
    }

    #[test]
    fn aggregate_status_mixed_returns_partial() {
        let mut verifiers = HashMap::new();
        verifiers.insert(
            "etherscan".to_string(),
            VerifierStatus { status: "VERIFIED".to_string(), url: String::new(), reason: String::new() },
        );
        verifiers.insert(
            "sourcify".to_string(),
            VerifierStatus { status: "FAILED".to_string(), url: String::new(), reason: "error".to_string() },
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
            VerifierStatus { status: "VERIFIED".to_string(), url: String::new(), reason: String::new() },
        );
        assert_eq!(aggregate_status(&verifiers), VerificationStatus::Verified);
    }

    #[test]
    fn aggregate_status_single_failed() {
        let mut verifiers = HashMap::new();
        verifiers.insert(
            "etherscan".to_string(),
            VerifierStatus { status: "FAILED".to_string(), url: String::new(), reason: "err".to_string() },
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
}
