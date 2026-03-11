//! `treb addressbook` subcommands.

use std::{collections::HashMap, env};

use alloy_chains::Chain;
use anyhow::{Context, bail};
use clap::Subcommand;
use owo_colors::{OwoColorize, Style};
use serde::Serialize;
use treb_config::{ResolveOpts, resolve_config};
use treb_registry::Registry;

use crate::{output, ui::color};

const FOUNDRY_TOML: &str = "foundry.toml";
const TREB_DIR: &str = ".treb";

#[derive(Subcommand, Debug)]
pub enum AddressbookSubcommand {
    /// Set an addressbook entry for the current chain
    Set {
        /// Entry name
        name: String,
        /// Contract or account address
        address: String,
    },
    /// Remove an addressbook entry for the current chain
    Remove {
        /// Entry name
        name: String,
    },
    /// List addressbook entries for the current chain
    #[command(alias = "ls")]
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(
    namespace: Option<String>,
    network: Option<String>,
    subcommand: AddressbookSubcommand,
) -> anyhow::Result<()> {
    match subcommand {
        AddressbookSubcommand::Set { name, address } => run_set(namespace, network, name, address),
        AddressbookSubcommand::Remove { name } => run_remove(namespace, network, name),
        AddressbookSubcommand::List { json } => run_list(namespace, network, json),
    }
}

fn run_set(
    namespace: Option<String>,
    network: Option<String>,
    name: String,
    address: String,
) -> anyhow::Result<()> {
    validate_address(&address)?;

    let cwd = env::current_dir().context("failed to determine current directory")?;
    ensure_initialized(&cwd)?;

    let chain_id = resolve_effective_chain_id(&cwd, namespace, network)
        .context("failed to resolve chain ID")?;

    let mut registry = Registry::open(&cwd).context("failed to open registry")?;
    registry
        .set_addressbook_entry(&chain_id.to_string(), &name, &address)
        .map_err(|err| anyhow::anyhow!("{err}"))?;

    println!("Set {name} = {address} (chain {chain_id})");
    Ok(())
}

fn run_remove(
    namespace: Option<String>,
    network: Option<String>,
    name: String,
) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    ensure_initialized(&cwd)?;

    let chain_id = resolve_effective_chain_id(&cwd, namespace, network)
        .context("failed to resolve chain ID")?;

    let mut registry = Registry::open(&cwd).context("failed to open registry")?;
    registry
        .remove_addressbook_entry(&chain_id.to_string(), &name)
        .map_err(|err| map_remove_entry_error(&name, chain_id, err))?;

    println!("Removed {name} (chain {chain_id})");
    Ok(())
}

fn run_list(namespace: Option<String>, network: Option<String>, json: bool) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;
    ensure_initialized(&cwd)?;

    let chain_id = resolve_effective_chain_id(&cwd, namespace, network)
        .context("failed to resolve chain ID")?;

    let mut registry = Registry::open(&cwd).context("failed to open registry")?;
    let entries = registry
        .list_addressbook_entries(&chain_id.to_string())
        .map_err(|err| anyhow::anyhow!("{err}"))?;

    if json {
        let json_entries: Vec<AddressbookEntryJson> = entries
            .into_iter()
            .map(|(name, address)| AddressbookEntryJson { name, address })
            .collect();
        output::print_json(&json_entries)?;
        return Ok(());
    }

    if entries.is_empty() {
        println!("No addressbook entries found");
        return Ok(());
    }

    for (name, address) in entries {
        print_entry(&name, &address);
    }

    Ok(())
}

fn map_remove_entry_error(name: &str, chain_id: u64, err: impl std::fmt::Display) -> anyhow::Error {
    let message = err.to_string();
    if message.contains("addressbook entry not found") {
        anyhow::anyhow!("addressbook entry '{name}' not found on chain {chain_id}")
    } else {
        anyhow::anyhow!("{message}")
    }
}

fn print_entry(name: &str, address: &str) {
    let padded_name = format!("{name:<24}");
    let rendered_name = if color::is_color_enabled() {
        padded_name.style(Style::new().yellow().bold()).to_string()
    } else {
        padded_name
    };

    println!("  {rendered_name}  {address}");
}

fn ensure_initialized(cwd: &std::path::Path) -> anyhow::Result<()> {
    if !cwd.join(FOUNDRY_TOML).exists() {
        bail!(
            "no foundry.toml found in {}\n\n\
             Run `forge init` to create a Foundry project, then `treb init`.",
            cwd.display()
        );
    }
    if !cwd.join(TREB_DIR).exists() {
        bail!(
            "project not initialized — .treb/ directory not found in {}\n\n\
             Run `treb init` first.",
            cwd.display()
        );
    }

    Ok(())
}

fn resolve_effective_chain_id(
    cwd: &std::path::Path,
    namespace: Option<String>,
    network: Option<String>,
) -> anyhow::Result<u64> {
    let resolved = resolve_config(ResolveOpts {
        project_root: cwd.to_path_buf(),
        namespace,
        network,
        profile: None,
        sender_overrides: HashMap::new(),
    })
    .map_err(|err| anyhow::anyhow!("{err}"))?;

    let configured_network = resolved.network.ok_or_else(|| {
        anyhow::anyhow!(
            "no network configured; set one with --network or 'treb config set network <name>'"
        )
    })?;

    resolve_chain_id(&configured_network)
}

fn resolve_chain_id(network: &str) -> anyhow::Result<u64> {
    if let Ok(chain_id) = network.parse::<u64>() {
        return Ok(chain_id);
    }

    let chain: Chain =
        network.parse().map_err(|_| anyhow::anyhow!("unknown network: {network}"))?;
    Ok(chain.id())
}

fn validate_address(address: &str) -> anyhow::Result<()> {
    let is_valid = address.len() == 42
        && address.starts_with("0x")
        && address[2..].chars().all(|ch| ch.is_ascii_hexdigit());

    if is_valid {
        Ok(())
    } else {
        bail!("invalid address \"{address}\": must be a 0x-prefixed 40-character hex string")
    }
}

#[derive(Serialize)]
struct AddressbookEntryJson {
    name: String,
    address: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_chain_id_accepts_numeric_and_named_networks() {
        assert_eq!(resolve_chain_id("1").unwrap(), 1);
        assert_eq!(resolve_chain_id("mainnet").unwrap(), 1);
    }

    #[test]
    fn validate_address_rejects_invalid_values() {
        let err = validate_address("0x1234").unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid address \"0x1234\": must be a 0x-prefixed 40-character hex string"
        );
    }

    #[test]
    fn remove_not_found_errors_use_cli_facing_message() {
        let err = map_remove_entry_error(
            "Treasury",
            1,
            "addressbook entry not found: Treasury on chain 1",
        );

        assert_eq!(err.to_string(), "addressbook entry 'Treasury' not found on chain 1");
    }

    #[test]
    fn list_json_output_preserves_sorted_entries() {
        let json_entries = vec![
            AddressbookEntryJson {
                name: "Alpha".to_string(),
                address: "0x1111111111111111111111111111111111111111".to_string(),
            },
            AddressbookEntryJson {
                name: "Zulu".to_string(),
                address: "0x9999999999999999999999999999999999999999".to_string(),
            },
        ];

        let value = serde_json::to_value(&json_entries).unwrap();
        assert_eq!(
            value,
            serde_json::json!([
                {
                    "name": "Alpha",
                    "address": "0x1111111111111111111111111111111111111111"
                },
                {
                    "name": "Zulu",
                    "address": "0x9999999999999999999999999999999999999999"
                }
            ])
        );
    }
}
