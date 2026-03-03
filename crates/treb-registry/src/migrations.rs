//! Versioned registry schema migration runner.
//!
//! Applies forward-only migrations in version order, creating a timestamped
//! backup before each step for safe rollback.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use treb_core::TrebError;

use crate::io::{read_json_file, write_json_file};
use crate::store::fork_state::snapshot_registry;
use crate::types::RegistryMeta;
use crate::{REGISTRY_FILE, REGISTRY_VERSION};

// ── MigrationReport ───────────────────────────────────────────────────────────

/// Report returned by [`run_migrations`].
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationReport {
    /// Version numbers of migrations that were applied in this run.
    pub applied: Vec<u32>,
    /// The registry version after all migrations have been applied.
    pub current_version: u32,
}

// ── Migration functions ────────────────────────────────────────────────────────

/// Type alias for a migration function.
type MigrationFn = fn(&Path) -> Result<(), TrebError>;

/// All known registry migrations, ordered by target version.
///
/// Each entry is `(target_version, migration_fn)`. The runner applies all
/// entries where `target_version > current_version` in ascending order and
/// creates a timestamped backup before each one.
static MIGRATIONS: &[(u32, MigrationFn)] = &[(1, migrate_v0_to_v1), (2, migrate_v1_to_v2)];

/// v0 → v1: first release of the versioned registry format.
///
/// No data transformations are required — v1 is the initial versioned release.
/// The runner's post-step version bump in `registry.json` is the only change.
fn migrate_v0_to_v1(_registry_dir: &Path) -> Result<(), TrebError> {
    Ok(())
}

/// Old-to-new filename renames applied by the v1 → v2 migration.
const FILE_RENAMES: &[(&str, &str)] = &[
    ("safe_txs.json", "safe-txs.json"),
    ("governor_proposals.json", "governor-txs.json"),
    ("fork-state.json", "fork.json"),
];

/// Rename `old` to `new` inside `dir`, skipping if `old` doesn't exist or
/// `new` already exists (no clobber).
fn rename_if_needed(dir: &Path, old: &str, new: &str) -> Result<(), TrebError> {
    let old_path = dir.join(old);
    let new_path = dir.join(new);
    if old_path.exists() && !new_path.exists() {
        fs::rename(&old_path, &new_path).map_err(|e| {
            TrebError::Registry(format!(
                "failed to rename {} → {} in {}: {e}",
                old,
                new,
                dir.display()
            ))
        })?;
    }
    Ok(())
}

/// v1 → v2: rename registry files to match Go CLI filenames.
///
/// Renames:
/// - `safe_txs.json` → `safe-txs.json`
/// - `governor_proposals.json` → `governor-txs.json`
/// - `fork-state.json` → `fork.json`
///
/// Also walks `.treb/snapshots/*/` to rename files inside fork snapshot
/// directories. Skips renames where the old file doesn't exist or the new
/// file already exists (no clobber).
fn migrate_v1_to_v2(registry_dir: &Path) -> Result<(), TrebError> {
    // Rename files in the top-level registry directory.
    for &(old, new) in FILE_RENAMES {
        rename_if_needed(registry_dir, old, new)?;
    }

    // Rename files inside fork snapshot directories.
    let snapshots_dir = registry_dir.join("snapshots");
    if snapshots_dir.is_dir() {
        for entry in fs::read_dir(&snapshots_dir).map_err(|e| {
            TrebError::Registry(format!(
                "failed to read snapshots directory {}: {e}",
                snapshots_dir.display()
            ))
        })? {
            let entry = entry.map_err(|e| {
                TrebError::Registry(format!("failed to read snapshot entry: {e}"))
            })?;
            let path = entry.path();
            if path.is_dir() {
                for &(old, new) in FILE_RENAMES {
                    rename_if_needed(&path, old, new)?;
                }
            }
        }
    }

    Ok(())
}

// ── run_migrations ─────────────────────────────────────────────────────────────

