//! Compose plan file — tracks ordered component execution and links to
//! per-script broadcast files for checkpoint/resume.
//!
//! Written to `broadcast/<compose-file>/<chain>/compose-latest.json`.
//! This is the single source of truth for compose resume — no separate
//! session-state or compose-state files needed.

use std::{
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use treb_core::error::TrebError;

/// A compose execution plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposePlan {
    /// Source compose YAML file path.
    pub compose_file: String,
    /// Hash of compose file contents for change detection.
    pub compose_hash: String,
    /// Chain ID for this execution.
    pub chain_id: u64,
    /// Ordered list of components in execution order.
    pub components: Vec<ComponentEntry>,
}

/// A single component in the compose plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentEntry {
    /// Component name from compose YAML.
    pub name: String,
    /// Execution step (1-indexed).
    pub step: usize,
    /// Script path (e.g. `script/Deploy.s.sol`).
    pub script: String,
    /// Path to the immutable Foundry broadcast file (run-<timestamp>.json),
    /// relative to project root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broadcast_file: Option<String>,
    /// Path to the immutable queued operations file
    /// (run-<timestamp>.queued.json).
    #[serde(default, alias = "deferredFile", skip_serializing_if = "Option::is_none")]
    pub queued_file: Option<String>,
    /// Component execution status.
    pub status: ComponentStatus,
}

/// Execution status for a compose component.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ComponentStatus {
    /// Not yet executed.
    Pending,
    /// Simulation complete, broadcast not started.
    Simulated,
    /// Immediate transactions broadcast, queued operations recorded.
    Broadcast,
    /// Failed during execution.
    Failed,
}

/// Resolve the compose artifact directory name from the source YAML path.
///
/// For consistency with script broadcast folders, this preserves the full file
/// name including the `.yaml`/`.yml` extension.
pub fn compose_artifact_dir_name(compose_file: &str) -> String {
    Path::new(compose_file)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "compose".to_string())
}

/// Compute the compose plan file path.
///
/// Layout: `broadcast/<compose-file>/<chain>/compose-latest.json`
pub fn compose_plan_path(project_root: &Path, compose_file: &str, chain_id: u64) -> PathBuf {
    compose_plan_latest_path(project_root, compose_file, chain_id)
}

/// Compute the mutable latest compose plan path.
pub fn compose_plan_latest_path(project_root: &Path, compose_file: &str, chain_id: u64) -> PathBuf {
    project_root
        .join("broadcast")
        .join(compose_artifact_dir_name(compose_file))
        .join(chain_id.to_string())
        .join("compose-latest.json")
}

/// Compute the immutable archived compose plan path.
pub fn compose_plan_archive_path(
    project_root: &Path,
    compose_file: &str,
    chain_id: u64,
    timestamp: u128,
) -> PathBuf {
    let latest = compose_plan_latest_path(project_root, compose_file, chain_id);
    super::broadcast_writer::timestamped_path_from_latest(&latest, timestamp)
}

/// Create a new compose plan from the execution order.
pub fn create_plan(
    compose_file: &str,
    compose_hash: &str,
    chain_id: u64,
    components: &[(String, String)], // (name, script_path)
) -> ComposePlan {
    ComposePlan {
        compose_file: compose_file.to_string(),
        compose_hash: compose_hash.to_string(),
        chain_id,
        components: components
            .iter()
            .enumerate()
            .map(|(i, (name, script))| ComponentEntry {
                name: name.clone(),
                step: i + 1,
                script: script.clone(),
                broadcast_file: None,
                queued_file: None,
                status: ComponentStatus::Pending,
            })
            .collect(),
    }
}

