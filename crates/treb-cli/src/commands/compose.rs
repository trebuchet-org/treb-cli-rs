//! `treb compose` command implementation.
//!
//! YAML-based multi-step deployment orchestration that executes multiple
//! Forge scripts in dependency order.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};

// ── Compose file schema ──────────────────────────────────────────────────

/// Top-level compose file structure.
#[derive(Debug, Deserialize, Serialize)]
pub struct ComposeFile {
    /// Deployment group name.
    pub group: String,
    /// Map of component name → component definition (sorted for determinism).
    pub components: BTreeMap<String, Component>,
}

/// A single component in the compose file.
#[derive(Debug, Deserialize, Serialize)]
pub struct Component {
    /// Path to the Forge script (e.g., `script/Deploy.s.sol`).
    pub script: String,
    /// Names of components this one depends on (must execute first).
    #[serde(default)]
    pub deps: Option<Vec<String>>,
    /// Per-component environment variables (merged with global `--env`).
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    /// Function signature to call (defaults to `run()` at execution time).
    #[serde(default)]
    pub sig: Option<String>,
    /// Arguments to pass to the script function.
    #[serde(default)]
    pub args: Option<Vec<String>>,
    /// Per-component verify override (overrides global `--verify` when set).
    #[serde(default)]
    pub verify: Option<bool>,
}

// ── Parsing and validation ───────────────────────────────────────────────

/// Load and parse a compose YAML file from disk.
pub fn load_compose_file(path: &str) -> anyhow::Result<ComposeFile> {
    let path = Path::new(path);
    if !path.exists() {
        bail!("compose file not found: {}", path.display());
    }
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read compose file: {}", path.display()))?;
    let compose: ComposeFile = serde_yaml::from_str(&contents)
        .with_context(|| format!("failed to parse compose file: {}", path.display()))?;
    Ok(compose)
}

/// Validate a parsed compose file.
///
/// Checks structural invariants that serde alone cannot enforce:
/// non-empty group, non-empty components, valid script paths,
/// valid dependency references, no self-dependencies, and valid
/// component names.
pub fn validate_compose_file(compose: &ComposeFile) -> anyhow::Result<()> {
    if compose.group.is_empty() {
        bail!("compose file validation failed: 'group' must not be empty");
    }
    if compose.components.is_empty() {
        bail!("compose file validation failed: 'components' must not be empty");
    }
    for (name, component) in &compose.components {
        // Validate component name: alphanumeric, hyphens, underscores only
        if !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            bail!(
                "component '{}' has an invalid name: must contain only alphanumeric characters, hyphens, and underscores",
                name
            );
        }
        // Validate script is non-empty
        if component.script.is_empty() {
            bail!(
                "component '{}' has an empty 'script' field",
                name
            );
        }
        // Validate dependency references
        if let Some(deps) = &component.deps {
            for dep in deps {
                if dep == name {
                    bail!("component '{}' cannot depend on itself", name);
                }
                if !compose.components.contains_key(dep) {
                    bail!(
                        "component '{}' depends on unknown component '{}'",
                        name,
                        dep
                    );
                }
            }
        }
    }
    Ok(())
}

// ── Command entry point ──────────────────────────────────────────────────