/// Apply all pending registry migrations in order.
///
/// # Arguments
/// * `registry_dir` — path to the `.treb/` registry directory.
///
/// # Errors
/// Returns [`TrebError::Registry`] when the recorded version is *newer* than
/// [`REGISTRY_VERSION`] (the tool is out of date).
pub fn run_migrations(registry_dir: &Path) -> Result<MigrationReport, TrebError> {
    let meta_path = registry_dir.join(REGISTRY_FILE);

    let current_version = if meta_path.exists() {
        let meta: RegistryMeta = read_json_file(&meta_path)?;
        meta.version
    } else {
        // No registry.json yet — treat as current version (nothing to migrate).
        REGISTRY_VERSION
    };

    if current_version > REGISTRY_VERSION {
        return Err(TrebError::Registry(format!(
            "registry version {current_version} is newer than supported version \
             {REGISTRY_VERSION}; please upgrade treb"
        )));
    }

    let mut applied = Vec::new();
    let mut version = current_version;

    for &(target_version, migration_fn) in MIGRATIONS {
        if target_version > version {
            // Create backup before mutating.
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let backup_dir =
                registry_dir.join(format!("backups/migrate-v{target_version}-{ts}"));
            snapshot_registry(registry_dir, &backup_dir).map_err(|e| {
                TrebError::Registry(format!(
                    "failed to create migration backup at {}: {e}",
                    backup_dir.display()
                ))
            })?;

            migration_fn(registry_dir)?;

            // Bump version in registry.json (create it if missing).
            let mut meta = if meta_path.exists() {
                read_json_file::<RegistryMeta>(&meta_path)?
            } else {
                RegistryMeta::new()
            };
            meta.version = target_version;
            write_json_file(&meta_path, &meta)?;

            version = target_version;
            applied.push(target_version);
        }
    }

    Ok(MigrationReport {
        applied,
        current_version: version,
    })
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    use crate::{Registry, REGISTRY_DIR};

    fn init_registry_dir(dir: &Path) -> std::path::PathBuf {
        Registry::init(dir).unwrap();
        dir.join(REGISTRY_DIR)
    }

    #[test]
    fn up_to_date_registry_returns_empty_applied() {
        let dir = TempDir::new().unwrap();
        let registry_dir = init_registry_dir(dir.path());

        let report = run_migrations(&registry_dir).unwrap();
        assert!(report.applied.is_empty(), "no migrations expected: {:?}", report.applied);
        assert_eq!(report.current_version, REGISTRY_VERSION);
    }

    #[test]
    fn version_0_fixture_applies_pending_migrations() {
        let dir = TempDir::new().unwrap();
        let registry_dir = init_registry_dir(dir.path());

        // Downgrade to version 0.
        let meta_path = registry_dir.join(REGISTRY_FILE);
        let mut meta: RegistryMeta = read_json_file(&meta_path).unwrap();
        meta.version = 0;
        write_json_file(&meta_path, &meta).unwrap();

        let report = run_migrations(&registry_dir).unwrap();
        assert!(!report.applied.is_empty(), "at least one migration should be applied");
        assert_eq!(report.current_version, REGISTRY_VERSION);

        // registry.json version must be updated on disk.
        let updated: RegistryMeta = read_json_file(&meta_path).unwrap();
        assert_eq!(updated.version, REGISTRY_VERSION);
    }

    #[test]
    fn version_0_migration_creates_backup() {
        let dir = TempDir::new().unwrap();
        let registry_dir = init_registry_dir(dir.path());

        let meta_path = registry_dir.join(REGISTRY_FILE);
        let mut meta: RegistryMeta = read_json_file(&meta_path).unwrap();
        meta.version = 0;
        write_json_file(&meta_path, &meta).unwrap();

        run_migrations(&registry_dir).unwrap();

        // Backup directory must exist at .treb/backups/migrate-v1-<ts>/.
        let backups_dir = registry_dir.join("backups");
        assert!(backups_dir.exists(), "backups/ directory should be created");
        let v1_backups: Vec<_> = std::fs::read_dir(&backups_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("migrate-v1-"))
            .collect();
        assert!(!v1_backups.is_empty(), "backup for v1 migration should exist");
    }

    #[test]
    fn newer_version_returns_registry_error() {
        let dir = TempDir::new().unwrap();
        let registry_dir = init_registry_dir(dir.path());

        let meta_path = registry_dir.join(REGISTRY_FILE);
        let mut meta: RegistryMeta = read_json_file(&meta_path).unwrap();
        meta.version = REGISTRY_VERSION + 1;
        write_json_file(&meta_path, &meta).unwrap();

        let err = run_migrations(&registry_dir).unwrap_err();
        match &err {
            TrebError::Registry(msg) => {
                assert!(msg.contains("newer than supported"), "unexpected message: {msg}");
            }
            other => panic!("expected TrebError::Registry, got: {other:?}"),
        }
    }

    #[test]
    fn migration_report_serializes_to_camel_case_json() {
        let report = MigrationReport {
            applied: vec![1],
            current_version: 1,
        };
        let json = serde_json::to_value(&report).unwrap();
        let obj = json.as_object().unwrap();
        assert!(obj.contains_key("applied"), "should have 'applied' key");
        assert!(obj.contains_key("currentVersion"), "should have 'currentVersion' key");
    }

    // ── v1 → v2 migration tests ──────────────────────────────────────────

    #[test]
    fn v1_to_v2_renames_old_files() {
        let dir = TempDir::new().unwrap();
        let registry_dir = dir.path().join(REGISTRY_DIR);
        std::fs::create_dir_all(&registry_dir).unwrap();

        // Create old-format files.
        std::fs::write(registry_dir.join("safe_txs.json"), r#"{"h1":{}}"#).unwrap();
        std::fs::write(
            registry_dir.join("governor_proposals.json"),
            r#"{"p1":{}}"#,
        )
        .unwrap();
        std::fs::write(registry_dir.join("fork-state.json"), r#"{"forks":{}}"#).unwrap();

        migrate_v1_to_v2(&registry_dir).unwrap();

        // New files exist with correct content.
        assert!(registry_dir.join("safe-txs.json").exists());
        assert!(registry_dir.join("governor-txs.json").exists());
        assert!(registry_dir.join("fork.json").exists());

        assert_eq!(
            std::fs::read_to_string(registry_dir.join("safe-txs.json")).unwrap(),
            r#"{"h1":{}}"#
        );
        assert_eq!(
            std::fs::read_to_string(registry_dir.join("governor-txs.json")).unwrap(),
            r#"{"p1":{}}"#
        );
        assert_eq!(
            std::fs::read_to_string(registry_dir.join("fork.json")).unwrap(),
            r#"{"forks":{}}"#
        );

        // Old files no longer exist.
        assert!(!registry_dir.join("safe_txs.json").exists());
        assert!(!registry_dir.join("governor_proposals.json").exists());
        assert!(!registry_dir.join("fork-state.json").exists());
    }

    #[test]
    fn v1_to_v2_skips_when_new_file_exists_no_clobber() {
        let dir = TempDir::new().unwrap();
        let registry_dir = dir.path().join(REGISTRY_DIR);
        std::fs::create_dir_all(&registry_dir).unwrap();

        // Both old and new exist — old should NOT overwrite new.
        std::fs::write(registry_dir.join("safe_txs.json"), "old-content").unwrap();
        std::fs::write(registry_dir.join("safe-txs.json"), "new-content").unwrap();

        migrate_v1_to_v2(&registry_dir).unwrap();

        // New file keeps its content (no clobber).
        assert_eq!(
            std::fs::read_to_string(registry_dir.join("safe-txs.json")).unwrap(),
            "new-content"
        );
        // Old file still exists (wasn't renamed because new already existed).
        assert!(registry_dir.join("safe_txs.json").exists());
    }

    #[test]
    fn v1_to_v2_skips_when_old_file_missing() {
        let dir = TempDir::new().unwrap();
        let registry_dir = dir.path().join(REGISTRY_DIR);
        std::fs::create_dir_all(&registry_dir).unwrap();

        // No old files present — migration should succeed silently.
        migrate_v1_to_v2(&registry_dir).unwrap();

        assert!(!registry_dir.join("safe-txs.json").exists());
        assert!(!registry_dir.join("governor-txs.json").exists());
        assert!(!registry_dir.join("fork.json").exists());
    }

    #[test]
    fn v1_to_v2_renames_files_in_snapshot_dirs() {
        let dir = TempDir::new().unwrap();
        let registry_dir = dir.path().join(REGISTRY_DIR);
        let snap_dir = registry_dir.join("snapshots/mainnet");
        std::fs::create_dir_all(&snap_dir).unwrap();

        // Create old-format files in the snapshot directory.
        std::fs::write(snap_dir.join("safe_txs.json"), r#"{"s1":{}}"#).unwrap();
        std::fs::write(snap_dir.join("governor_proposals.json"), r#"{"g1":{}}"#).unwrap();

        migrate_v1_to_v2(&registry_dir).unwrap();

        assert!(snap_dir.join("safe-txs.json").exists());
        assert!(snap_dir.join("governor-txs.json").exists());
        assert!(!snap_dir.join("safe_txs.json").exists());
        assert!(!snap_dir.join("governor_proposals.json").exists());
    }

    #[test]
    fn v1_to_v2_handles_no_snapshots_dir() {
        let dir = TempDir::new().unwrap();
        let registry_dir = dir.path().join(REGISTRY_DIR);
        std::fs::create_dir_all(&registry_dir).unwrap();

        // No snapshots/ directory — should succeed.
        migrate_v1_to_v2(&registry_dir).unwrap();
    }

    #[test]
    fn full_v1_to_v2_migration_creates_backup_and_bumps_version() {
        let dir = TempDir::new().unwrap();
        let registry_dir = init_registry_dir(dir.path());

        // Downgrade to v1 and create old-format files.
        let meta_path = registry_dir.join(REGISTRY_FILE);
        let mut meta: RegistryMeta = read_json_file(&meta_path).unwrap();
        meta.version = 1;
        write_json_file(&meta_path, &meta).unwrap();

        std::fs::write(registry_dir.join("safe_txs.json"), "{}").unwrap();
        std::fs::write(registry_dir.join("fork-state.json"), "{}").unwrap();

        let report = run_migrations(&registry_dir).unwrap();
        assert!(report.applied.contains(&2), "v2 migration should be applied");
        assert_eq!(report.current_version, REGISTRY_VERSION);

        // Verify version bumped on disk.
        let updated: RegistryMeta = read_json_file(&meta_path).unwrap();
        assert_eq!(updated.version, 2);

        // Verify files were renamed.
        assert!(registry_dir.join("safe-txs.json").exists());
        assert!(registry_dir.join("fork.json").exists());

        // Verify backup was created.
        let backups_dir = registry_dir.join("backups");
        let v2_backups: Vec<_> = std::fs::read_dir(&backups_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("migrate-v2-"))
            .collect();
        assert!(!v2_backups.is_empty(), "backup for v2 migration should exist");
    }
}
