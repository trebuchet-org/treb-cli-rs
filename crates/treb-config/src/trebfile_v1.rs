//! treb.toml v1 parser — legacy read-only support for `[ns.*]` format.
//!
//! The v1 format uses `[ns.default]` and `[ns.<name>]` sections where each
//! namespace contains an inline `profile`, optional `slow` flag, and a
//! `[ns.<name>.senders.*]` table. This module parses the file and provides
//! conversion to `ResolvedSenders` by merging `ns.default` with a selected
//! namespace (namespace values override defaults).

use std::path::Path;

use treb_core::error::{Result, TrebError};

use crate::trebfile::expand_sender_config_env_vars;
use crate::{ResolvedSenders, TrebFileConfigV1};

/// Loads and parses a treb.toml v1 config file.
///
/// Returns `TrebError::Config` if the file does not exist or contains
/// invalid TOML. After parsing, all string fields in sender configs are
/// expanded for `${VAR_NAME}` environment variable references.
pub fn load_treb_config_v1(path: &Path) -> Result<TrebFileConfigV1> {
    if !path.exists() {
        return Err(TrebError::Config(format!(
            "treb.toml not found: {}",
            path.display()
        )));
    }

    let contents = std::fs::read_to_string(path)?;
    let mut config: TrebFileConfigV1 = toml::from_str(&contents).map_err(|e| {
        TrebError::Config(format!(
            "invalid TOML in {}: {e}",
            path.display()
        ))
    })?;

    // Expand environment variables in all sender config string fields.
    for ns_config in config.ns.values_mut() {
        for sender in ns_config.senders.values_mut() {
            expand_sender_config_env_vars(sender);
        }
    }

    Ok(config)
}

