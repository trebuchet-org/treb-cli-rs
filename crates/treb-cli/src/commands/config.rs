//! `treb config` command implementation.

use std::{collections::HashMap, env};

use anyhow::{Context, bail};
use serde::Serialize;
use treb_config::{
    LocalConfig, ResolveOpts, SenderConfig, load_local_config, load_treb_config_v2_raw,
    resolve_config, save_local_config,
};

use owo_colors::OwoColorize;

use crate::{output, ui::color, ui::emoji};

const FOUNDRY_TOML: &str = "foundry.toml";
const TREB_DIR: &str = ".treb";

// ── config show ──────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigShowOutput {
    pub namespace: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    pub profile: String,
    pub config_source: String,
    pub project_root: String,
    pub senders: HashMap<String, SenderConfig>,
}

pub async fn show(json: bool) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    ensure_initialized(&cwd)?;

    let resolved = resolve_config(ResolveOpts {
        project_root: cwd.clone(),
        namespace: None,
        network: None,
        profile: None,
        sender_overrides: HashMap::new(),
    })
    .context("failed to resolve configuration")?;

    let output_data = ConfigShowOutput {
        namespace: resolved.namespace.clone(),
        network: resolved.network.clone(),
        profile: resolved.profile.clone(),
        config_source: resolved.config_source.clone(),
        project_root: resolved.project_root.display().to_string(),
        senders: resolved.senders.clone(),
    };

    if json {
        output::print_json(&output_data)?;
    } else {
        let network_display = resolved.network.as_deref().unwrap_or("(not set)");

        println!("Namespace: {}", resolved.namespace.style(color::CYAN));
        println!("Network:   {}", network_display.style(color::CYAN));
        println!(
            "Source:    {}",
            human_config_source(&resolved.config_source).style(color::GRAY)
        );

        // Load raw (unexpanded) senders so env var references like
        // ${DEPLOYER_PRIVATE_KEY} are shown instead of resolved values.
        let display_senders = load_raw_senders(&cwd, &resolved.config_source)
            .unwrap_or(resolved.senders.clone());
        if !display_senders.is_empty() {
            println!();
            println!("{}", format_sender_rows(&display_senders));
        }
    }

    Ok(())
}

// ── config set ───────────────────────────────────────────────────────────

pub async fn set(key: &str, value: &str) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    ensure_treb_dir(&cwd)?;

    let mut config = load_local_config(&cwd).context("failed to load local config")?;

    match key {
        "namespace" => config.namespace = value.to_string(),
        "network" => config.network = value.to_string(),
        _ => bail!("unknown config key '{}'; valid keys: namespace, network", key),
    }

    save_local_config(&cwd, &config).context("failed to save local config")?;

    println!("{} Set {} to: {}", emoji::CHECK, key, value);
    let config_path = cwd.join(".treb/config.local.json");
    let relative_path = config_path
        .strip_prefix(&cwd)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| config_path.display().to_string());
    println!("{} config saved to: {}", emoji::FOLDER, relative_path);

    Ok(())
}

// ── config remove ────────────────────────────────────────────────────────

pub async fn remove(key: &str) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    ensure_treb_dir(&cwd)?;

    let mut config = load_local_config(&cwd).context("failed to load local config")?;
    let defaults = LocalConfig::default();

    match key {
        "namespace" => {
            config.namespace = defaults.namespace.clone();
            save_local_config(&cwd, &config).context("failed to save local config")?;
            println!("{} Reset namespace to: {}", emoji::CHECK, defaults.namespace);
        }
        "network" => {
            config.network = defaults.network;
            save_local_config(&cwd, &config).context("failed to save local config")?;
            println!("{} Removed network from config (will be required as flag)", emoji::CHECK);
        }
        _ => bail!("unknown config key '{}'; valid keys: namespace, network", key),
    }

    let config_path = cwd.join(".treb/config.local.json");
    let relative_path = config_path
        .strip_prefix(&cwd)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| config_path.display().to_string());
    println!("{} config saved to: {}", emoji::FOLDER, relative_path);

    Ok(())
}

