//! `treb migrate` command implementation.
//!
//! Handles config format detection and v1→v2 conversion (`treb migrate config`)
//! and versioned registry schema migrations (`treb migrate registry`).

use std::{
    collections::HashMap,
    env,
    io::IsTerminal,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, bail};
use clap::Subcommand;
use treb_config::{
    AccountConfig, NamespaceRoles, TrebConfigFormat, detect_treb_config_format,
    extract_treb_senders_from_foundry, load_treb_config_v1, serialize_treb_config_v2,
};
use treb_registry::{REGISTRY_DIR, REGISTRY_VERSION, run_migrations};

use owo_colors::OwoColorize;

use crate::{
    output,
    ui::{color, emoji},
};

/// Apply a color style when color is enabled, plain text otherwise.
fn styled(text: &str, style: owo_colors::Style) -> String {
    if color::is_color_enabled() { format!("{}", text.style(style)) } else { text.to_string() }
}

// ── Subcommand ────────────────────────────────────────────────────────────────

/// Subcommands for `treb migrate`.
#[derive(Subcommand, Debug)]
pub enum MigrateSubcommand {
    /// Detect and convert treb.toml v1 → v2
    ///
    /// Reads the existing `treb.toml`, detects its format version, and writes
    /// an equivalent v2 configuration. Use `--dry-run` to preview the result
    /// without modifying any files.
    Config {
        /// Simulate execution without making changes
        #[arg(long)]
        dry_run: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
        /// Skip confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
        /// Remove [profile.*.treb.*] sections from foundry.toml after migration
        #[arg(long)]
        cleanup_foundry: bool,
    },
    /// Apply versioned registry schema migrations
    ///
    /// Runs all pending schema migrations on the deployment registry. Use
    /// `--dry-run` to list pending migrations without applying them.
    Registry {
        /// Simulate execution without making changes
        #[arg(long)]
        dry_run: bool,
    },
}

// ── run ───────────────────────────────────────────────────────────────────────

/// Entry point for `treb migrate`.
pub async fn run(subcommand: MigrateSubcommand) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;

    if !cwd.join("foundry.toml").exists() {
        bail!("no foundry.toml found in {}\n\nRun `forge init`, then `treb init`.", cwd.display());
    }

    match subcommand {
        MigrateSubcommand::Config { dry_run, json, yes, cleanup_foundry } => {
            run_config(&cwd, dry_run, json, yes, cleanup_foundry).await
        }
        MigrateSubcommand::Registry { dry_run } => run_registry(&cwd, dry_run).await,
    }
}

// ── run_config ────────────────────────────────────────────────────────────────

async fn run_config(
    project_root: &Path,
    dry_run: bool,
    json: bool,
    yes: bool,
    cleanup_foundry: bool,
) -> anyhow::Result<()> {
    if !json {
        output::print_stage("\u{1f50d}", "Detecting config format...");
    }

    let format = detect_treb_config_format(project_root);
    let treb_toml = project_root.join("treb.toml");

    match format {
        TrebConfigFormat::None => {
            // Check for deprecated foundry.toml senders as a migration path.
            let foundry_senders = extract_treb_senders_from_foundry(project_root, "default");
            if foundry_senders.is_empty() {
                bail!("no treb.toml found in {}", project_root.display());
            }

            if !json {
                eprintln!(
                    "{}",
                    output::format_warning_banner(
                        "\u{26a0}\u{fe0f}",
                        "No treb.toml found \u{2014} migrating senders from foundry.toml (deprecated location)."
                    )
                );
                output::print_stage("\u{1f504}", "Converting foundry.toml senders to v2 config...");
            }

            // Create a v2 config from foundry senders alone.
            let v2 = convert_foundry_senders_to_v2(foundry_senders);
            let v2_toml = serialize_treb_config_v2(&v2).context("failed to serialize v2 config")?;

            return write_or_print_v2(
                project_root,
                &treb_toml,
                &v2_toml,
                dry_run,
                json,
                false,
                yes,
                cleanup_foundry,
                Some("foundry"),
            )
            .await;
        }
        TrebConfigFormat::V2 => {
            if json {
                output::print_json(&serde_json::json!({
                    "status": "already_v2",
                    "message": "treb.toml is already v2 format"
                }))?;
            } else {
                println!("treb.toml is already v2 format — nothing to migrate.");
            }
            return Ok(());
        }
        TrebConfigFormat::V1 => {}
    }

    // Load v1 config.
    let v1 = load_treb_config_v1(&treb_toml)
        .map_err(|e| anyhow::anyhow!("failed to load v1 config: {e}"))?;

    // Also extract any senders from foundry.toml (deprecated location).
    let foundry_senders = extract_treb_senders_from_foundry(project_root, "default");

    if !json {
        output::print_stage("\u{1f504}", "Converting v1 config to v2...");
    }

    // Convert v1 → v2, merging in foundry senders as additional accounts.
    let v2 = convert_v1_to_v2(&v1, &foundry_senders);
    let v2_toml = serialize_treb_config_v2(&v2).context("failed to serialize v2 config")?;

    write_or_print_v2(
        project_root,
        &treb_toml,
        &v2_toml,
        dry_run,
        json,
        true,
        yes,
        cleanup_foundry,
        None,
    )
    .await
}

