//! treb.toml v2 parser — loads and validates the primary treb config file.
//!
//! Supports `[accounts.*]`, `[namespace.*]`, and `[fork]` sections with
//! environment variable expansion (`${VAR_NAME}`) in all string fields of
//! account/sender configs.

use std::path::Path;

use treb_core::error::{Result, TrebError};

use crate::{TrebConfigFormat, TrebFileConfigV2};

/// Loads and parses a treb.toml v2 config file.
///
/// Returns `TrebError::Config` if the file does not exist or contains
/// invalid TOML. After parsing, all string fields in account configs are
/// expanded for `${VAR_NAME}` environment variable references.
pub fn load_treb_config_v2(path: &Path) -> Result<TrebFileConfigV2> {
    if !path.exists() {
        return Err(TrebError::Config(format!(
            "treb.toml not found: {}",
            path.display()
        )));
    }

    let contents = std::fs::read_to_string(path)?;
    let mut config: TrebFileConfigV2 = toml::from_str(&contents).map_err(|e| {
        TrebError::Config(format!(
            "invalid TOML in {}: {e}",
            path.display()
        ))
    })?;

    // Expand environment variables in all account config string fields.
    for account in config.accounts.values_mut() {
        expand_sender_config_env_vars(account);
    }

    Ok(config)
}

/// Detects the format of a `treb.toml` file in the given project root.
///
/// Returns `V2` if the file contains `[accounts]`, `[namespace]`, or `[fork]`
/// sections. Returns `V1` if it contains `[ns.` sections. Returns `None` if
/// no `treb.toml` exists.
pub fn detect_treb_config_format(project_root: &Path) -> TrebConfigFormat {
    let path = project_root.join("treb.toml");
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return TrebConfigFormat::None,
    };

    // Check for v2 markers first (higher priority).
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("[accounts")
            || trimmed.starts_with("[namespace")
            || trimmed.starts_with("[fork")
        {
            return TrebConfigFormat::V2;
        }
    }

    // Check for v1 markers.
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("[ns.") {
            return TrebConfigFormat::V1;
        }
    }

    // File exists but has no recognizable sections — treat as v2
    // (empty config with defaults).
    TrebConfigFormat::V2
}