/// Compute a deterministic hash of compose file contents.
pub fn compute_compose_hash(contents: &str) -> String {
    let mut hasher = DefaultHasher::new();
    contents.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Save the compose plan to disk.
pub fn save_plan(project_root: &Path, plan: &ComposePlan) -> Result<PathBuf, TrebError> {
    let path = compose_plan_latest_path(project_root, &plan.compose_file, plan.chain_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            TrebError::Forge(format!(
                "failed to create compose plan directory {}: {e}",
                parent.display()
            ))
        })?;
    }

    let file = fs::File::create(&path).map_err(|e| {
        TrebError::Forge(format!("failed to create compose plan file {}: {e}", path.display()))
    })?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, plan)
        .map_err(|e| TrebError::Forge(format!("failed to serialize compose plan: {e}")))?;
    writer.flush().map_err(|e| TrebError::Forge(format!("failed to flush compose plan: {e}")))?;

    Ok(path)
}

/// Save an immutable archived compose plan snapshot after full completion.
pub fn save_plan_archive(project_root: &Path, plan: &ComposePlan) -> Result<PathBuf, TrebError> {
    let timestamp =
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let path =
        compose_plan_archive_path(project_root, &plan.compose_file, plan.chain_id, timestamp);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            TrebError::Forge(format!(
                "failed to create compose archive directory {}: {e}",
                parent.display()
            ))
        })?;
    }

    let file = fs::File::create(&path).map_err(|e| {
        TrebError::Forge(format!("failed to create compose archive file {}: {e}", path.display()))
    })?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, plan)
        .map_err(|e| TrebError::Forge(format!("failed to serialize compose plan: {e}")))?;
    writer
        .flush()
        .map_err(|e| TrebError::Forge(format!("failed to flush compose archive: {e}")))?;

    Ok(path)
}

