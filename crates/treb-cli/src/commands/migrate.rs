//! `treb migrate` command implementation.
//!
//! Handles config format detection and v1→v2 conversion (`treb migrate config`)
//! and versioned registry schema migrations (`treb migrate registry`).

use std::env;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context};
use clap::Subcommand;
use serde::Serialize;
use treb_config::{detect_treb_config_format, load_treb_config_v1, TrebConfigFormat};
use treb_registry::{snapshot_registry, REGISTRY_DIR, REGISTRY_VERSION};

use crate::output;

// ── Subcommand ────────────────────────────────────────────────────────────────

/// Subcommands for `treb migrate`.
#[derive(Subcommand, Debug)]
pub enum MigrateSubcommand {
    /// Detect and convert treb.toml v1 → v2
    Config {
        /// Print v2 TOML to stdout without modifying any files
        #[arg(long)]
        dry_run: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Apply versioned registry schema migrations
    Registry {
        /// List pending migrations without applying them
        #[arg(long)]
        dry_run: bool,
    },
}

// ── MigrationReport ────────────────────────────────────────────────────────────

/// Report returned by `run_migrations`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationReport {
    /// Version numbers of migrations that were applied.
    pub applied: Vec<u32>,
    /// The registry version after all migrations.
    pub current_version: u32,
}

// ── Serialise v2 config ───────────────────────────────────────────────────────

/// Render a `TrebFileConfigV2` as a TOML string.
///
/// Each `[accounts.<name>]` and `[namespace.<name>]` section is written
/// individually so the output is stable and readable.
pub fn serialize_treb_config_v2(config: &treb_config::TrebFileConfigV2) -> anyhow::Result<String> {
    let raw = toml::to_string_pretty(config)
        .context("failed to serialize treb.toml v2 as TOML")?;
    Ok(raw)
}

// ── run_migrations ─────────────────────────────────────────────────────────────

/// Apply all pending registry migrations in order.
///
/// Currently there are no migrations (v1 is the only version). This function
/// returns immediately with an empty report when the registry is already at
/// [`REGISTRY_VERSION`], and returns a `TrebError::Registry` error when the
/// recorded version is *newer* than the tool supports.
pub fn run_migrations(project_root: &Path) -> anyhow::Result<MigrationReport> {
    use treb_registry::io::read_json_file;
    use treb_registry::types::RegistryMeta;
    use treb_registry::REGISTRY_FILE;

    let registry_dir = project_root.join(REGISTRY_DIR);
    let meta_path = registry_dir.join(REGISTRY_FILE);

    let current_version = if meta_path.exists() {
        let meta: RegistryMeta = read_json_file(&meta_path)
            .map_err(|e| anyhow::anyhow!("failed to read registry.json: {e}"))?;
        meta.version
    } else {
        // No registry.json — treat as current version (nothing to migrate).
        REGISTRY_VERSION
    };

    if current_version > REGISTRY_VERSION {
        bail!(
            "registry version {} is newer than supported version {}; please upgrade treb",
            current_version,
            REGISTRY_VERSION
        );
    }

    // Static migration table: (target_version, migration_fn).
    // Currently empty — v1 is the initial version.
    type MigrationFn = fn(&Path) -> anyhow::Result<()>;
    static MIGRATIONS: &[(u32, MigrationFn)] = &[];

    let mut applied = Vec::new();
    let mut version = current_version;

    for &(target_version, migration_fn) in MIGRATIONS {
        if target_version > version {
            // Backup before each migration step.
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let backup_dir =
                registry_dir.join(format!("backups/migrate-v{target_version}-{ts}"));
            snapshot_registry(&registry_dir, &backup_dir).with_context(|| {
                format!(
                    "failed to create migration backup at {}",
                    backup_dir.display()
                )
            })?;

            migration_fn(project_root)?;

            // Bump version in registry.json.
            use treb_registry::io::write_json_file;
            let mut meta: RegistryMeta = read_json_file(&meta_path)
                .map_err(|e| anyhow::anyhow!("failed to read registry.json: {e}"))?;
            meta.version = target_version;
            write_json_file(&meta_path, &meta)
                .map_err(|e| anyhow::anyhow!("failed to write registry.json: {e}"))?;

            version = target_version;
            applied.push(target_version);
        }
    }

    Ok(MigrationReport {
        applied,
        current_version: version,
    })
}

// ── run ───────────────────────────────────────────────────────────────────────

/// Entry point for `treb migrate`.
pub async fn run(subcommand: MigrateSubcommand) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;

    if !cwd.join("foundry.toml").exists() {
        bail!(
            "no foundry.toml found in {}\n\nRun `forge init`, then `treb init`.",
            cwd.display()
        );
    }

    match subcommand {
        MigrateSubcommand::Config { dry_run, json } => run_config(&cwd, dry_run, json).await,
        MigrateSubcommand::Registry { dry_run } => run_registry(&cwd, dry_run).await,
    }
}

// ── run_config ────────────────────────────────────────────────────────────────

