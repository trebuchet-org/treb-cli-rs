//! `treb addressbook` subcommands.

use std::{collections::HashMap, env};

use alloy_chains::Chain;
use anyhow::{Context, bail};
use clap::Subcommand;
use treb_config::{ResolveOpts, resolve_config};
use treb_registry::Registry;

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
    List,
}

pub async fn run(
    namespace: Option<String>,
    network: Option<String>,
    subcommand: AddressbookSubcommand,
) -> anyhow::Result<()> {
    match subcommand {
        AddressbookSubcommand::Set { name, address } => run_set(namespace, network, name, address),
        AddressbookSubcommand::Remove { .. } => {
            bail!("addressbook remove is not implemented yet")
        }
        AddressbookSubcommand::List => bail!("addressbook list is not implemented yet"),
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
            "no network configured; pass --network <network> or set one with `treb config set network <network>`"
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
        bail!("invalid address '{address}'; expected a 0x-prefixed 40-hex-character address")
    }
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
            "invalid address '0x1234'; expected a 0x-prefixed 40-hex-character address"
        );
    }
}
