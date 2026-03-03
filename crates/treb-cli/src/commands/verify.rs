//! `treb verify` command implementation.

use std::env;
use std::time::Duration;

use anyhow::{bail, Context};
use chrono::Utc;
use serde::Serialize;
use treb_core::types::{VerificationStatus, VerifierStatus};
use treb_registry::Registry;
use treb_verify::VerifyOpts;

use crate::commands::resolve::resolve_deployment;
use crate::output;

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

/// Run the verify command.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    deployment: Option<String>,
    all: bool,
    verifier: &str,
    verifier_url: Option<String>,
    verifier_api_key: Option<String>,
    force: bool,
    watch: bool,
    retries: u32,
    delay: u64,
    json: bool,
) -> anyhow::Result<()> {
    // Validate that either a deployment or --all is provided.
    if deployment.is_none() && !all {
        bail!(
            "either a <DEPLOYMENT> argument or --all flag is required\n\n\
             Usage:\n  treb verify <DEPLOYMENT>\n  treb verify --all"
        );
    }

    // Validate verifier value.
    match verifier {
        "etherscan" | "sourcify" | "blockscout" => {}
        other => {
            bail!(
                "unknown verifier '{}': expected one of etherscan, sourcify, blockscout",
                other
            );
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
        return run_batch(verifier, verifier_url, verifier_api_key, force, watch, retries, delay, json, &cwd).await;
    }

    // --- Single deployment verification ---

    let query = deployment.as_deref().unwrap();
    let mut registry = Registry::open(&cwd).context("failed to open registry")?;
    let lookup = registry.load_lookup_index().context("failed to load lookup index")?;
    let resolved = resolve_deployment(query, &registry, &lookup)?;

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

    // Build verification options from CLI args.
    let opts = VerifyOpts {
        verifier: verifier.to_string(),
        verifier_url,
        verifier_api_key: verifier_api_key.clone(),
        etherscan_api_key: verifier_api_key,
        rpc_url: None,
        force,
        watch,
        retries,
        delay: delay as u32,
        root: cwd,
    };

    let verify_args = treb_verify::build_verify_args(resolved, &opts)?;
    // `resolved` borrow ends here (NLL) — registry is free for mutation.

    eprintln!("Verifying {} ({})...", contract_name, &address);
    let result = verify_args.run().await;

    let explorer_url = treb_verify::explorer_url(chain_id, &address, verifier);

    // Update registry based on verification result.
    let mut dep = registry.get_deployment(&deployment_id).unwrap().clone();

    match result {
        Ok(()) => {
            dep.verification.status = VerificationStatus::Verified;
            dep.verification.verified_at = Some(Utc::now());
            if let Some(ref url) = explorer_url {
                dep.verification.etherscan_url = url.clone();
            }
            dep.verification.verifiers.insert(
                verifier.to_string(),
                VerifierStatus {
                    status: "VERIFIED".to_string(),
                    url: explorer_url.clone().unwrap_or_default(),
                    reason: String::new(),
                },
            );
            registry.update_deployment(dep)?;

            if json {
                let out = VerifyOutputJson {
                    deployment_id,
                    contract_name,
                    address,
                    chain_id,
                    verifier: verifier.to_string(),
                    status: "VERIFIED".to_string(),
                    explorer_url: explorer_url.unwrap_or_default(),
                    reason: String::new(),
                    verified_at: Some(Utc::now().to_rfc3339()),
                };
                output::print_json(&out)?;
            } else {
                eprintln!("Verified {} on {}", contract_name, verifier);
                if let Some(ref url) = explorer_url {
                    eprintln!("  {url}");
                }
            }
        }
        Err(e) => {
            let reason = format!("{e:#}");
            dep.verification.status = VerificationStatus::Failed;
            dep.verification.reason = reason.clone();
            dep.verification.verifiers.insert(
                verifier.to_string(),
                VerifierStatus {
                    status: "FAILED".to_string(),
                    url: String::new(),
                    reason: reason.clone(),
                },
            );
            registry.update_deployment(dep)?;

            if json {
                let out = VerifyOutputJson {
                    deployment_id,
                    contract_name,
                    address,
                    chain_id,
                    verifier: verifier.to_string(),
                    status: "FAILED".to_string(),
                    explorer_url: String::new(),
                    reason,
                    verified_at: None,
                };
                output::print_json(&out)?;
            } else {
                bail!("verification failed for {}: {}", contract_name, reason);
            }
        }
    }

    Ok(())
}

/// Batch verification: verify all unverified (or all with --force) deployments.
#[allow(clippy::too_many_arguments)]
async fn run_batch(
    verifier: &str,
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

    // Collect candidate deployment IDs up front to avoid borrow conflicts.
    let candidate_ids: Vec<String> = registry
        .list_deployments()
        .iter()
        .filter(|d| force || d.verification.status != VerificationStatus::Verified)
        .map(|d| d.id.clone())
        .collect();

    if candidate_ids.is_empty() {
        if json {
            output::print_json(&Vec::<VerifyOutputJson>::new())?;
        } else {
            eprintln!("No unverified deployments found.");
        }
        return Ok(());
    }

    let total = candidate_ids.len();
    let mut results: Vec<VerifyOutputJson> = Vec::new();

    for (i, dep_id) in candidate_ids.iter().enumerate() {
        let dep = registry.get_deployment(dep_id).unwrap();
        let contract_name = dep.contract_name.clone();
        let address = dep.address.clone();
        let chain_id = dep.chain_id;

        eprintln!("[{}/{}] Verifying {} ({})...", i + 1, total, contract_name, output::truncate_address(&address));

        let opts = VerifyOpts {
            verifier: verifier.to_string(),
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

        let verify_args = match treb_verify::build_verify_args(dep, &opts) {
            Ok(args) => args,
            Err(e) => {
                let reason = format!("{e:#}");
                eprintln!("  Failed to build verify args: {reason}");
                results.push(VerifyOutputJson {
                    deployment_id: dep_id.clone(),
                    contract_name,
                    address,
                    chain_id,
                    verifier: verifier.to_string(),
                    status: "FAILED".to_string(),
                    explorer_url: String::new(),
                    reason,
                    verified_at: None,
                });
                continue;
            }
        };
        // Borrow on `dep` released here (NLL).

        let result = verify_args.run().await;
        let explorer_url = treb_verify::explorer_url(chain_id, &address, verifier);

        let mut dep_owned = registry.get_deployment(dep_id).unwrap().clone();

        match result {
            Ok(()) => {
                dep_owned.verification.status = VerificationStatus::Verified;
                dep_owned.verification.verified_at = Some(Utc::now());
                if let Some(ref url) = explorer_url {
                    dep_owned.verification.etherscan_url = url.clone();
                }
                dep_owned.verification.verifiers.insert(
                    verifier.to_string(),
                    VerifierStatus {
                        status: "VERIFIED".to_string(),
                        url: explorer_url.clone().unwrap_or_default(),
                        reason: String::new(),
                    },
                );
                registry.update_deployment(dep_owned)?;

                results.push(VerifyOutputJson {
                    deployment_id: dep_id.clone(),
                    contract_name,
                    address,
                    chain_id,
                    verifier: verifier.to_string(),
                    status: "VERIFIED".to_string(),
                    explorer_url: explorer_url.unwrap_or_default(),
                    reason: String::new(),
                    verified_at: Some(Utc::now().to_rfc3339()),
                });
            }
            Err(e) => {
                let reason = format!("{e:#}");
                dep_owned.verification.status = VerificationStatus::Failed;
                dep_owned.verification.reason = reason.clone();
                dep_owned.verification.verifiers.insert(
                    verifier.to_string(),
                    VerifierStatus {
                        status: "FAILED".to_string(),
                        url: String::new(),
                        reason: reason.clone(),
                    },
                );
                registry.update_deployment(dep_owned)?;

                eprintln!("  Failed: {reason}");
                results.push(VerifyOutputJson {
                    deployment_id: dep_id.clone(),
                    contract_name,
                    address,
                    chain_id,
                    verifier: verifier.to_string(),
                    status: "FAILED".to_string(),
                    explorer_url: String::new(),
                    reason,
                    verified_at: None,
                });
            }
        }

        // Rate limiting delay between requests (skip after the last one).
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
            let detail = if r.status == "VERIFIED" {
                &r.explorer_url
            } else {
                &r.reason
            };
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