/// Write v2 TOML to `treb.toml` (with backup) or print it to stdout (dry-run).
#[allow(clippy::too_many_arguments)]
async fn write_or_print_v2(
    project_root: &Path,
    treb_toml: &Path,
    v2_toml: &str,
    dry_run: bool,
    json: bool,
    treb_toml_existed: bool,
    yes: bool,
    cleanup_foundry: bool,
    source: Option<&str>,
) -> anyhow::Result<()> {
    write_or_print_v2_with_prompt(
        project_root,
        treb_toml,
        v2_toml,
        dry_run,
        json,
        treb_toml_existed,
        yes,
        cleanup_foundry,
        source,
        std::io::stdin().is_terminal(),
        crate::ui::prompt::confirm,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn write_or_print_v2_with_prompt<F>(
    project_root: &Path,
    treb_toml: &Path,
    v2_toml: &str,
    dry_run: bool,
    json: bool,
    treb_toml_existed: bool,
    yes: bool,
    cleanup_foundry: bool,
    source: Option<&str>,
    stdin_is_tty: bool,
    confirm_prompt: F,
) -> anyhow::Result<()>
where
    F: Fn(&str, bool) -> bool,
{
    if dry_run {
        if json {
            let mut obj = serde_json::json!({
                "dryRun": true,
                "v2Content": v2_toml
            });
            if let Some(src) = source {
                obj.as_object_mut().unwrap().insert("source".to_string(), serde_json::json!(src));
            }
            output::print_json(&obj)?;
        } else {
            println!("{v2_toml}");
        }
        return Ok(());
    }

    // Warn when treb.toml already exists (Go: migrate.go:126-138)
    if treb_toml_existed && !json {
        if yes || !stdin_is_tty {
            // Non-interactive: plain text to stderr (Go: fmt.Fprintln(os.Stderr, ...))
            eprintln!("Warning: treb.toml already exists and will be overwritten.");
        } else {
            // Interactive: yellow warning to stderr + overwrite confirmation prompt
            let warning = "Warning: treb.toml already exists.";
            eprintln!("{}", styled(warning, color::WARNING));
            if !confirm_prompt("Overwrite existing treb.toml?", false) {
                println!("Migration cancelled.");
                return Ok(());
            }
        }
    }

    // Interactive preview + confirmation (unless --yes, --json, or non-TTY).
    if !yes && !json && stdin_is_tty {
        println!("Generated treb.toml:");
        println!();
        println!("{v2_toml}");
        if !confirm_prompt("Write this to treb.toml?", false) {
            println!("Migration cancelled.");
            return Ok(());
        }
    }

    // Write backup if treb.toml already exists.
    let backup_path = if treb_toml_existed && treb_toml.exists() {
        if !json {
            output::print_stage("\u{1f4be}", "Creating backup...");
        }
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
        let path = project_root.join(format!("treb.toml.bak-{ts}"));
        std::fs::copy(treb_toml, &path)
            .with_context(|| format!("failed to create backup at {}", path.display()))?;
        Some(path)
    } else {
        None
    };

    // Write v2 content.
    std::fs::write(treb_toml, v2_toml)
        .with_context(|| format!("failed to write {}", treb_toml.display()))?;

    // Clean up foundry.toml treb sections if requested (ignored in dry-run, which returns early
    // above).
    let foundry_cleanup_path =
        if cleanup_foundry { cleanup_foundry_treb_sections(project_root, json)? } else { None };

    if json {
        let mut obj = serde_json::json!({
            "status": "migrated",
            "backupPath": backup_path.as_ref().map(|p| p.display().to_string()),
            "foundryCleanupBackup": foundry_cleanup_path.as_ref().map(|p| p.display().to_string()),
        });
        if let Some(src) = source {
            obj.as_object_mut().unwrap().insert("source".to_string(), serde_json::json!(src));
        }
        output::print_json(&obj)?;
    } else {
        if let Some(bp) = &backup_path {
            println!("Backup written to: {}", bp.display());
        }
        if let Some(fp) = &foundry_cleanup_path {
            println!("Foundry backup written to: {}", fp.display());
        }

        // Green bold success message matching Go: green.Printf("✓ treb.toml written
        // successfully\n")
        let success_msg = format!("{} treb.toml written successfully", emoji::CHECK_MARK);
        println!("{}", styled(&success_msg, color::SUCCESS));

        // Green bold foundry cleanup message if cleanup was performed
        if foundry_cleanup_path.is_some() {
            let cleanup_msg = format!("{} foundry.toml cleaned up", emoji::CHECK_MARK);
            println!("{}", styled(&cleanup_msg, color::SUCCESS));
        }

        // Next steps section
        println!();
        println!("Next steps:");
        println!("  1. Review the generated treb.toml");
        if foundry_cleanup_path.is_some() {
            // Foundry was cleaned up — skip the manual removal step
            println!("  2. Run `treb config show` to verify your config is loaded correctly");
        } else {
            println!("  2. Remove [profile.*.treb.*] sections from foundry.toml");
            println!("  3. Run `treb config show` to verify your config is loaded correctly");
        }
    }

    Ok(())
}

// ── foundry.toml cleanup ─────────────────────────────────────────────────────

/// Remove `[profile.*.treb.*]` sections from foundry.toml.
///
/// Creates a backup (`foundry.toml.bak-{timestamp}`) before modifying the file.
/// Returns the backup path if sections were removed, or `None` if no treb
/// sections were found.
fn cleanup_foundry_treb_sections(
    project_root: &Path,
    json: bool,
) -> anyhow::Result<Option<std::path::PathBuf>> {
    let foundry_path = project_root.join("foundry.toml");
    let content = std::fs::read_to_string(&foundry_path)
        .with_context(|| format!("failed to read {}", foundry_path.display()))?;

    let cleaned = remove_treb_sections(&content);

    // No changes needed.
    if cleaned == content {
        return Ok(None);
    }

    if !json {
        output::print_stage("\u{1f9f9}", "Cleaning up foundry.toml...");
    }

    // Create backup before modifying.
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
    let backup_path = project_root.join(format!("foundry.toml.bak-{ts}"));
    std::fs::copy(&foundry_path, &backup_path).with_context(|| {
        format!("failed to create foundry.toml backup at {}", backup_path.display())
    })?;

    // Write cleaned content.
    std::fs::write(&foundry_path, &cleaned)
        .with_context(|| format!("failed to write {}", foundry_path.display()))?;

    Ok(Some(backup_path))
}

/// Remove all `[profile.*.treb]` and `[profile.*.treb.*]` sections (header + key/value lines)
/// from TOML content, preserving everything else.
fn remove_treb_sections(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut skipping = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if is_treb_section_header(trimmed) {
            skipping = true;
            continue;
        }

        if skipping {
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with('[') {
                // A new non-treb section starts — stop skipping.
                skipping = false;
                if !result.is_empty() && !result.ends_with("\n\n") {
                    result.push('\n');
                }
                result.push_str(line);
                result.push('\n');
                continue;
            }
            // Key-value line inside a treb section — skip it.
            continue;
        }

        // Not skipping — emit the line.
        result.push_str(line);
        result.push('\n');
    }

    // Trim trailing whitespace but keep a final newline.
    let trimmed = result.trim_end();
    if trimmed.is_empty() {
        return String::new();
    }
    format!("{trimmed}\n")
}

/// Check if a trimmed line is a `[profile.*.treb]` or `[profile.*.treb.*]` section header.
fn is_treb_section_header(trimmed: &str) -> bool {
    if !trimmed.starts_with("[profile.") {
        return false;
    }

    // Accept valid section headers with trailing inline comments, e.g.
    // `[profile.default.treb.senders.deployer] # comment`.
    let Some(close_idx) = trimmed.find(']') else {
        return false;
    };
    // Extract inner content between [ and ].
    let inner = &trimmed[1..close_idx];
    // Split by '.' and look for "treb" after "profile.<name>"
    let parts: Vec<&str> = inner.split('.').collect();
    // Must be at least: profile, <name>, treb
    parts.len() >= 3 && parts[0] == "profile" && parts[2] == "treb"
}

// ── v1 → v2 conversion ────────────────────────────────────────────────────────

/// Convert a v1 config to v2 format, optionally merging in foundry senders.
///
/// Each namespace in the v1 config becomes:
/// - An `accounts.*` entry for each sender (using `<namespace>_<role>` as the key when the sender
///   doesn't already have a matching account).
/// - A `namespace.*` entry mapping roles to account names.
///
/// Any `foundry_senders` not already present in the accounts map are added
/// as additional accounts (migration from deprecated foundry.toml sender config).
fn convert_v1_to_v2(
    v1: &treb_config::TrebFileConfigV1,
    foundry_senders: &HashMap<String, AccountConfig>,
) -> treb_config::TrebFileConfigV2 {
    use treb_config::TrebFileConfigV2;

    let mut accounts: HashMap<String, AccountConfig> = HashMap::new();
    let mut namespace: HashMap<String, NamespaceRoles> = HashMap::new();

    // First pass: collect all unique sender configs as accounts.
    // We use `<namespace>_<role>` as the account key, then de-duplicate
    // by content so identical configs share a single account entry.
    let mut sender_to_account: HashMap<(String, String), String> = HashMap::new();

    for (ns_name, ns_config) in &v1.ns {
        for (role, sender) in &ns_config.senders {
            // Use the role name as account key if it's unique, otherwise prefix with namespace.
            let account_key = if !accounts.contains_key(role) {
                role.clone()
            } else if accounts.get(role) == Some(sender) {
                // Same config → reuse existing account.
                role.clone()
            } else {
                format!("{ns_name}_{role}")
            };
            accounts.entry(account_key.clone()).or_insert_with(|| sender.clone());
            sender_to_account.insert((ns_name.clone(), role.clone()), account_key);
        }
    }

    // Second pass: build namespace entries.
    for (ns_name, ns_config) in &v1.ns {
        let mut senders_map: HashMap<String, String> = HashMap::new();
        for role in ns_config.senders.keys() {
            let account_key = sender_to_account
                .get(&(ns_name.clone(), role.clone()))
                .cloned()
                .unwrap_or_else(|| role.clone());
            senders_map.insert(role.clone(), account_key);
        }
        namespace.insert(
            ns_name.clone(),
            NamespaceRoles { profile: ns_config.profile.clone(), senders: senders_map },
        );
    }

    // Add any foundry senders not already present in accounts.
    for (name, config) in foundry_senders {
        accounts.entry(name.clone()).or_insert_with(|| config.clone());
    }

    TrebFileConfigV2 { accounts, namespace, fork: Default::default() }
}

/// Convert foundry-only sender config into a minimal v2 config with a resolvable
/// default namespace.
fn convert_foundry_senders_to_v2(
    foundry_senders: HashMap<String, AccountConfig>,
) -> treb_config::TrebFileConfigV2 {
    let senders = foundry_senders.keys().map(|name| (name.clone(), name.clone())).collect();

    let mut namespace = HashMap::new();
    namespace.insert(
        "default".to_string(),
        NamespaceRoles { profile: Some("default".to_string()), senders },
    );

    treb_config::TrebFileConfigV2 { accounts: foundry_senders, namespace, fork: Default::default() }
}

// ── run_registry ──────────────────────────────────────────────────────────────

async fn run_registry(project_root: &Path, dry_run: bool) -> anyhow::Result<()> {
    if !project_root.join(".treb").exists() {
        bail!("project not initialized — .treb/ directory not found\n\nRun `treb init` first.");
    }

    use treb_registry::{REGISTRY_FILE, io::read_json_file, types::RegistryMeta};

    output::print_stage("\u{1f50d}", "Checking registry version...");

    let registry_dir = project_root.join(REGISTRY_DIR);
    let meta_path = registry_dir.join(REGISTRY_FILE);

    let current_version = if meta_path.exists() {
        let meta: RegistryMeta = read_json_file(&meta_path)
            .map_err(|e| anyhow::anyhow!("failed to read registry.json: {e}"))?;
        meta.version
    } else {
        REGISTRY_VERSION
    };

    if current_version > REGISTRY_VERSION {
        bail!(
            "registry version {} is newer than supported version {}; please upgrade treb",
            current_version,
            REGISTRY_VERSION
        );
    }

    if current_version == REGISTRY_VERSION {
        output::print_stage(
            "\u{2705}",
            &format!("Registry is up to date (version {REGISTRY_VERSION})."),
        );
        return Ok(());
    }

    if dry_run {
        println!(
            "Registry is at version {current_version}; would migrate to version {REGISTRY_VERSION}."
        );
        return Ok(());
    }

    let report = run_migrations(&registry_dir)?;
    output::print_stage(
        "\u{2705}",
        &format!(
            "Registry migrated to version {}. Applied: {:?}",
            report.current_version, report.applied
        ),
    );
    Ok(())
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use treb_config::{SenderConfig, SenderType};

    /// Helper: write a minimal foundry.toml so run_config doesn't bail early.
    fn write_foundry_toml(dir: &Path) {
        std::fs::write(dir.join("foundry.toml"), "[profile.default]\n").unwrap();
    }

    /// Helper: write a v1 treb.toml fixture.
    fn write_v1_treb_toml(dir: &Path) {
        std::fs::write(
            dir.join("treb.toml"),
            r#"[ns.default.senders.deployer]
type = "private_key"
address = "0xDeployerAddr"
private_key = "0xDeployerKey"
"#,
        )
        .unwrap();
    }

    /// Helper: write a v2 treb.toml fixture.
    fn write_v2_treb_toml(dir: &Path) {
        std::fs::write(dir.join("treb.toml"), "[accounts.deployer]\ntype = \"private_key\"\n")
            .unwrap();
    }

    // ── serialize_v2 round-trip (using treb_config::serialize_treb_config_v2) ──

    #[test]
    fn serialize_v2_round_trips() {
        use std::collections::HashMap;
        use treb_config::{AccountConfig, NamespaceRoles, TrebFileConfigV2};

        let mut accounts = HashMap::new();
        accounts.insert(
            "deployer".to_string(),
            AccountConfig {
                type_: Some(SenderType::PrivateKey),
                private_key: Some("0xkey".to_string()),
                ..Default::default()
            },
        );
        let mut senders = HashMap::new();
        senders.insert("deployer".to_string(), "deployer".to_string());
        let mut namespace = HashMap::new();
        namespace.insert(
            "default".to_string(),
            NamespaceRoles { profile: Some("default".to_string()), senders },
        );
        let config = TrebFileConfigV2 { accounts, namespace, fork: Default::default() };

        let toml_str = serialize_treb_config_v2(&config).unwrap();
        let reparsed: treb_config::TrebFileConfigV2 = toml::from_str(&toml_str).unwrap();
        assert_eq!(reparsed.accounts["deployer"].type_, Some(SenderType::PrivateKey));
        assert_eq!(reparsed.namespace["default"].senders["deployer"], "deployer");
    }

    // ── golden test: v1 → v2 conversion ──────────────────────────────────────

    #[test]
    fn golden_v1_to_v2_conversion() {
        use std::collections::HashMap;
        use treb_config::{NamespaceConfigV1, TrebFileConfigV1};

        let mut senders = HashMap::new();
        senders.insert(
            "deployer".to_string(),
            SenderConfig {
                type_: Some(SenderType::PrivateKey),
                address: Some("0xDeployerAddr".to_string()),
                private_key: Some("0xDeployerKey".to_string()),
                ..Default::default()
            },
        );
        let mut ns = HashMap::new();
        ns.insert(
            "default".to_string(),
            NamespaceConfigV1 { profile: Some("default".to_string()), slow: None, senders },
        );
        let v1 = TrebFileConfigV1 { slow: None, ns };

        let v2 = convert_v1_to_v2(&v1, &HashMap::new());

        // Verify accounts
        assert!(v2.accounts.contains_key("deployer"), "deployer account should exist");
        let deployer = &v2.accounts["deployer"];
        assert_eq!(deployer.type_, Some(SenderType::PrivateKey));
        assert_eq!(deployer.address.as_deref(), Some("0xDeployerAddr"));

        // Verify namespace mapping
        assert!(v2.namespace.contains_key("default"), "default namespace should exist");
        let default_ns = &v2.namespace["default"];
        assert_eq!(default_ns.profile.as_deref(), Some("default"));
        assert_eq!(default_ns.senders["deployer"], "deployer");

        // Verify round-trip through TOML
        let toml_str = serialize_treb_config_v2(&v2).unwrap();
        let reparsed: treb_config::TrebFileConfigV2 = toml::from_str(&toml_str).unwrap();
        assert_eq!(reparsed.accounts["deployer"].type_, Some(SenderType::PrivateKey));
        assert_eq!(reparsed.namespace["default"].senders["deployer"], "deployer");
    }

    // ── run_config dry-run ────────────────────────────────────────────────────

    #[tokio::test]
    async fn run_config_dry_run_does_not_modify_files() {
        let dir = TempDir::new().unwrap();
        write_foundry_toml(dir.path());
        write_v1_treb_toml(dir.path());

        let original = std::fs::read_to_string(dir.path().join("treb.toml")).unwrap();

        run_config(dir.path(), true, false, false, false).await.unwrap();

        // treb.toml must be unchanged
        let after = std::fs::read_to_string(dir.path().join("treb.toml")).unwrap();
        assert_eq!(original, after);

        // No backup file should exist
        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("treb.toml.bak-"))
            .collect();
        assert!(backups.is_empty(), "dry-run should not create backup files");
    }

    // ── run_config already-v2 ─────────────────────────────────────────────────

    #[tokio::test]
    async fn run_config_already_v2_exits_without_changes() {
        let dir = TempDir::new().unwrap();
        write_foundry_toml(dir.path());
        write_v2_treb_toml(dir.path());

        let original = std::fs::read_to_string(dir.path().join("treb.toml")).unwrap();

        run_config(dir.path(), false, false, true, false).await.unwrap();

        let after = std::fs::read_to_string(dir.path().join("treb.toml")).unwrap();
        assert_eq!(original, after, "v2 config should not be modified");

        // No backup should be created
        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("treb.toml.bak-"))
            .collect();
        assert!(backups.is_empty(), "already-v2 should not create backup");
    }

    // ── run_config backup creation ─────────────────────────────────────────────

    #[tokio::test]
    async fn run_config_creates_backup_before_overwrite() {
        let dir = TempDir::new().unwrap();
        write_foundry_toml(dir.path());
        write_v1_treb_toml(dir.path());

        let original = std::fs::read_to_string(dir.path().join("treb.toml")).unwrap();

        run_config(dir.path(), false, false, true, false).await.unwrap();

        // treb.toml should now be v2
        let after = std::fs::read_to_string(dir.path().join("treb.toml")).unwrap();
        assert_ne!(original, after, "treb.toml should have been rewritten");
        let v2_format = treb_config::detect_treb_config_format(dir.path());
        assert_eq!(v2_format, treb_config::TrebConfigFormat::V2);

        // A backup file should exist with the original content
        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("treb.toml.bak-"))
            .collect();
        assert_eq!(backups.len(), 1, "exactly one backup should be created");
        let backup_content = std::fs::read_to_string(backups[0].path()).unwrap();
        assert_eq!(backup_content, original, "backup should contain original v1 content");
    }

    #[tokio::test]
    async fn write_or_print_v2_cancelled_overwrite_prompt_does_not_write() {
        let dir = TempDir::new().unwrap();
        let treb_toml = dir.path().join("treb.toml");
        write_v1_treb_toml(dir.path());
        let original = std::fs::read_to_string(&treb_toml).unwrap();

        // Decline the overwrite prompt — should cancel without writing.
        write_or_print_v2_with_prompt(
            dir.path(),
            &treb_toml,
            "[accounts.deployer]\ntype = \"private_key\"\n",
            false,
            false,
            true,
            false,
            false,
            None,
            true,
            |prompt, default| {
                assert_eq!(prompt, "Overwrite existing treb.toml?");
                assert!(!default, "overwrite confirmation must default to no");
                false
            },
        )
        .await
        .unwrap();

        let after = std::fs::read_to_string(&treb_toml).unwrap();
        assert_eq!(after, original, "declining overwrite must not write changes");

        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("treb.toml.bak-"))
            .collect();
        assert!(backups.is_empty(), "declining overwrite should not create backup files");
    }

    #[tokio::test]
    async fn write_or_print_v2_cancelled_write_prompt_does_not_write() {
        use std::cell::Cell;

        let dir = TempDir::new().unwrap();
        let treb_toml = dir.path().join("treb.toml");
        write_v1_treb_toml(dir.path());
        let original = std::fs::read_to_string(&treb_toml).unwrap();

        let call_count = Cell::new(0u32);

        // Accept the overwrite prompt, decline the write prompt.
        write_or_print_v2_with_prompt(
            dir.path(),
            &treb_toml,
            "[accounts.deployer]\ntype = \"private_key\"\n",
            false,
            false,
            true,
            false,
            false,
            None,
            true,
            |prompt, default| {
                let count = call_count.get();
                call_count.set(count + 1);
                assert!(!default, "interactive migrate confirmations must default to no");
                if count == 0 {
                    assert_eq!(prompt, "Overwrite existing treb.toml?");
                    true // accept overwrite
                } else {
                    assert_eq!(prompt, "Write this to treb.toml?");
                    false // decline write
                }
            },
        )
        .await
        .unwrap();

        let after = std::fs::read_to_string(&treb_toml).unwrap();
        assert_eq!(after, original, "declining write prompt must not write changes");

        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("treb.toml.bak-"))
            .collect();
        assert!(backups.is_empty(), "declining write prompt should not create backup files");
    }

    // ── run_config foundry sender migration ───────────────────────────────────

    #[tokio::test]
    async fn run_config_migrates_foundry_senders() {
        let dir = TempDir::new().unwrap();

        // foundry.toml with treb senders in the deprecated location
        std::fs::write(
            dir.path().join("foundry.toml"),
            r#"[profile.default]
src = "src"

[profile.default.treb.senders.ledger_signer]
type = "ledger"
address = "0xLedgerAddr"
derivation_path = "m/44'/60'/0'/0/0"
"#,
        )
        .unwrap();

        // v1 treb.toml with deployer only
        write_v1_treb_toml(dir.path());

        run_config(dir.path(), false, false, true, false).await.unwrap();

        // Parse the resulting v2 treb.toml
        let v2 = treb_config::load_treb_config_v2(&dir.path().join("treb.toml")).unwrap();

        // Should have deployer from v1 config
        assert!(v2.accounts.contains_key("deployer"), "deployer should be present");

        // Should also have ledger_signer from foundry.toml
        assert!(
            v2.accounts.contains_key("ledger_signer"),
            "ledger_signer from foundry.toml should be migrated"
        );
        assert_eq!(v2.accounts["ledger_signer"].type_, Some(SenderType::Ledger));
    }

    // ── run_migrations ────────────────────────────────────────────────────────

    #[test]
    fn run_migrations_on_uptodate_registry_returns_empty_report() {
        let dir = TempDir::new().unwrap();
        // Create foundry.toml + .treb/
        std::fs::write(dir.path().join("foundry.toml"), "[profile.default]\n").unwrap();
        let _ = treb_registry::Registry::init(dir.path()).unwrap();

        let registry_dir = dir.path().join(treb_registry::REGISTRY_DIR);
        let report = run_migrations(&registry_dir).unwrap();
        assert!(report.applied.is_empty());
        assert_eq!(report.current_version, REGISTRY_VERSION);
    }

    #[test]
    fn run_migrations_newer_version_returns_error() {
        use treb_registry::{
            REGISTRY_DIR, REGISTRY_FILE, io::write_json_file, types::RegistryMeta,
        };

        let dir = TempDir::new().unwrap();
        let registry_dir = dir.path().join(REGISTRY_DIR);
        std::fs::create_dir_all(&registry_dir).unwrap();

        let mut meta = RegistryMeta::new();
        meta.version = REGISTRY_VERSION + 1;
        write_json_file(&registry_dir.join(REGISTRY_FILE), &meta).unwrap();

        let result = run_migrations(&registry_dir);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("newer than supported"), "got: {msg}");
    }

    #[test]
    fn already_v2_detection() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("treb.toml"),
            "[accounts.deployer]\ntype = \"private_key\"\n",
        )
        .unwrap();
        let format = treb_config::detect_treb_config_format(dir.path());
        assert_eq!(format, treb_config::TrebConfigFormat::V2);
    }

    // ── remove_treb_sections ─────────────────────────────────────────────────

    #[test]
    fn remove_treb_sections_removes_treb_sections() {
        let input = r#"[profile.default]
src = "src"
out = "out"

[profile.default.treb.senders.deployer]
type = "private_key"
address = "0xDeployerAddr"

[profile.default.treb.senders.ledger_signer]
type = "ledger"
address = "0xLedgerAddr"

[rpc_endpoints]
mainnet = "https://eth.example.com"
"#;
        let result = remove_treb_sections(input);
        assert!(!result.contains("[profile.default.treb"), "treb sections should be removed");
        assert!(result.contains("[profile.default]"), "profile.default should remain");
        assert!(result.contains("src = \"src\""), "profile.default keys should remain");
        assert!(result.contains("[rpc_endpoints]"), "rpc_endpoints should remain");
        assert!(result.contains("mainnet"), "rpc_endpoints keys should remain");
    }

    #[test]
    fn remove_treb_sections_no_treb_is_noop() {
        let input = "[profile.default]\nsrc = \"src\"\n";
        let result = remove_treb_sections(input);
        assert_eq!(result, input);
    }

    #[test]
    fn remove_treb_sections_removes_bare_treb_section() {
        let input = r#"[profile.default]
src = "src"

[profile.default.treb]
some_key = "value"

[rpc_endpoints]
mainnet = "https://eth.example.com"
"#;
        let result = remove_treb_sections(input);
        assert!(!result.contains("[profile.default.treb]"), "bare treb section should be removed");
        assert!(!result.contains("some_key"), "treb keys should be removed");
        assert!(result.contains("[rpc_endpoints]"), "other sections should remain");
    }

    // ── cleanup_foundry_treb_sections ────────────────────────────────────────

    #[test]
    fn cleanup_foundry_creates_backup_and_removes_sections() {
        let dir = TempDir::new().unwrap();
        let foundry_content = r#"[profile.default]
src = "src"

[profile.default.treb.senders.deployer]
type = "private_key"
address = "0xAddr"

[rpc_endpoints]
mainnet = "https://eth.example.com"
"#;
        std::fs::write(dir.path().join("foundry.toml"), foundry_content).unwrap();

        let result = cleanup_foundry_treb_sections(dir.path(), false).unwrap();
        assert!(result.is_some(), "should return backup path when sections removed");

        // Verify backup exists with original content
        let backup_path = result.unwrap();
        assert!(backup_path.exists(), "backup file should exist");
        let backup_content = std::fs::read_to_string(&backup_path).unwrap();
        assert_eq!(backup_content, foundry_content, "backup should contain original content");

        // Verify foundry.toml was cleaned
        let cleaned = std::fs::read_to_string(dir.path().join("foundry.toml")).unwrap();
        assert!(!cleaned.contains("[profile.default.treb"), "treb sections should be removed");
        assert!(cleaned.contains("[profile.default]"), "non-treb sections should remain");
        assert!(cleaned.contains("[rpc_endpoints]"), "rpc_endpoints should remain");
    }

    #[test]
    fn cleanup_foundry_removes_treb_sections_with_inline_header_comments() {
        let dir = TempDir::new().unwrap();
        let foundry_content = r#"[profile.default]
src = "src"

[profile.default.treb.senders.deployer] # migrated sender
type = "private_key"
address = "0xAddr"

[rpc_endpoints]
mainnet = "https://eth.example.com"
"#;
        std::fs::write(dir.path().join("foundry.toml"), foundry_content).unwrap();

        let result = cleanup_foundry_treb_sections(dir.path(), false).unwrap();
        assert!(result.is_some(), "should return backup path when sections removed");

        let cleaned = std::fs::read_to_string(dir.path().join("foundry.toml")).unwrap();
        assert!(!cleaned.contains("[profile.default.treb"), "treb sections should be removed");
        assert!(cleaned.contains("[profile.default]"), "non-treb sections should remain");
        assert!(cleaned.contains("[rpc_endpoints]"), "rpc_endpoints should remain");
    }

    #[test]
    fn cleanup_foundry_noop_when_no_treb_sections() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("foundry.toml"), "[profile.default]\nsrc = \"src\"\n")
            .unwrap();

        let result = cleanup_foundry_treb_sections(dir.path(), false).unwrap();
        assert!(result.is_none(), "should return None when no treb sections to remove");

        // No backup file should exist
        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("foundry.toml.bak-"))
            .collect();
        assert!(backups.is_empty(), "no backup should be created when nothing to clean");
    }

    // ── foundry-only migration path ──────────────────────────────────────────

    /// Helper: write a foundry.toml with treb senders (no treb.toml needed).
    fn write_foundry_toml_with_senders(dir: &Path) {
        std::fs::write(
            dir.join("foundry.toml"),
            r#"[profile.default]
src = "src"

[profile.default.treb.senders.deployer]
type = "private_key"
address = "0xDeployerAddr"
private_key = "0xDeployerKey"
"#,
        )
        .unwrap();
    }

    #[tokio::test]
    async fn run_config_foundry_only_creates_v2_config() {
        let dir = TempDir::new().unwrap();
        write_foundry_toml_with_senders(dir.path());
        // No treb.toml — foundry-only migration path

        run_config(dir.path(), false, false, true, false).await.unwrap();

        // treb.toml should now exist as v2
        let treb_toml = dir.path().join("treb.toml");
        assert!(treb_toml.exists(), "treb.toml should be created");
        let v2 = treb_config::load_treb_config_v2(&treb_toml).unwrap();
        assert!(v2.accounts.contains_key("deployer"), "deployer should be present");
        assert_eq!(v2.accounts["deployer"].type_, Some(SenderType::PrivateKey));
        assert_eq!(v2.accounts["deployer"].address.as_deref(), Some("0xDeployerAddr"));
        assert_eq!(v2.namespace["default"].profile.as_deref(), Some("default"));
        assert_eq!(
            v2.namespace["default"].senders.get("deployer").map(String::as_str),
            Some("deployer")
        );

        let (profile, senders) = treb_config::resolve_namespace_v2(&v2, "default").unwrap();
        assert_eq!(profile, "default");
        assert_eq!(senders["deployer"].address.as_deref(), Some("0xDeployerAddr"));

        // No backup should be created (treb.toml didn't exist before)
        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("treb.toml.bak-"))
            .collect();
        assert!(backups.is_empty(), "no treb.toml backup since it didn't exist");
    }

    #[tokio::test]
    async fn run_config_foundry_only_dry_run_does_not_create_file() {
        let dir = TempDir::new().unwrap();
        write_foundry_toml_with_senders(dir.path());

        run_config(dir.path(), true, false, false, false).await.unwrap();

        // treb.toml should NOT be created
        assert!(!dir.path().join("treb.toml").exists(), "dry-run should not create treb.toml");
    }

    // ── cleanup_foundry ignored in dry-run ───────────────────────────────────

    #[tokio::test]
    async fn cleanup_foundry_ignored_in_dry_run() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("foundry.toml"),
            r#"[profile.default]
src = "src"

[profile.default.treb.senders.deployer]
type = "private_key"
address = "0xAddr"
"#,
        )
        .unwrap();
        write_v1_treb_toml(dir.path());

        let original_foundry = std::fs::read_to_string(dir.path().join("foundry.toml")).unwrap();

        // dry_run=true, cleanup_foundry=true
        run_config(dir.path(), true, false, false, true).await.unwrap();

        // foundry.toml must be unchanged
        let after_foundry = std::fs::read_to_string(dir.path().join("foundry.toml")).unwrap();
        assert_eq!(original_foundry, after_foundry, "dry-run should not modify foundry.toml");
    }
}