/// Merges `ns.default` with `ns.<namespace>` to produce resolved senders.
///
/// The merge strategy:
/// 1. Start with senders from `ns.default`
/// 2. Override/extend with senders from `ns.<namespace>` (namespace wins)
/// 3. Profile: namespace-specific profile overrides default profile
/// 4. Slow flag: namespace-specific slow overrides default slow, which
///    overrides the top-level slow flag; defaults to `false`
pub fn convert_v1_to_resolved(v1: &TrebFileConfigV1, namespace: &str) -> ResolvedSenders {
    let mut resolved = ResolvedSenders::default();

    // Start with ns.default if it exists.
    if let Some(default_ns) = v1.ns.get("default") {
        if let Some(ref profile) = default_ns.profile {
            resolved.profile = profile.clone();
        }
        // Default slow: top-level -> ns.default
        resolved.slow = v1.slow.unwrap_or(false);
        if let Some(slow) = default_ns.slow {
            resolved.slow = slow;
        }
        // Copy all default senders.
        resolved.senders = default_ns.senders.clone();
    } else {
        // No default namespace — use top-level slow only.
        resolved.slow = v1.slow.unwrap_or(false);
    }

    // Override with namespace-specific config if it exists and isn't "default".
    if namespace != "default" {
        if let Some(ns_config) = v1.ns.get(namespace) {
            if let Some(ref profile) = ns_config.profile {
                resolved.profile = profile.clone();
            }
            if let Some(slow) = ns_config.slow {
                resolved.slow = slow;
            }
            // Merge senders: namespace-specific overrides default.
            for (role, sender) in &ns_config.senders {
                resolved.senders.insert(role.clone(), sender.clone());
            }
        }
    }

    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SenderConfig, SenderType};
    use tempfile::TempDir;

    // ---- load_treb_config_v1 ----

    #[test]
    fn parse_v1_fixture() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("treb.toml");
        std::fs::write(
            &path,
            r#"
slow = true

[ns.default]
profile = "default"

[ns.default.senders.deployer]
type = "private_key"
address = "0xDefaultDeployer"
private_key = "0xDefaultKey"

[ns.default.senders.admin]
type = "ledger"
address = "0xDefaultAdmin"

[ns.live]
profile = "optimized"
slow = false

[ns.live.senders.deployer]
type = "ledger"
address = "0xLiveDeployer"
derivation_path = "m/44'/60'/0'/0/1"
"#,
        )
        .unwrap();

        let config = load_treb_config_v1(&path).unwrap();

        // Top-level slow flag.
        assert_eq!(config.slow, Some(true));

        // Two namespaces: default and live.
        assert_eq!(config.ns.len(), 2);

        // Default namespace.
        let default_ns = &config.ns["default"];
        assert_eq!(default_ns.profile, Some("default".to_string()));
        assert_eq!(default_ns.senders.len(), 2);
        assert_eq!(
            default_ns.senders["deployer"].type_,
            Some(SenderType::PrivateKey)
        );
        assert_eq!(
            default_ns.senders["deployer"].address,
            Some("0xDefaultDeployer".to_string())
        );
        assert_eq!(
            default_ns.senders["admin"].type_,
            Some(SenderType::Ledger)
        );

        // Live namespace.
        let live_ns = &config.ns["live"];
        assert_eq!(live_ns.profile, Some("optimized".to_string()));
        assert_eq!(live_ns.slow, Some(false));
        assert_eq!(live_ns.senders.len(), 1);
        assert_eq!(
            live_ns.senders["deployer"].type_,
            Some(SenderType::Ledger)
        );
        assert_eq!(
            live_ns.senders["deployer"].derivation_path,
            Some("m/44'/60'/0'/0/1".to_string())
        );
    }

    #[test]
    fn parse_v1_env_var_expansion() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("treb.toml");
        std::fs::write(
            &path,
            r#"
[ns.default.senders.deployer]
type = "private_key"
private_key = "${TREB_TEST_V1_KEY}"
"#,
        )
        .unwrap();

        unsafe { std::env::set_var("TREB_TEST_V1_KEY", "0xExpandedV1Key") };
        let config = load_treb_config_v1(&path).unwrap();
        assert_eq!(
            config.ns["default"].senders["deployer"].private_key,
            Some("0xExpandedV1Key".to_string())
        );
        unsafe { std::env::remove_var("TREB_TEST_V1_KEY") };
    }

    #[test]
    fn parse_v1_missing_file_errors() {
        let err = load_treb_config_v1(Path::new("/nonexistent/treb.toml")).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("treb.toml not found"),
            "expected 'treb.toml not found', got: {msg}"
        );
    }

    #[test]
    fn parse_v1_invalid_toml_errors() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("treb.toml");
        std::fs::write(&path, "[[[ not valid toml").unwrap();

        let err = load_treb_config_v1(&path).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("invalid TOML"),
            "expected 'invalid TOML', got: {msg}"
        );
    }

    // ---- convert_v1_to_resolved ----

    fn make_test_v1() -> TrebFileConfigV1 {
        let mut v1 = TrebFileConfigV1 {
            slow: Some(true),
            ns: Default::default(),
        };

        // ns.default: deployer (private_key) + admin (ledger), profile="default"
        let mut default_ns = crate::NamespaceConfigV1 {
            profile: Some("default".to_string()),
            slow: None,
            senders: Default::default(),
        };
        default_ns.senders.insert(
            "deployer".to_string(),
            SenderConfig {
                type_: Some(SenderType::PrivateKey),
                address: Some("0xDefaultDeployer".to_string()),
                private_key: Some("0xDefaultKey".to_string()),
                ..Default::default()
            },
        );
        default_ns.senders.insert(
            "admin".to_string(),
            SenderConfig {
                type_: Some(SenderType::Ledger),
                address: Some("0xDefaultAdmin".to_string()),
                ..Default::default()
            },
        );
        v1.ns.insert("default".to_string(), default_ns);

        // ns.live: deployer (ledger) overrides default, profile="optimized", slow=false
        let mut live_ns = crate::NamespaceConfigV1 {
            profile: Some("optimized".to_string()),
            slow: Some(false),
            senders: Default::default(),
        };
        live_ns.senders.insert(
            "deployer".to_string(),
            SenderConfig {
                type_: Some(SenderType::Ledger),
                address: Some("0xLiveDeployer".to_string()),
                derivation_path: Some("m/44'/60'/0'/0/1".to_string()),
                ..Default::default()
            },
        );
        v1.ns.insert("live".to_string(), live_ns);

        v1
    }

    #[test]
    fn convert_v1_default_namespace() {
        let v1 = make_test_v1();
        let resolved = convert_v1_to_resolved(&v1, "default");

        assert_eq!(resolved.profile, "default");
        // slow: top-level is true, ns.default.slow is None → inherits true
        assert!(resolved.slow);
        assert_eq!(resolved.senders.len(), 2);
        assert_eq!(
            resolved.senders["deployer"].type_,
            Some(SenderType::PrivateKey)
        );
        assert_eq!(
            resolved.senders["admin"].type_,
            Some(SenderType::Ledger)
        );
    }

    #[test]
    fn convert_v1_live_namespace_merges_and_overrides() {
        let v1 = make_test_v1();
        let resolved = convert_v1_to_resolved(&v1, "live");

        // Profile overridden from "default" to "optimized".
        assert_eq!(resolved.profile, "optimized");

        // Slow: ns.live.slow=false overrides top-level slow=true.
        assert!(!resolved.slow);

        // Senders: default "admin" preserved, default "deployer" overridden by live.
        assert_eq!(resolved.senders.len(), 2);

        // deployer is now ledger (from live), not private_key (from default).
        assert_eq!(
            resolved.senders["deployer"].type_,
            Some(SenderType::Ledger)
        );
        assert_eq!(
            resolved.senders["deployer"].address,
            Some("0xLiveDeployer".to_string())
        );
        assert_eq!(
            resolved.senders["deployer"].derivation_path,
            Some("m/44'/60'/0'/0/1".to_string())
        );

        // admin inherited from default, unchanged.
        assert_eq!(
            resolved.senders["admin"].type_,
            Some(SenderType::Ledger)
        );
        assert_eq!(
            resolved.senders["admin"].address,
            Some("0xDefaultAdmin".to_string())
        );
    }

    #[test]
    fn convert_v1_slow_flag_precedence() {
        // Test: ns.default.slow overrides top-level slow.
        let mut v1 = TrebFileConfigV1 {
            slow: Some(true),
            ns: Default::default(),
        };
        v1.ns.insert(
            "default".to_string(),
            crate::NamespaceConfigV1 {
                profile: None,
                slow: Some(false),
                senders: Default::default(),
            },
        );

        let resolved = convert_v1_to_resolved(&v1, "default");
        // ns.default.slow=false overrides top-level slow=true.
        assert!(!resolved.slow);
    }

    #[test]
    fn convert_v1_unknown_namespace_uses_defaults() {
        let v1 = make_test_v1();
        let resolved = convert_v1_to_resolved(&v1, "staging");

        // Falls back to default namespace values since "staging" doesn't exist.
        assert_eq!(resolved.profile, "default");
        assert!(resolved.slow);
        assert_eq!(resolved.senders.len(), 2);
    }

    #[test]
    fn convert_v1_no_default_namespace() {
        let mut v1 = TrebFileConfigV1 {
            slow: Some(true),
            ns: Default::default(),
        };
        v1.ns.insert(
            "live".to_string(),
            crate::NamespaceConfigV1 {
                profile: Some("live-profile".to_string()),
                slow: Some(false),
                senders: Default::default(),
            },
        );

        let resolved = convert_v1_to_resolved(&v1, "live");
        assert_eq!(resolved.profile, "live-profile");
        assert!(!resolved.slow);
        assert!(resolved.senders.is_empty());
    }
}