// ── helpers ──────────────────────────────────────────────────────────────

fn ensure_initialized(cwd: &std::path::Path) -> anyhow::Result<()> {
    if !cwd.join(FOUNDRY_TOML).exists() {
        bail!(
            "no foundry.toml found in {}\n\n\
             Run `forge init` to create a Foundry project, then `treb init`.",
            cwd.display()
        );
    }
    ensure_treb_dir(cwd)?;
    Ok(())
}

fn ensure_treb_dir(cwd: &std::path::Path) -> anyhow::Result<()> {
    if !cwd.join(TREB_DIR).exists() {
        bail!(
            "project not initialized — .treb/ directory not found in {}\n\n\
             Run `treb init` first.",
            cwd.display()
        );
    }
    Ok(())
}

/// Load raw sender configs without env var expansion for display purposes.
fn load_raw_senders(
    cwd: &std::path::Path,
    config_source: &str,
) -> Option<HashMap<String, SenderConfig>> {
    let treb_toml = cwd.join("treb.toml");
    if !treb_toml.exists() {
        return None;
    }
    // Only v2 treb.toml has the raw loader; for v1/foundry.toml fall back to resolved.
    if !config_source.contains("treb.toml") {
        return None;
    }
    let raw_config = load_treb_config_v2_raw(&treb_toml).ok()?;
    Some(raw_config.accounts)
}

fn human_config_source(config_source: &str) -> &str {
    if let Some((source_name, version)) = config_source.rsplit_once(" (v") {
        if version
            .strip_suffix(')')
            .is_some_and(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()))
        {
            return source_name;
        }
    }

    config_source
}

/// Build display detail pairs for a sender based on its type.
///
/// Returns key=value style details showing the most relevant fields per type,
/// matching Go CLI behavior: private_key shows env var ref or "(literal key)",
/// safe shows safe address + signer, ledger/trezor shows derivation path,
/// oz_governor shows governor + timelock + proposer.
fn sender_details(sender: &SenderConfig) -> Vec<(&'static str, String)> {
    let mut details = Vec::new();

    match sender.type_.as_ref() {
        Some(treb_config::SenderType::PrivateKey) => {
            if let Some(key) = sender.private_key.as_deref() {
                if key.contains("${") || key.starts_with('$') {
                    details.push(("key", key.to_string()));
                } else {
                    details.push(("key", "(literal)".to_string()));
                }
            }
            if let Some(addr) = sender.address.as_deref() {
                details.push(("address", addr.to_string()));
            }
        }
        Some(treb_config::SenderType::Safe) => {
            if let Some(safe) = sender.safe.as_deref() {
                details.push(("safe", safe.to_string()));
            }
            if let Some(signer) = sender.signer.as_deref() {
                details.push(("signer", signer.to_string()));
            }
        }
        Some(treb_config::SenderType::Ledger | treb_config::SenderType::Trezor) => {
            if let Some(addr) = sender.address.as_deref() {
                details.push(("address", addr.to_string()));
            }
            if let Some(path) = sender.derivation_path.as_deref() {
                details.push(("path", path.to_string()));
            }
        }
        Some(treb_config::SenderType::OZGovernor) => {
            // Show timelock address if present, otherwise governor address,
            // with a label indicating which it is.
            if let Some(timelock) = sender.timelock.as_deref() {
                details.push(("timelock", timelock.to_string()));
            } else if let Some(gov) = sender.governor.as_deref() {
                details.push(("governor", gov.to_string()));
            }
            if let Some(proposer) = sender.proposer.as_deref() {
                details.push(("proposer", proposer.to_string()));
            }
        }
        None => {}
    }

    details
}

