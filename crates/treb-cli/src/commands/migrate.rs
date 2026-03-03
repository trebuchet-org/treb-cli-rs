//! `treb migrate` command implementation.
//!
//! Handles config format detection and v1→v2 conversion (`treb migrate config`)
//! and versioned registry schema migrations (`treb migrate registry`).

use std::collections::HashMap;
use std::env;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context};
use clap::Subcommand;
use treb_config::{
    detect_treb_config_format, extract_treb_senders_from_foundry, load_treb_config_v1,
    serialize_treb_config_v2, AccountConfig, TrebConfigFormat,
};
use treb_registry::{run_migrations, REGISTRY_DIR, REGISTRY_VERSION};

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
            // Check for deprecated foundry.toml senders as a migration path.
            let foundry_senders =
                extract_treb_senders_from_foundry(project_root, "default");
            if foundry_senders.is_empty() {
                bail!("no treb.toml found in {}", project_root.display());
            }

            // Create a v2 config from foundry senders alone.
            let v2 = treb_config::TrebFileConfigV2 {
                accounts: foundry_senders,
                namespace: Default::default(),
                fork: Default::default(),
            };
            let v2_toml =
                serialize_treb_config_v2(&v2).context("failed to serialize v2 config")?;

            return write_or_print_v2(
                project_root,
                &treb_toml,
                &v2_toml,
                dry_run,
                json,
                false,
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

    // Convert v1 → v2, merging in foundry senders as additional accounts.
    let v2 = convert_v1_to_v2(&v1, &foundry_senders);
    let v2_toml = serialize_treb_config_v2(&v2).context("failed to serialize v2 config")?;

    write_or_print_v2(project_root, &treb_toml, &v2_toml, dry_run, json, true).await
}

/// Write v2 TOML to `treb.toml` (with backup) or print it to stdout (dry-run).
async fn write_or_print_v2(
    project_root: &Path,
    treb_toml: &Path,
    v2_toml: &str,
    dry_run: bool,
    json: bool,
    treb_toml_existed: bool,
) -> anyhow::Result<()> {
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

    // Write backup if treb.toml already exists.
    let backup_path = if treb_toml_existed && treb_toml.exists() {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
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

    if json {
        output::print_json(&serde_json::json!({
            "status": "migrated",
            "backupPath": backup_path.as_ref().map(|p| p.display().to_string()),
        }))?;
    } else {
        println!("Migration complete.");
        if let Some(bp) = &backup_path {
            println!("Backup written to: {}", bp.display());
        }
        println!("treb.toml updated to v2 format.");
    }

    Ok(())
}

// ── v1 → v2 conversion ────────────────────────────────────────────────────────

/// Convert a v1 config to v2 format, optionally merging in foundry senders.
///
/// Each namespace in the v1 config becomes:
/// - An `accounts.*` entry for each sender (using `<namespace>_<role>` as the key
///   when the sender doesn't already have a matching account).
/// - A `namespace.*` entry mapping roles to account names.
///
/// Any `foundry_senders` not already present in the accounts map are added
/// as additional accounts (migration from deprecated foundry.toml sender config).
fn convert_v1_to_v2(
    v1: &treb_config::TrebFileConfigV1,
    foundry_senders: &HashMap<String, AccountConfig>,
) -> treb_config::TrebFileConfigV2 {
    use treb_config::{NamespaceRoles, TrebFileConfigV2};

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

    // Add any foundry senders not already present in accounts.
    for (name, config) in foundry_senders {
        accounts.entry(name.clone()).or_insert_with(|| config.clone());
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

    let report = run_migrations(&registry_dir)?;
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
        std::fs::write(
            dir.join("treb.toml"),
            "[accounts.deployer]\ntype = \"private_key\"\n",
        )
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

    // ── golden test: v1 → v2 conversion ──────────────────────────────────────

    #[test]
    fn golden_v1_to_v2_conversion() {
        use treb_config::{NamespaceConfigV1, TrebFileConfigV1};
        use std::collections::HashMap;

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
            NamespaceConfigV1 {
                profile: Some("default".to_string()),
                slow: None,
                senders,
            },
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

        run_config(dir.path(), true, false).await.unwrap();

        // treb.toml must be unchanged
        let after = std::fs::read_to_string(dir.path().join("treb.toml")).unwrap();
        assert_eq!(original, after);

        // No backup file should exist
        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("treb.toml.bak-")
            })
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

        run_config(dir.path(), false, false).await.unwrap();

        let after = std::fs::read_to_string(dir.path().join("treb.toml")).unwrap();
        assert_eq!(original, after, "v2 config should not be modified");

        // No backup should be created
        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("treb.toml.bak-")
            })
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

        run_config(dir.path(), false, false).await.unwrap();

        // treb.toml should now be v2
        let after = std::fs::read_to_string(dir.path().join("treb.toml")).unwrap();
        assert_ne!(original, after, "treb.toml should have been rewritten");
        let v2_format = treb_config::detect_treb_config_format(dir.path());
        assert_eq!(v2_format, treb_config::TrebConfigFormat::V2);

        // A backup file should exist with the original content
        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("treb.toml.bak-")
            })
            .collect();
        assert_eq!(backups.len(), 1, "exactly one backup should be created");
        let backup_content = std::fs::read_to_string(backups[0].path()).unwrap();
        assert_eq!(backup_content, original, "backup should contain original v1 content");
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

        run_config(dir.path(), false, false).await.unwrap();

        // Parse the resulting v2 treb.toml
        let v2 = treb_config::load_treb_config_v2(&dir.path().join("treb.toml")).unwrap();

        // Should have deployer from v1 config
        assert!(v2.accounts.contains_key("deployer"), "deployer should be present");

        // Should also have ledger_signer from foundry.toml
        assert!(
            v2.accounts.contains_key("ledger_signer"),
            "ledger_signer from foundry.toml should be migrated"
        );
        assert_eq!(
            v2.accounts["ledger_signer"].type_,
            Some(SenderType::Ledger)
        );
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
        use treb_registry::io::write_json_file;
        use treb_registry::types::RegistryMeta;
        use treb_registry::{REGISTRY_DIR, REGISTRY_FILE};

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
}
