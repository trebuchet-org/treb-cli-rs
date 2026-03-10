//! `treb networks` command implementation.
//!
//! Lists all configured RPC endpoints from `foundry.toml` with their resolved
//! chain IDs via `eth_chainId` JSON-RPC calls.

use std::time::Duration;

use serde::Serialize;

use crate::{output, ui::emoji};

/// Network information for a single RPC endpoint.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInfo {
    pub name: String,
    pub rpc_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    pub status: String,
}

/// Resolve the chain ID for a single RPC endpoint via `eth_chainId`.
async fn resolve_chain_id(client: &reqwest::Client, url: &str) -> (Option<u64>, String) {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "eth_chainId",
        "params": [],
        "id": 1
    });

    let resp = match client.post(url).json(&body).send().await {
        Ok(r) => r,
        Err(_) => return (None, "unreachable".to_string()),
    };

    let json: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(_) => return (None, "invalid response".to_string()),
    };

    match json.get("result").and_then(|r| r.as_str()) {
        Some(hex) => {
            let hex = hex.strip_prefix("0x").unwrap_or(hex);
            match u64::from_str_radix(hex, 16) {
                Ok(id) => (Some(id), "ok".to_string()),
                Err(_) => (None, "invalid chain id".to_string()),
            }
        }
        None => (None, "no result".to_string()),
    }
}

/// Returns true if the URL string contains unresolved environment variable
/// references like `${VAR}` or `$VAR`.
fn has_unresolved_env_vars(url: &str) -> bool {
    let bytes = url.as_bytes();
    let mut idx = 0;

    while idx < bytes.len() {
        if bytes[idx] == b'$' {
            match bytes.get(idx + 1).copied() {
                Some(b'{') => return true,
                Some(next) if next == b'_' || next.is_ascii_alphabetic() => return true,
                _ => {}
            }
        }

        idx += 1;
    }

    false
}

pub async fn run(json: bool) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;

    // Check that foundry.toml actually exists
    if !cwd.join("foundry.toml").exists() {
        anyhow::bail!(
            "no foundry.toml found in the current directory.\n\
             Run this command from a Foundry project root, or create a foundry.toml file."
        );
    }

    let config = treb_config::load_foundry_config(&cwd).map_err(|e| anyhow::anyhow!("{e}"))?;
    let endpoints = treb_config::rpc_endpoints(&config);

    if endpoints.is_empty() {
        if json {
            output::print_json(&Vec::<NetworkInfo>::new())?;
        } else {
            println!("No networks configured in foundry.toml [rpc_endpoints]");
        }
        return Ok(());
    }

    // Sort endpoints alphabetically by name
    let mut names: Vec<&String> = endpoints.keys().collect();
    names.sort();

    let client = reqwest::Client::builder().timeout(Duration::from_secs(5)).build()?;

    // Resolve chain IDs concurrently
    let mut results: Vec<NetworkInfo> = Vec::with_capacity(names.len());
    let mut futures = Vec::new();

    for name in &names {
        let url = endpoints[*name].clone();
        let name = (*name).clone();
        let client = client.clone();

        futures.push(tokio::spawn(async move {
            if has_unresolved_env_vars(&url) {
                NetworkInfo {
                    name,
                    rpc_url: url,
                    chain_id: None,
                    status: "unresolved env var".to_string(),
                }
            } else {
                let (chain_id, status) = resolve_chain_id(&client, &url).await;
                NetworkInfo { name, rpc_url: url, chain_id, status }
            }
        }));
    }

    for handle in futures {
        results.push(handle.await?);
    }

    // Sort results alphabetically by name (spawn order may differ)
    results.sort_by(|a, b| a.name.cmp(&b.name));

    if json {
        output::print_json(&results)?;
    } else {
        println!("{} Available Networks:", emoji::GLOBE);
        println!();

        for info in &results {
            if info.status == "ok" {
                println!(
                    "  {} {} - Chain ID: {}",
                    emoji::CHECK,
                    info.name,
                    info.chain_id.unwrap_or(0),
                );
            } else {
                println!("  {} {} - Error: {}", emoji::CROSS, info.name, info.status);
            }
        }
    }

    Ok(())
}
