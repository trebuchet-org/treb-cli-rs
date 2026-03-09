//! `treb init` command implementation.

use std::{env, path::Path};

use anyhow::{Context, bail};
use owo_colors::OwoColorize;
use treb_config::{LocalConfig, save_local_config};
use treb_registry::Registry;

use crate::{
    output,
    ui::{color, emoji},
};

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
        if color::is_color_enabled() {
            println!("{}  {}", emoji::WARNING, "Project already initialized".style(color::WARNING));
        } else {
            println!("{}  Project already initialized", emoji::WARNING);
        }
        return Ok(());
    }

    if force && treb_dir.exists() {
        // --force: overwrite config.local.json but preserve registry data files.
        write_default_config(&cwd)
            .inspect_err(|err| render_init_failure("Failed to reset local config", err))?;
        println!("{}", output::format_success("Reset local config"));
        return Ok(());
    }

    // Fresh init: create .treb/ with registry + local config.
    Registry::init(&cwd)
        .context("failed to initialize registry")
        .inspect_err(|err| render_init_failure("Failed to initialize registry in .treb/", err))?;
    println!("{}", output::format_success("Initialized registry in .treb/"));

    write_default_config(&cwd)
        .inspect_err(|err| render_init_failure("Failed to create config.local.json", err))?;
    println!("{}", output::format_success("Created config.local.json"));

    // Success banner.
    println!();
    if color::is_color_enabled() {
        println!("{} {}", emoji::PARTY, "treb initialized successfully!".style(color::SUCCESS));
    } else {
        println!("{} treb initialized successfully!", emoji::PARTY);
    }

    // Next steps.
    print_next_steps();

    Ok(())
}

fn print_next_steps() {
    println!();
    if color::is_color_enabled() {
        println!("{} {}", emoji::CLIPBOARD, "Next steps:".style(color::STAGE));
    } else {
        println!("{} Next steps:", emoji::CLIPBOARD);
    }

    println!("1. View and configure your project:");
    print_command("treb config show");
    print_command("treb config set namespace <name>");
    print_command("treb config set network <network>");

    println!();
    println!("2. Run a deployment script:");
    print_command("treb run script/Deploy.s.sol --network sepolia");

    println!();
    println!("3. View and manage deployments:");
    print_command("treb list");
}

fn print_command(cmd: &str) {
    if color::is_color_enabled() {
        println!("   {}", cmd.style(color::GRAY));
    } else {
        println!("   {cmd}");
    }
}

fn render_init_failure(step: &str, err: &anyhow::Error) {
    println!("{}", output::format_error(step));

    let detail = err.chain().last().map(ToString::to_string).unwrap_or_else(|| err.to_string());
    if color::is_color_enabled() {
        println!("   {}", detail.style(color::RED));
    } else {
        println!("   {detail}");
    }
}

fn write_default_config(project_root: &Path) -> anyhow::Result<()> {
    let config = LocalConfig::default();
    save_local_config(project_root, &config).context("failed to write config.local.json")?;
    Ok(())
}
