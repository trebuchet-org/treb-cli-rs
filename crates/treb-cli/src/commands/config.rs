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

fn format_sender_rows(senders: &HashMap<String, SenderConfig>) -> String {
    let mut rows: Vec<(&str, String, &str)> = senders
        .iter()
        .map(|(role, sender)| {
            (
                role.as_str(),
                sender.type_.as_ref().map(ToString::to_string).unwrap_or_default(),
                sender.address.as_deref().unwrap_or(""),
            )
        })
        .collect();
    rows.sort_by(|(left, ..), (right, ..)| left.cmp(right));

    let role_width = rows.iter().map(|(role, ..)| role.len()).max().unwrap_or(0);
    let type_width = rows.iter().map(|(_, type_str, _)| type_str.len()).max().unwrap_or(0);

    rows.into_iter()
        .map(|(role, type_str, address)| {
            if address.is_empty() {
                format!("  {role:<role_width$}  {type_str}")
            } else {
                format!("  {role:<role_width$}  {type_str:<type_width$}  {address}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use treb_config::SenderType;

    fn sender_config(type_: SenderType, address: Option<&str>) -> SenderConfig {
        SenderConfig {
            type_: Some(type_),
            address: address.map(str::to_string),
            ..SenderConfig::default()
        }
    }

    #[test]
    fn format_sender_rows_sorts_roles_and_aligns_columns() {
        let mut senders = HashMap::new();
        senders.insert(
            "ops".to_string(),
            sender_config(SenderType::Ledger, Some("0x2222222222222222222222222222222222222222")),
        );
        senders.insert(
            "deployer".to_string(),
            sender_config(
                SenderType::PrivateKey,
                Some("0x1111111111111111111111111111111111111111"),
            ),
        );
        senders.insert("admin".to_string(), sender_config(SenderType::Safe, None));

        assert_eq!(
            format_sender_rows(&senders),
            concat!(
                "  admin     safe\n",
                "  deployer  private_key  0x1111111111111111111111111111111111111111\n",
                "  ops       ledger       0x2222222222222222222222222222222222222222"
            )
        );
    }

    #[test]
    fn format_sender_rows_returns_empty_string_when_no_senders() {
        assert!(format_sender_rows(&HashMap::new()).is_empty());
    }
}