/// Load a compose plan from disk. Returns `None` if the file doesn't exist.
pub fn load_plan(project_root: &Path, compose_file: &str, chain_id: u64) -> Option<ComposePlan> {
    let path = compose_plan_latest_path(project_root, compose_file, chain_id);
    if !path.exists() {
        return None;
    }
    let contents = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Update a component's status and broadcast file links in the plan.
pub fn update_component(
    plan: &mut ComposePlan,
    name: &str,
    status: ComponentStatus,
    broadcast_file: Option<String>,
    queued_file: Option<String>,
) {
    if let Some(entry) = plan.components.iter_mut().find(|c| c.name == name) {
        entry.status = status;
        if broadcast_file.is_some() {
            entry.broadcast_file = broadcast_file;
        }
        if queued_file.is_some() {
            entry.queued_file = queued_file;
        }
    }
}

/// Get the list of components that still need execution.
pub fn pending_components(plan: &ComposePlan) -> Vec<&ComponentEntry> {
    plan.components
        .iter()
        .filter(|c| c.status == ComponentStatus::Pending || c.status == ComponentStatus::Simulated)
        .collect()
}

/// Check if the compose file has changed since the plan was created.
pub fn plan_matches_compose(plan: &ComposePlan, current_hash: &str) -> bool {
    plan.compose_hash == current_hash
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_plan_builds_ordered_entries() {
        let components = vec![
            ("box".to_string(), "script/DeployBox.s.sol".to_string()),
            ("counter".to_string(), "script/DeployCounter.s.sol".to_string()),
            ("vault".to_string(), "script/DeployVault.s.sol".to_string()),
        ];

        let plan = create_plan("compose/full.yaml", "abc123", 42220, &components);

        assert_eq!(plan.components.len(), 3);
        assert_eq!(plan.components[0].name, "box");
        assert_eq!(plan.components[0].step, 1);
        assert_eq!(plan.components[0].status, ComponentStatus::Pending);
        assert_eq!(plan.components[2].name, "vault");
        assert_eq!(plan.components[2].step, 3);
    }

    #[test]
    fn update_component_sets_status_and_files() {
        let mut plan = create_plan(
            "compose.yaml",
            "hash",
            1,
            &[("a".into(), "script/A.s.sol".into()), ("b".into(), "script/B.s.sol".into())],
        );

        update_component(
            &mut plan,
            "a",
            ComponentStatus::Broadcast,
            Some("broadcast/A.s.sol/1/run-1234567890.json".into()),
            Some("broadcast/A.s.sol/1/run-1234567890.queued.json".into()),
        );

        assert_eq!(plan.components[0].status, ComponentStatus::Broadcast);
        assert_eq!(
            plan.components[0].broadcast_file.as_deref(),
            Some("broadcast/A.s.sol/1/run-1234567890.json")
        );
        assert_eq!(
            plan.components[0].queued_file.as_deref(),
            Some("broadcast/A.s.sol/1/run-1234567890.queued.json")
        );
        assert_eq!(plan.components[1].status, ComponentStatus::Pending);
    }

    #[test]
    fn pending_components_filters_correctly() {
        let mut plan = create_plan(
            "compose.yaml",
            "hash",
            1,
            &[("a".into(), "s".into()), ("b".into(), "s".into()), ("c".into(), "s".into())],
        );

        plan.components[0].status = ComponentStatus::Broadcast;
        plan.components[1].status = ComponentStatus::Pending;
        plan.components[2].status = ComponentStatus::Simulated;

        let pending = pending_components(&plan);
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].name, "b");
        assert_eq!(pending[1].name, "c");
    }

    #[test]
    fn plan_hash_comparison() {
        let plan = create_plan("compose.yaml", "abc123", 1, &[]);
        assert!(plan_matches_compose(&plan, "abc123"));
        assert!(!plan_matches_compose(&plan, "different"));
    }

    #[test]
    fn compose_hash_is_deterministic() {
        let contents = "group: test\ncomponents:\n  a:\n    script: script/A.s.sol\n";
        assert_eq!(compute_compose_hash(contents), compute_compose_hash(contents));
    }

    #[test]
    fn compose_hash_changes_with_contents() {
        assert_ne!(compute_compose_hash("a"), compute_compose_hash("b"));
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut plan = create_plan(
            "compose/full.yaml",
            "hash123",
            42220,
            &[("box".into(), "script/DeployBox.s.sol".into())],
        );

        update_component(
            &mut plan,
            "box",
            ComponentStatus::Broadcast,
            Some("broadcast/DeployBox.s.sol/42220/run-1234567890.json".into()),
            None,
        );

        let path = save_plan(dir.path(), &plan).unwrap();
        assert!(path.exists());

        let loaded = load_plan(dir.path(), "compose/full.yaml", 42220).unwrap();
        assert_eq!(loaded.compose_hash, "hash123");
        assert_eq!(loaded.components.len(), 1);
        assert_eq!(loaded.components[0].status, ComponentStatus::Broadcast);
        assert_eq!(
            loaded.components[0].broadcast_file.as_deref(),
            Some("broadcast/DeployBox.s.sol/42220/run-1234567890.json")
        );
    }

    #[test]
    fn compose_plan_path_derives_name() {
        let path = compose_plan_path(Path::new("/project"), "script/compose/full.yaml", 42220);
        assert_eq!(path, PathBuf::from("/project/broadcast/full.yaml/42220/compose-latest.json"));
    }

    #[test]
    fn compose_plan_archive_path_derives_name() {
        let path = compose_plan_archive_path(
            Path::new("/project"),
            "script/compose/full.yaml",
            42220,
            123,
        );
        assert_eq!(path, PathBuf::from("/project/broadcast/full.yaml/42220/compose-123.json"));
    }

    #[test]
    fn save_plan_archive_writes_timestamped_copy() {
        let dir = tempfile::tempdir().unwrap();
        let plan = create_plan("compose/full.yaml", "hash123", 42220, &[]);

        let path = save_plan_archive(dir.path(), &plan).unwrap();
        assert!(path.exists());
        assert_eq!(path.parent().unwrap(), dir.path().join("broadcast/full.yaml/42220"));
        assert!(path.file_name().unwrap().to_string_lossy().starts_with("compose-"));
        assert_ne!(path.file_name().unwrap(), "compose-latest.json");
    }
}