/// Execute a compose deployment pipeline.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    file: String,
    _network: Option<String>,
    _rpc_url: Option<String>,
    _namespace: Option<String>,
    _profile: Option<String>,
    _broadcast: bool,
    _dry_run: bool,
    _resume: bool,
    _verify: bool,
    _slow: bool,
    _legacy: bool,
    _verbose: bool,
    _json: bool,
    _env_vars: Vec<String>,
    _non_interactive: bool,
) -> anyhow::Result<()> {
    // Parse and validate the compose file.
    let compose = load_compose_file(&file)?;
    validate_compose_file(&compose)?;

    // Remaining orchestration implemented in subsequent user stories.
    let _ = compose;
    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_minimal_compose_file() {
        let yaml = r#"
group: my-deployment
components:
  token:
    script: script/DeployToken.s.sol
"#;
        let compose: ComposeFile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(compose.group, "my-deployment");
        assert_eq!(compose.components.len(), 1);
        let token = &compose.components["token"];
        assert_eq!(token.script, "script/DeployToken.s.sol");
        assert!(token.deps.is_none());
        assert!(token.env.is_none());
        assert!(token.sig.is_none());
        assert!(token.args.is_none());
        assert!(token.verify.is_none());
    }

    #[test]
    fn deserialize_full_compose_file() {
        let yaml = r#"
group: full-deploy
components:
  libraries:
    script: script/DeployLibs.s.sol
    sig: "deploy(uint256)"
    args:
      - "42"
    verify: true
  core:
    script: script/DeployCore.s.sol
    deps:
      - libraries
    env:
      DEPLOYER_KEY: "0xabc"
      SALT: "0x01"
  periphery:
    script: script/DeployPeriphery.s.sol
    deps:
      - libraries
      - core
    verify: false
"#;
        let compose: ComposeFile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(compose.group, "full-deploy");
        assert_eq!(compose.components.len(), 3);

        let libs = &compose.components["libraries"];
        assert_eq!(libs.script, "script/DeployLibs.s.sol");
        assert!(libs.deps.is_none());
        assert!(libs.env.is_none());
        assert_eq!(libs.sig.as_deref(), Some("deploy(uint256)"));
        assert_eq!(libs.args.as_ref().unwrap(), &vec!["42".to_string()]);
        assert_eq!(libs.verify, Some(true));

        let core = &compose.components["core"];
        assert_eq!(core.deps.as_ref().unwrap(), &vec!["libraries".to_string()]);
        let env = core.env.as_ref().unwrap();
        assert_eq!(env.get("DEPLOYER_KEY").unwrap(), "0xabc");
        assert_eq!(env.get("SALT").unwrap(), "0x01");
        assert!(core.sig.is_none());
        assert!(core.args.is_none());
        assert!(core.verify.is_none());

        let periphery = &compose.components["periphery"];
        assert_eq!(
            periphery.deps.as_ref().unwrap(),
            &vec!["libraries".to_string(), "core".to_string()]
        );
        assert_eq!(periphery.verify, Some(false));
    }

    #[test]
    fn optional_fields_deserialize_as_none() {
        let yaml = r#"
group: test
components:
  a:
    script: script/A.s.sol
"#;
        let compose: ComposeFile = serde_yaml::from_str(yaml).unwrap();
        let a = &compose.components["a"];
        assert!(a.deps.is_none(), "deps should be None, not Some(vec![])");
        assert!(a.env.is_none(), "env should be None, not Some(map)");
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let yaml = r#"
group: test
extra_field: ignored
components:
  a:
    script: script/A.s.sol
    unknown_option: true
"#;
        // serde_yaml with default settings ignores unknown fields
        let result = serde_yaml::from_str::<ComposeFile>(yaml);
        assert!(result.is_ok(), "unknown fields should be ignored: {:?}", result.err());
    }

    // ── Validation tests ────────────────────────────────────────────────

    #[test]
    fn validate_valid_compose_file() {
        let yaml = r#"
group: my-deploy
components:
  libs:
    script: script/DeployLibs.s.sol
  core:
    script: script/DeployCore.s.sol
    deps:
      - libs
"#;
        let compose: ComposeFile = serde_yaml::from_str(yaml).unwrap();
        assert!(validate_compose_file(&compose).is_ok());
    }

    #[test]
    fn validate_empty_group_fails() {
        let yaml = r#"
group: ""
components:
  a:
    script: script/A.s.sol
"#;
        let compose: ComposeFile = serde_yaml::from_str(yaml).unwrap();
        let err = validate_compose_file(&compose).unwrap_err();
        assert!(
            err.to_string().contains("'group' must not be empty"),
            "expected empty group error, got: {}",
            err
        );
    }

    #[test]
    fn validate_empty_components_fails() {
        let yaml = r#"
group: test
components: {}
"#;
        let compose: ComposeFile = serde_yaml::from_str(yaml).unwrap();
        let err = validate_compose_file(&compose).unwrap_err();
        assert!(
            err.to_string().contains("'components' must not be empty"),
            "expected empty components error, got: {}",
            err
        );
    }

    #[test]
    fn validate_empty_script_fails() {
        let yaml = r#"
group: test
components:
  bad:
    script: ""
"#;
        let compose: ComposeFile = serde_yaml::from_str(yaml).unwrap();
        let err = validate_compose_file(&compose).unwrap_err();
        assert!(
            err.to_string().contains("component 'bad'")
                && err.to_string().contains("empty 'script'"),
            "expected empty script error for 'bad', got: {}",
            err
        );
    }

    #[test]
    fn validate_unknown_dep_fails() {
        let yaml = r#"
group: test
components:
  a:
    script: script/A.s.sol
    deps:
      - nonexistent
"#;
        let compose: ComposeFile = serde_yaml::from_str(yaml).unwrap();
        let err = validate_compose_file(&compose).unwrap_err();
        assert!(
            err.to_string()
                .contains("component 'a' depends on unknown component 'nonexistent'"),
            "expected unknown dep error, got: {}",
            err
        );
    }

    #[test]
    fn validate_self_dep_fails() {
        let yaml = r#"
group: test
components:
  a:
    script: script/A.s.sol
    deps:
      - a
"#;
        let compose: ComposeFile = serde_yaml::from_str(yaml).unwrap();
        let err = validate_compose_file(&compose).unwrap_err();
        assert!(
            err.to_string().contains("component 'a' cannot depend on itself"),
            "expected self-dep error, got: {}",
            err
        );
    }

    #[test]
    fn validate_invalid_component_name_fails() {
        let yaml = r#"
group: test
components:
  "bad name":
    script: script/A.s.sol
"#;
        let compose: ComposeFile = serde_yaml::from_str(yaml).unwrap();
        let err = validate_compose_file(&compose).unwrap_err();
        assert!(
            err.to_string().contains("component 'bad name'")
                && err.to_string().contains("invalid name"),
            "expected invalid name error, got: {}",
            err
        );
    }

    #[test]
    fn validate_component_name_with_hyphens_and_underscores() {
        let yaml = r#"
group: test
components:
  my-component_v2:
    script: script/A.s.sol
"#;
        let compose: ComposeFile = serde_yaml::from_str(yaml).unwrap();
        assert!(validate_compose_file(&compose).is_ok());
    }

    // ── Loading tests ───────────────────────────────────────────────────

    #[test]
    fn load_missing_file_fails() {
        let err = load_compose_file("/nonexistent/path/compose.yaml").unwrap_err();
        assert!(
            err.to_string().contains("compose file not found"),
            "expected file not found error, got: {}",
            err
        );
    }

    #[test]
    fn load_malformed_yaml_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.yaml");
        std::fs::write(&path, "not: [valid: yaml: {{").unwrap();
        let err = load_compose_file(path.to_str().unwrap()).unwrap_err();
        assert!(
            format!("{:#}", err).contains("failed to parse compose file"),
            "expected parse error, got: {:#}",
            err
        );
    }

    #[test]
    fn load_valid_file_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deploy.yaml");
        std::fs::write(
            &path,
            "group: test\ncomponents:\n  a:\n    script: script/A.s.sol\n",
        )
        .unwrap();
        let compose = load_compose_file(path.to_str().unwrap()).unwrap();
        assert_eq!(compose.group, "test");
        assert_eq!(compose.components.len(), 1);
    }
}