fn format_sender_rows(senders: &HashMap<String, SenderConfig>) -> String {
    let mut entries: Vec<(&str, String, Vec<(&'static str, String)>)> = senders
        .iter()
        .map(|(role, sender)| {
            let type_str = sender.type_.as_ref().map(ToString::to_string).unwrap_or_default();
            let details = sender_details(sender);
            (role.as_str(), type_str, details)
        })
        .collect();
    entries.sort_by_key(|(role, ..)| *role);

    let role_width = entries.iter().map(|(role, ..)| role.len()).max().unwrap_or(0);
    let type_width = entries.iter().map(|(_, type_str, _)| type_str.len()).max().unwrap_or(0);

    let use_color = color::is_color_enabled();

    entries
        .into_iter()
        .map(|(role, type_str, details)| {
            let detail_str = if use_color {
                details
                    .iter()
                    .map(|(k, v)| {
                        format!(
                            "{}={}",
                            k.style(color::GRAY),
                            v.style(color::CYAN)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("  ")
            } else {
                details
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join("  ")
            };

            // Pad raw strings first, then apply color to preserve alignment.
            let padded_role = format!("{role:<role_width$}");
            let role_display = if use_color {
                format!("{}", padded_role.style(color::BOLD))
            } else {
                padded_role
            };

            if detail_str.is_empty() {
                let type_display = if use_color {
                    format!("{}", type_str.style(color::GRAY))
                } else {
                    type_str
                };
                format!("  {role_display}  {type_display}")
            } else {
                let padded_type = format!("{type_str:<type_width$}");
                let type_display = if use_color {
                    format!("{}", padded_type.style(color::GRAY))
                } else {
                    padded_type
                };
                format!("  {role_display}  {type_display}  {detail_str}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use treb_config::SenderType;

    #[test]
    fn format_sender_rows_sorts_roles_and_shows_type_specific_details() {
        // Disable color so assertions compare plain text.
        crate::ui::color::color_enabled(true);
        let mut senders = HashMap::new();
        senders.insert(
            "ops".to_string(),
            SenderConfig {
                type_: Some(SenderType::Ledger),
                address: Some("${OPS_ADDR}".to_string()),
                derivation_path: Some("m/44'/60'/0'/0/0".to_string()),
                ..SenderConfig::default()
            },
        );
        senders.insert(
            "deployer".to_string(),
            SenderConfig {
                type_: Some(SenderType::PrivateKey),
                private_key: Some("${DEPLOYER_PRIVATE_KEY}".to_string()),
                ..SenderConfig::default()
            },
        );
        senders.insert(
            "admin".to_string(),
            SenderConfig {
                type_: Some(SenderType::Safe),
                safe: Some("0xSafe1111".to_string()),
                signer: Some("deployer".to_string()),
                ..SenderConfig::default()
            },
        );
        senders.insert(
            "gov".to_string(),
            SenderConfig {
                type_: Some(SenderType::OZGovernor),
                governor: Some("0xGov2222".to_string()),
                timelock: Some("0xTime333".to_string()),
                proposer: Some("admin".to_string()),
                ..SenderConfig::default()
            },
        );

        assert_eq!(
            format_sender_rows(&senders),
            concat!(
                "  admin     safe         safe=0xSafe1111  signer=deployer\n",
                "  deployer  private_key  key=${DEPLOYER_PRIVATE_KEY}\n",
                "  gov       oz_governor  timelock=0xTime333  proposer=admin\n",
                "  ops       ledger       address=${OPS_ADDR}  path=m/44'/60'/0'/0/0"
            )
        );
    }

    #[test]
    fn sender_details_private_key_literal_shows_placeholder() {
        let sender = SenderConfig {
            type_: Some(SenderType::PrivateKey),
            private_key: Some("0xdeadbeef".to_string()),
            ..SenderConfig::default()
        };
        let details = sender_details(&sender);
        assert_eq!(details, vec![("key", "(literal)".to_string())]);
    }

    #[test]
    fn format_sender_rows_returns_empty_string_when_no_senders() {
        assert!(format_sender_rows(&HashMap::new()).is_empty());
    }
}
