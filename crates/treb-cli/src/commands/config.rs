//! `treb config` command implementation.

use std::{collections::HashMap, env};

use anyhow::{Context, bail};
use serde::Serialize;
use treb_config::{
    LocalConfig, ResolveOpts, SenderConfig, load_local_config, resolve_config, save_local_config,
};

use crate::output;

const FOUNDRY_TOML: &str = "foundry.toml";
const TREB_DIR: &str = ".treb";

// ── config show ──────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct ConfigShowOutput {
    pub namespace: String,
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
        project_root: cwd,
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
        let network_display = resolved.network.as_deref().unwrap_or("not set");

        output::print_kv(&[
            ("Namespace", &resolved.namespace),
            ("Network", network_display),
            ("Profile", &resolved.profile),
            ("Config Source", &resolved.config_source),
            ("Project Root", &resolved.project_root.display().to_string()),
        ]);

        if !resolved.senders.is_empty() {
            println!();
            let mut table = output::build_table(&["Role", "Type", "Address"]);
            let mut roles: Vec<&String> = resolved.senders.keys().collect();
            roles.sort();
            for role in roles {
                let sender = &resolved.senders[role];
                let type_str = sender.type_.as_ref().map(|t| t.to_string()).unwrap_or_default();
                let addr = sender.address.as_deref().unwrap_or("");
                table.add_row(vec![role.as_str(), &type_str, addr]);
            }
            output::print_table(&table);
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
    println!("Set {} = {}", key, value);

    Ok(())
}

// ── config remove ────────────────────────────────────────────────────────

pub async fn remove(key: &str) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    ensure_treb_dir(&cwd)?;

    let mut config = load_local_config(&cwd).context("failed to load local config")?;
    let defaults = LocalConfig::default();

    match key {
        "namespace" => config.namespace = defaults.namespace,
        "network" => config.network = defaults.network,
        _ => bail!("unknown config key '{}'; valid keys: namespace, network", key),
    }

    save_local_config(&cwd, &config).context("failed to save local config")?;
    println!("Removed {} (reset to default)", key);

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
