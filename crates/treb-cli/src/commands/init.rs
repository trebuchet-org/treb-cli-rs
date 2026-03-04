//! `treb init` command implementation.

use std::{env, path::Path};

use anyhow::{Context, bail};
use treb_config::{LocalConfig, save_local_config};
use treb_registry::Registry;

const FOUNDRY_TOML: &str = "foundry.toml";
const TREB_DIR: &str = ".treb";

pub async fn run(force: bool) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;

    // Validate foundry.toml exists.
    if !cwd.join(FOUNDRY_TOML).exists() {
        bail!(
            "no foundry.toml found in {}\n\n\
             treb requires a Foundry project. Run `forge init` first, \
             then try `treb init` again.",
            cwd.display()
        );
    }

    let treb_dir = cwd.join(TREB_DIR);

    if treb_dir.exists() && !force {
        println!("Project already initialized at {}", treb_dir.display());
        return Ok(());
    }

    if force && treb_dir.exists() {
        // --force: overwrite config.local.json but preserve registry data files.
        write_default_config(&cwd)?;
        println!("Reset local config at {}", treb_dir.display());
        println!("Run `treb config show` to view your configuration.");
        return Ok(());
    }

    // Fresh init: create .treb/ with registry + local config.
    Registry::init(&cwd).context("failed to initialize registry")?;
    write_default_config(&cwd)?;

    println!("Initialized treb project at {}", treb_dir.display());
    println!("Run `treb config show` to view your configuration.");

    Ok(())
}

fn write_default_config(project_root: &Path) -> anyhow::Result<()> {
    let config = LocalConfig::default();
    save_local_config(project_root, &config).context("failed to write config.local.json")?;
    Ok(())
}