async fn run_config(project_root: &Path, dry_run: bool, json: bool) -> anyhow::Result<()> {
    let format = detect_treb_config_format(project_root);
    let treb_toml = project_root.join("treb.toml");

    match format {
        TrebConfigFormat::None => {
            bail!("no treb.toml found in {}", project_root.display());
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

    // Convert v1 → v2.
    let v2 = convert_v1_to_v2(&v1);
    let v2_toml = serialize_treb_config_v2(&v2)?;

    if dry_run {
        if json {
            output::print_json(&serde_json::json!({
                "dryRun": true,
                "v2Content": v2_toml
            }))?;
        } else {
            println!("{v2_toml}");
        }
        return Ok(());
    }

    // Write backup.
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let backup_path = project_root.join(format!("treb.toml.bak-{ts}"));
    std::fs::copy(&treb_toml, &backup_path).with_context(|| {
        format!("failed to create backup at {}", backup_path.display())
    })?;

    // Overwrite treb.toml with v2 content.
    std::fs::write(&treb_toml, &v2_toml)
        .with_context(|| format!("failed to write {}", treb_toml.display()))?;

    if json {
        output::print_json(&serde_json::json!({
            "status": "migrated",
            "backupPath": backup_path.display().to_string(),
        }))?;
    } else {
        println!("Migration complete.");
        println!("Backup written to: {}", backup_path.display());
        println!("treb.toml updated to v2 format.");
    }

    Ok(())
}

// ── v1 → v2 conversion ────────────────────────────────────────────────────────

/// Convert a v1 config to v2 format.
///
/// Each namespace in the v1 config becomes:
/// - An `accounts.*` entry for each sender (using `<namespace>_<role>` as the key
///   when the sender doesn't already have a matching account).
/// - A `namespace.*` entry mapping roles to account names.
fn convert_v1_to_v2(
    v1: &treb_config::TrebFileConfigV1,
) -> treb_config::TrebFileConfigV2 {
    use std::collections::HashMap;
    use treb_config::{AccountConfig, NamespaceRoles, TrebFileConfigV2};

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
            NamespaceRoles {
                profile: ns_config.profile.clone(),
                senders: senders_map,
            },
        );
    }

    TrebFileConfigV2 {
        accounts,
        namespace,
        fork: Default::default(),
    }
}

// ── run_registry ──────────────────────────────────────────────────────────────

async fn run_registry(project_root: &Path, dry_run: bool) -> anyhow::Result<()> {
    if !project_root.join(".treb").exists() {
        bail!(
            "project not initialized — .treb/ directory not found\n\nRun `treb init` first."
        );
    }

    use treb_registry::io::read_json_file;
    use treb_registry::types::RegistryMeta;
    use treb_registry::REGISTRY_FILE;

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
        println!("Registry is up to date (version {REGISTRY_VERSION}).");
        return Ok(());
    }

    if dry_run {
        println!(
            "Registry is at version {current_version}; would migrate to version {REGISTRY_VERSION}."
        );
        return Ok(());
    }

    let report = run_migrations(project_root)?;
    println!(
        "Registry migrated to version {}. Applied: {:?}",
        report.current_version, report.applied
    );
    Ok(())
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use treb_config::{SenderConfig, SenderType, TrebFileConfigV1};

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
            NamespaceRoles {
                profile: Some("default".to_string()),
                senders,
            },
        );
        let config = TrebFileConfigV2 {
            accounts,
            namespace,
            fork: Default::default(),
        };

        let toml_str = serialize_treb_config_v2(&config).unwrap();
        let reparsed: treb_config::TrebFileConfigV2 = toml::from_str(&toml_str).unwrap();
        assert_eq!(reparsed.accounts["deployer"].type_, Some(SenderType::PrivateKey));
        assert_eq!(reparsed.namespace["default"].senders["deployer"], "deployer");
    }

    #[test]
    fn run_migrations_on_uptodate_registry_returns_empty_report() {
        let dir = TempDir::new().unwrap();
        // Create foundry.toml + .treb/
        std::fs::write(dir.path().join("foundry.toml"), "[profile.default]\n").unwrap();
        let _ = treb_registry::Registry::init(dir.path()).unwrap();

        let report = run_migrations(dir.path()).unwrap();
        assert!(report.applied.is_empty());
        assert_eq!(report.current_version, REGISTRY_VERSION);
    }

    #[test]
    fn run_migrations_newer_version_returns_error() {
        use treb_registry::io::write_json_file;
        use treb_registry::types::RegistryMeta;
        use treb_registry::{REGISTRY_DIR, REGISTRY_FILE};

        let dir = TempDir::new().unwrap();
        let registry_dir = dir.path().join(REGISTRY_DIR);
        std::fs::create_dir_all(&registry_dir).unwrap();

        let mut meta = RegistryMeta::new();
        meta.version = REGISTRY_VERSION + 1;
        write_json_file(&registry_dir.join(REGISTRY_FILE), &meta).unwrap();

        let result = run_migrations(dir.path());
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
        let format = detect_treb_config_format(dir.path());
        assert_eq!(format, TrebConfigFormat::V2);
    }
}
