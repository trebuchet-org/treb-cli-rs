//! Local config store — reads/writes `.treb/config.local.json`.
//!
//! Provides persistence for user defaults (namespace, network) in a
//! project-local `.treb/` directory. The JSON format is identical to the
//! Go implementation (2-space indentation, trailing newline).

use std::path::Path;

use treb_core::error::{Result, TrebError};

use crate::LocalConfig;

const LOCAL_DIR: &str = ".treb";
const LOCAL_FILE: &str = "config.local.json";

/// Loads the local config from `<project_root>/.treb/config.local.json`.
///
/// Returns `LocalConfig::default()` if the file does not exist.
/// Returns `TrebError::Config` if the file exists but contains invalid JSON.
pub fn load_local_config(project_root: &Path) -> Result<LocalConfig> {
    let path = project_root.join(LOCAL_DIR).join(LOCAL_FILE);

    if !path.exists() {
        return Ok(LocalConfig::default());
    }

    let contents = std::fs::read_to_string(&path)?;
    serde_json::from_str(&contents)
        .map_err(|e| TrebError::Config(format!("invalid JSON in {}: {e}", path.display())))
}

/// Saves the local config to `<project_root>/.treb/config.local.json`.
///
/// Creates the `.treb/` directory if it does not exist. Writes 2-space
/// indented JSON with a trailing newline to match the Go output format.
pub fn save_local_config(project_root: &Path, config: &LocalConfig) -> Result<()> {
    let dir = project_root.join(LOCAL_DIR);
    std::fs::create_dir_all(&dir)?;

    let path = dir.join(LOCAL_FILE);
    let mut json = serde_json::to_string_pretty(config)
        .map_err(|e| TrebError::Config(format!("failed to serialize local config: {e}")))?;
    json.push('\n');

    std::fs::write(&path, json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn round_trip_save_then_load() {
        let tmp = TempDir::new().unwrap();
        let config =
            LocalConfig { namespace: "production".to_string(), network: "mainnet".to_string() };

        save_local_config(tmp.path(), &config).unwrap();
        let loaded = load_local_config(tmp.path()).unwrap();

        assert_eq!(loaded, config);
    }

    #[test]
    fn nonexistent_file_returns_defaults() {
        let tmp = TempDir::new().unwrap();
        let config = load_local_config(tmp.path()).unwrap();

        assert_eq!(config, LocalConfig::default());
        assert_eq!(config.namespace, "default");
        assert_eq!(config.network, "");
    }

    #[test]
    fn invalid_json_returns_config_error() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(LOCAL_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(LOCAL_FILE), "{ not valid json }").unwrap();

        let err = load_local_config(tmp.path()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("config error"), "expected config error, got: {msg}");
        assert!(msg.contains("invalid JSON"), "expected 'invalid JSON', got: {msg}");
    }

    #[test]
    fn save_creates_treb_directory() {
        let tmp = TempDir::new().unwrap();
        let treb_dir = tmp.path().join(LOCAL_DIR);

        // Directory should not exist yet.
        assert!(!treb_dir.exists());

        save_local_config(tmp.path(), &LocalConfig::default()).unwrap();

        // Now it should exist, with the config file inside.
        assert!(treb_dir.exists());
        assert!(treb_dir.join(LOCAL_FILE).exists());
    }

    #[test]
    fn json_format_matches_go_output() {
        let tmp = TempDir::new().unwrap();
        let config = LocalConfig::default();
        save_local_config(tmp.path(), &config).unwrap();

        let contents =
            std::fs::read_to_string(tmp.path().join(LOCAL_DIR).join(LOCAL_FILE)).unwrap();

        // 2-space indentation, trailing newline.
        let expected = "{\n  \"namespace\": \"default\",\n  \"network\": \"\"\n}\n";
        assert_eq!(contents, expected);
    }
}
