//! `treb config` command implementation.

use std::{collections::HashMap, env};

use anyhow::{Context, bail};
use serde::Serialize;
use treb_config::{
    LocalConfig, ResolveOpts, SenderConfig, load_local_config, resolve_config, save_local_config,
};

use crate::{output, ui::emoji};

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

        println!("{} Current config:", emoji::CLIPBOARD);
        println!("Namespace: {}", resolved.namespace);
        println!("Network:   {}", network_display);

        println!();
        println!(
            "{} Config source: {}",
            emoji::PACKAGE,
            human_config_source(&resolved.config_source)
        );

        let config_path = resolved.project_root.join(".treb/config.local.json");
        let relative_path = config_path
            .strip_prefix(&cwd)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| config_path.display().to_string());
        println!("{} config file: {}", emoji::FOLDER, relative_path);

        if !resolved.senders.is_empty() {
            println!();
            println!("{}", format_sender_rows(&resolved.senders));
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

/// Extract the most relevant display detail for a sender based on its type.
fn sender_detail(sender: &SenderConfig) -> String {
    match sender.type_.as_ref() {
        Some(treb_config::SenderType::PrivateKey) => {
            match sender.private_key.as_deref() {
                Some(key) if key.starts_with("${") => key.to_string(),
                Some(_) => "(literal key)".to_string(),
                None => String::new(),
            }
        }
        Some(treb_config::SenderType::Safe) => {
            sender.safe.clone().unwrap_or_default()
        }
        Some(treb_config::SenderType::Ledger | treb_config::SenderType::Trezor) => {
            sender.derivation_path.clone().unwrap_or_default()
        }
        Some(treb_config::SenderType::OZGovernor) => {
            sender.governor.clone().unwrap_or_default()
        }
        None => String::new(),
    }
}

fn format_sender_rows(senders: &HashMap<String, SenderConfig>) -> String {
    let mut rows: Vec<(&str, String, String)> = senders
        .iter()
        .map(|(role, sender)| {
            let type_str = sender.type_.as_ref().map(ToString::to_string).unwrap_or_default();
            let detail = sender_detail(sender);
            (role.as_str(), type_str, detail)
        })
        .collect();
    rows.sort_by_key(|(role, ..)| *role);

    let role_width = rows.iter().map(|(role, ..)| role.len()).max().unwrap_or(0);
    let type_width = rows.iter().map(|(_, type_str, _)| type_str.len()).max().unwrap_or(0);

    rows.into_iter()
        .map(|(role, type_str, detail)| {
            if detail.is_empty() {
                format!("  {role:<role_width$}  {type_str}")
            } else {
                format!("  {role:<role_width$}  {type_str:<type_width$}  {detail}")
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
    fn format_sender_rows_sorts_roles_and_shows_type_specific_detail() {
        let mut senders = HashMap::new();
        senders.insert(
            "ops".to_string(),
            SenderConfig {
                type_: Some(SenderType::Ledger),
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
                safe: Some("0xSafe1111111111111111111111111111111111".to_string()),
                signer: Some("deployer".to_string()),
                ..SenderConfig::default()
            },
        );
        senders.insert(
            "gov".to_string(),
            SenderConfig {
                type_: Some(SenderType::OZGovernor),
                governor: Some("0xGov22222222222222222222222222222222222".to_string()),
                proposer: Some("admin".to_string()),
                ..SenderConfig::default()
            },
        );

        assert_eq!(
            format_sender_rows(&senders),
            concat!(
                "  admin     safe         0xSafe1111111111111111111111111111111111\n",
                "  deployer  private_key  ${DEPLOYER_PRIVATE_KEY}\n",
                "  gov       oz_governor  0xGov22222222222222222222222222222222222\n",
                "  ops       ledger       m/44'/60'/0'/0/0"
            )
        );
    }

    #[test]
    fn sender_detail_private_key_literal_shows_placeholder() {
        let sender = SenderConfig {
            type_: Some(SenderType::PrivateKey),
            private_key: Some("0xdeadbeef".to_string()),
            ..SenderConfig::default()
        };
        assert_eq!(sender_detail(&sender), "(literal key)");
    }

    #[test]
    fn format_sender_rows_returns_empty_string_when_no_senders() {
        assert!(format_sender_rows(&HashMap::new()).is_empty());
    }
}