/// Replaces `${VAR_NAME}` patterns in a string with environment variable
/// values. Unset variables expand to empty string.
pub fn expand_env_vars(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() == Some(&'{') {
            chars.next(); // consume '{'
            let mut var_name = String::new();
            for c in chars.by_ref() {
                if c == '}' {
                    break;
                }
                var_name.push(c);
            }
            if let Ok(val) = std::env::var(&var_name) {
                result.push_str(&val);
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Expands environment variables in all string fields of a `SenderConfig`.
pub(crate) fn expand_sender_config_env_vars(config: &mut crate::SenderConfig) {
    if let Some(ref mut v) = config.address {
        *v = expand_env_vars(v);
    }
    if let Some(ref mut v) = config.private_key {
        *v = expand_env_vars(v);
    }
    if let Some(ref mut v) = config.safe {
        *v = expand_env_vars(v);
    }
    if let Some(ref mut v) = config.signer {
        *v = expand_env_vars(v);
    }
    if let Some(ref mut v) = config.derivation_path {
        *v = expand_env_vars(v);
    }
    if let Some(ref mut v) = config.governor {
        *v = expand_env_vars(v);
    }
    if let Some(ref mut v) = config.timelock {
        *v = expand_env_vars(v);
    }
    if let Some(ref mut v) = config.proposer {
        *v = expand_env_vars(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SenderType;
    use tempfile::TempDir;

    // ---- load_treb_config_v2 ----

    #[test]
    fn parse_v2_fixture() {
        // Use a self-contained fixture to avoid env var race conditions.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("treb.toml");
        std::fs::write(
            &path,
            r#"
[accounts.deployer]
type = "private_key"
address = "0xDeployerAddr"
private_key = "0xStaticKey"

[accounts.ledger_signer]
type = "ledger"
address = "0xLedgerAddr"
derivation_path = "m/44'/60'/0'/0/0"

[accounts.multisig]
type = "safe"
address = "0xSafeAddr"
safe = "0xSafeContract"
signer = "deployer"

[accounts.governor]
type = "oz_governor"
address = "0xGovAddr"
governor = "0xGovernorContract"
timelock = "0xTimelockContract"
proposer = "deployer"

[namespace.default]
profile = "default"

[namespace.default.senders]
deployer = "deployer"

[namespace.production]
profile = "optimized"

[namespace.production.senders]
deployer = "ledger_signer"
admin = "multisig"

[fork]
setup = "script/ForkSetup.s.sol"
"#,
        )
        .unwrap();

        let config = load_treb_config_v2(&path).unwrap();

        // Verify account count and types.
        assert_eq!(config.accounts.len(), 4);
        assert_eq!(
            config.accounts["deployer"].type_,
            Some(SenderType::PrivateKey)
        );
        assert_eq!(
            config.accounts["ledger_signer"].type_,
            Some(SenderType::Ledger)
        );
        assert_eq!(config.accounts["multisig"].type_, Some(SenderType::Safe));
        assert_eq!(
            config.accounts["governor"].type_,
            Some(SenderType::OZGovernor)
        );

        // Verify namespace count.
        assert_eq!(config.namespace.len(), 2);
        assert_eq!(
            config.namespace["default"].profile,
            Some("default".to_string())
        );
        assert_eq!(
            config.namespace["production"].profile,
            Some("optimized".to_string())
        );

        // Verify fork config.
        assert_eq!(
            config.fork.setup,
            Some("script/ForkSetup.s.sol".to_string())
        );

        // Verify static private key (no env var expansion needed).
        assert_eq!(
            config.accounts["deployer"].private_key,
            Some("0xStaticKey".to_string())
        );
    }

    #[test]
    fn parse_v2_env_var_expansion() {
        // Use a dedicated TOML with a unique env var to avoid race conditions.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("treb.toml");
        std::fs::write(
            &path,
            "[accounts.deployer]\ntype = \"private_key\"\nprivate_key = \"${TREB_TEST_EXPAND_KEY}\"\n",
        )
        .unwrap();

        unsafe { std::env::set_var("TREB_TEST_EXPAND_KEY", "0xExpandedKey") };
        let config = load_treb_config_v2(&path).unwrap();
        assert_eq!(
            config.accounts["deployer"].private_key,
            Some("0xExpandedKey".to_string())
        );
        unsafe { std::env::remove_var("TREB_TEST_EXPAND_KEY") };
    }

    #[test]
    fn parse_v2_unset_env_var_expands_to_empty() {
        // Use a dedicated TOML with a unique env var to avoid race conditions.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("treb.toml");
        std::fs::write(
            &path,
            "[accounts.deployer]\ntype = \"private_key\"\nprivate_key = \"${TREB_TEST_UNSET_KEY}\"\n",
        )
        .unwrap();

        unsafe { std::env::remove_var("TREB_TEST_UNSET_KEY") };
        let config = load_treb_config_v2(&path).unwrap();
        assert_eq!(
            config.accounts["deployer"].private_key,
            Some(String::new())
        );
    }

    #[test]
    fn parse_v2_missing_file_errors() {
        let err = load_treb_config_v2(Path::new("/nonexistent/treb.toml")).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("treb.toml not found"),
            "expected 'treb.toml not found', got: {msg}"
        );
    }

    #[test]
    fn parse_v2_invalid_toml_errors() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("treb.toml");
        std::fs::write(&path, "[[[ not valid toml").unwrap();

        let err = load_treb_config_v2(&path).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("invalid TOML"),
            "expected 'invalid TOML', got: {msg}"
        );
        assert!(
            msg.contains("treb.toml"),
            "expected file path in error, got: {msg}"
        );
    }

    // ---- detect_treb_config_format ----

    #[test]
    fn detect_format_v2() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("treb.toml"),
            "[accounts.deployer]\ntype = \"private_key\"\n",
        )
        .unwrap();

        assert_eq!(detect_treb_config_format(tmp.path()), TrebConfigFormat::V2);
    }

    #[test]
    fn detect_format_v1() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("treb.toml"),
            "[ns.default]\nprofile = \"default\"\n",
        )
        .unwrap();

        assert_eq!(detect_treb_config_format(tmp.path()), TrebConfigFormat::V1);
    }

    #[test]
    fn detect_format_none() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(
            detect_treb_config_format(tmp.path()),
            TrebConfigFormat::None
        );
    }

    // ---- expand_env_vars ----

    #[test]
    fn expand_env_vars_replaces_set_var() {
        unsafe { std::env::set_var("TREB_TEST_VAR", "hello") };
        assert_eq!(expand_env_vars("prefix_${TREB_TEST_VAR}_suffix"), "prefix_hello_suffix");
        unsafe { std::env::remove_var("TREB_TEST_VAR") };
    }

    #[test]
    fn expand_env_vars_unset_becomes_empty() {
        unsafe { std::env::remove_var("TREB_UNSET_VAR") };
        assert_eq!(expand_env_vars("${TREB_UNSET_VAR}"), "");
    }

    #[test]
    fn expand_env_vars_no_vars_unchanged() {
        assert_eq!(expand_env_vars("no vars here"), "no vars here");
    }

    #[test]
    fn expand_env_vars_multiple_vars() {
        unsafe {
            std::env::set_var("TREB_A", "one");
            std::env::set_var("TREB_B", "two");
        }
        assert_eq!(expand_env_vars("${TREB_A}-${TREB_B}"), "one-two");
        unsafe {
            std::env::remove_var("TREB_A");
            std::env::remove_var("TREB_B");
        }
    }

    #[test]
    fn expand_env_vars_dollar_without_brace_unchanged() {
        assert_eq!(expand_env_vars("$NOT_A_VAR"), "$NOT_A_VAR");
    }

    // ---- v2 namespace sender mapping ----

    #[test]
    fn parse_v2_namespace_sender_mapping() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("treb.toml");
        std::fs::write(
            &path,
            r#"
[accounts.deployer]
type = "private_key"

[accounts.ledger_signer]
type = "ledger"

[namespace.default.senders]
deployer = "deployer"

[namespace.production.senders]
deployer = "ledger_signer"
admin = "multisig"

[namespace."production.ntt".senders]
deployer = "deployer"
governance = "governor"
"#,
        )
        .unwrap();

        let config = load_treb_config_v2(&path).unwrap();

        // Default namespace maps deployer role to deployer account.
        assert_eq!(config.namespace["default"].senders["deployer"], "deployer");

        // Production namespace maps deployer to ledger_signer, admin to multisig.
        assert_eq!(
            config.namespace["production"].senders["deployer"],
            "ledger_signer"
        );
        assert_eq!(
            config.namespace["production"].senders["admin"],
            "multisig"
        );

        // production.ntt overrides deployer back to deployer, adds governance.
        assert_eq!(
            config.namespace["production.ntt"].senders["deployer"],
            "deployer"
        );
        assert_eq!(
            config.namespace["production.ntt"].senders["governance"],
            "governor"
        );
    }
}
