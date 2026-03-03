//! `treb compose` command implementation.
//!
//! YAML-based multi-step deployment orchestration that executes multiple
//! Forge scripts in dependency order.

use std::collections::{BTreeMap, HashMap};

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

// ── Command entry point ──────────────────────────────────────────────────

/// Execute a compose deployment pipeline.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    _file: String,
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
    // Stub — full implementation in subsequent user stories.
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
}
