//! `treb verify` command implementation.

use std::{env, time::Duration};

use anyhow::{Context, bail};
use chrono::Utc;
use console::Term;
use serde::Serialize;
use treb_core::types::{VerificationStatus, VerifierStatus};
use treb_registry::Registry;
use treb_verify::VerifyOpts;

use crate::{
    commands::resolve::resolve_deployment,
    output,
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
                "deployment '{}' is already verified — use --force to re-verify",
                deployment_id
            );
        }
        return Ok(());
    }

    // Release the borrow on `resolved` before mutating registry.
    let _ = resolved;

    // --- Multi-verifier loop ---
    let mut dep = registry.get_deployment(&deployment_id).unwrap().clone();
    let verifier_count = verifiers.len();

    for (vi, verifier) in verifiers.iter().enumerate() {
        let opts = VerifyOpts {
            verifier: verifier.clone(),
            verifier_url: verifier_url.clone(),
            verifier_api_key: verifier_api_key.clone(),
            etherscan_api_key: verifier_api_key.clone(),
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
                        reason,
                    },
                );
                continue;
            }
        };

        eprintln!("Verifying {} ({}) on {}...", contract_name, &address, verifier);
        let result = verify_args.run().await;
        let explorer_url = treb_verify::explorer_url(chain_id, &address, verifier);

        match result {
            Ok(()) => {
                if let Some(ref url) = explorer_url {
                    dep.verification.etherscan_url = url.clone();
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
                    eprintln!("Verified {} on {}", contract_name, verifier);
                    if let Some(ref url) = explorer_url {
                        eprintln!("  {url}");
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
                    eprintln!("Failed to verify {} on {}: {}", contract_name, verifier, reason);
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
    if agg_status == VerificationStatus::Verified || agg_status == VerificationStatus::Partial {
        dep.verification.verified_at = Some(Utc::now());
    }
    // Set reason from first failed verifier, if any.
    dep.verification.reason = dep
        .verification
        .verifiers
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

    for (i, dep_id) in candidate_ids.iter().enumerate() {
        let dep_snapshot = registry.get_deployment(dep_id).unwrap().clone();
        let contract_name = dep_snapshot.contract_name.clone();
        let address = dep_snapshot.address.clone();
        let chain_id = dep_snapshot.chain_id;

        eprintln!(
            "[{}/{}] Verifying {} ({})...",
            i + 1,
            total,
            contract_name,
            output::truncate_address(&address)
        );

        let mut dep_owned = dep_snapshot;
        let verifier_count = verifiers.len();

        for (vi, verifier) in verifiers.iter().enumerate() {
            let opts = VerifyOpts {
                verifier: verifier.clone(),
                verifier_url: verifier_url.clone(),
                verifier_api_key: verifier_api_key.clone(),
                etherscan_api_key: verifier_api_key.clone(),
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
                    eprintln!("  Failed to build verify args for {}: {reason}", verifier);
                    dep_owned.verification.verifiers.insert(
                        verifier.to_lowercase(),
                        VerifierStatus {
                            status: "FAILED".to_string(),
                            url: String::new(),
                            reason,
                        },
                    );
                    continue;
                }
            };

            let result = verify_args.run().await;
            let explorer_url = treb_verify::explorer_url(chain_id, &address, verifier);

            match result {
                Ok(()) => {
                    if let Some(ref url) = explorer_url {
                        dep_owned.verification.etherscan_url = url.clone();
                    }
                    dep_owned.verification.verifiers.insert(
                        verifier.to_lowercase(),
                        VerifierStatus {
                            status: "VERIFIED".to_string(),
                            url: explorer_url.unwrap_or_default(),
                            reason: String::new(),
                        },
                    );
                }
                Err(e) => {
                    let reason = format!("{e:#}");
                    eprintln!("  Failed on {}: {reason}", verifier);
                    dep_owned.verification.verifiers.insert(
                        verifier.to_lowercase(),
                        VerifierStatus {
                            status: "FAILED".to_string(),
                            url: String::new(),
                            reason,
                        },
                    );
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
        if agg_status == VerificationStatus::Verified || agg_status == VerificationStatus::Partial {
            dep_owned.verification.verified_at = Some(Utc::now());
        }
        dep_owned.verification.reason = dep_owned
            .verification
            .verifiers
            .values()
            .find(|v| v.status == "FAILED")
            .map(|v| v.reason.clone())
            .unwrap_or_default();

        registry.update_deployment(dep_owned.clone())?;

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
        let mut table = output::build_table(&["Contract", "Address", "Status", "URL/Reason"]);
        for r in &results {
            let detail = if r.status == "VERIFIED" { &r.explorer_url } else { &r.reason };
            table.add_row(vec![
                r.contract_name.as_str(),
                &output::truncate_address(&r.address),
                r.status.as_str(),
                detail,
            ]);
        }
        output::print_table(&table);
    }

    Ok(())
}
