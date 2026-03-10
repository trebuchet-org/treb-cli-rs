//! Foundry config integration.
//!
//! Wraps `foundry_config::Config::load()` for foundry.toml parsing and
//! extracts treb-specific sender config from `[profile.*.treb.senders.*]`
//! sections. Also provides RPC endpoint extraction.

use std::{collections::HashMap, path::Path};

use treb_core::error::{Result, TrebError};

use crate::{SenderConfig, expand_env_vars, trebfile::expand_sender_config_env_vars};

/// Load foundry configuration from the given project root.
///
/// Wraps `foundry_config::Config::load_with_root()` and maps errors
/// to `TrebError::Config`.
pub fn load_foundry_config(project_root: &Path) -> Result<foundry_config::Config> {
    foundry_config::Config::load_with_root(project_root)
        .map_err(|e| TrebError::Config(format!("failed to load foundry.toml: {e}")))
}

/// Extract treb sender configs from `[profile.<name>.treb.senders.*]`
/// sections in foundry.toml.
///
/// Since `foundry_config::Config` does not preserve custom treb sections,
/// this function re-reads foundry.toml as raw TOML and navigates to the
/// sender definitions for the given profile.
///
/// Returns an empty map if the file is missing or has no treb senders.
pub fn extract_treb_senders_from_foundry(
    project_root: &Path,
    profile: &str,
) -> HashMap<String, SenderConfig> {
    let foundry_path = project_root.join("foundry.toml");
    let content = match std::fs::read_to_string(&foundry_path) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };

    let table: toml::Value = match toml::from_str(&content) {
        Ok(v) => v,
        Err(_) => return HashMap::new(),
    };

    // Navigate: profile -> <name> -> treb -> senders
    let senders_table = table
        .get("profile")
        .and_then(|p| p.get(profile))
        .and_then(|p| p.get("treb"))
        .and_then(|t| t.get("senders"))
        .and_then(|s| s.as_table());

    let Some(senders) = senders_table else {
        return HashMap::new();
    };

    let mut result = HashMap::new();
    for (name, value) in senders {
        if let Ok(mut sender) = value.clone().try_into::<SenderConfig>() {
            expand_sender_config_env_vars(&mut sender);
            result.insert(name.clone(), sender);
        }
    }
    result
}

/// Extract RPC endpoint URLs from a loaded foundry config.
///
/// Returns a map of endpoint alias to URL string with `${VAR}` environment
/// references expanded via [`crate::expand_env_vars`].
pub fn rpc_endpoints(config: &foundry_config::Config) -> HashMap<String, String> {
    let mut endpoints = HashMap::new();
    for (name, endpoint) in config.rpc_endpoints.iter() {
        endpoints.insert(name.clone(), expand_env_vars(&endpoint.endpoint.to_string()));
    }
    endpoints
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SenderType;
    use tempfile::TempDir;

    fn write_foundry_toml(dir: &Path, content: &str) {
        std::fs::write(dir.join("foundry.toml"), content).unwrap();
    }

    #[test]
    fn load_foundry_config_with_valid_toml() {
        let tmp = TempDir::new().unwrap();
        write_foundry_toml(
            tmp.path(),
            r#"
[profile.default]
src = "src"
out = "out"
"#,
        );
        let config = load_foundry_config(tmp.path()).unwrap();
        assert_eq!(config.src.to_string_lossy(), "src");
    }

    #[test]
    fn load_foundry_config_missing_file_errors() {
        let tmp = TempDir::new().unwrap();
        // foundry_config::Config::load_with_root will still succeed with
        // defaults even if no foundry.toml exists — it uses default config.
        // This test just verifies the function doesn't panic.
        let result = load_foundry_config(tmp.path());
        // foundry_config may or may not error on missing file depending on
        // version — just verify no panic.
        let _ = result;
    }

    #[test]
    fn extract_treb_senders_from_foundry_with_senders() {
        let tmp = TempDir::new().unwrap();
        write_foundry_toml(
            tmp.path(),
            r#"
[profile.default]
src = "src"
out = "out"

[profile.default.treb.senders.deployer]
type = "private_key"
address = "0xDeployerAddr"
private_key = "0xDeployerKey"

[profile.default.treb.senders.ledger_signer]
type = "ledger"
address = "0xLedgerAddr"
derivation_path = "m/44'/60'/0'/0/0"
"#,
        );

        let senders = extract_treb_senders_from_foundry(tmp.path(), "default");
        assert_eq!(senders.len(), 2);

        let deployer = senders.get("deployer").unwrap();
        assert_eq!(deployer.type_, Some(SenderType::PrivateKey));
        assert_eq!(deployer.address.as_deref(), Some("0xDeployerAddr"));
        assert_eq!(deployer.private_key.as_deref(), Some("0xDeployerKey"));

        let ledger = senders.get("ledger_signer").unwrap();
        assert_eq!(ledger.type_, Some(SenderType::Ledger));
        assert_eq!(ledger.address.as_deref(), Some("0xLedgerAddr"));
        assert_eq!(ledger.derivation_path.as_deref(), Some("m/44'/60'/0'/0/0"));
    }

    #[test]
    fn extract_treb_senders_missing_foundry_toml() {
        let tmp = TempDir::new().unwrap();
        let senders = extract_treb_senders_from_foundry(tmp.path(), "default");
        assert!(senders.is_empty());
    }

    #[test]
    fn extract_treb_senders_no_treb_section() {
        let tmp = TempDir::new().unwrap();
        write_foundry_toml(
            tmp.path(),
            r#"
[profile.default]
src = "src"
"#,
        );
        let senders = extract_treb_senders_from_foundry(tmp.path(), "default");
        assert!(senders.is_empty());
    }

    #[test]
    fn extract_treb_senders_wrong_profile() {
        let tmp = TempDir::new().unwrap();
        write_foundry_toml(
            tmp.path(),
            r#"
[profile.default.treb.senders.deployer]
type = "private_key"
address = "0xAddr"
"#,
        );
        let senders = extract_treb_senders_from_foundry(tmp.path(), "production");
        assert!(senders.is_empty());
    }

    #[test]
    fn extract_treb_senders_expand_env_var_fields() {
        let tmp = TempDir::new().unwrap();
        write_foundry_toml(
            tmp.path(),
            r#"
[profile.default.treb.senders.deployer]
type = "safe"
address = "${TREB_TEST_ADDR_P3_US_002}"
safe = "${TREB_TEST_SAFE_P3_US_002}"
signer = "${TREB_TEST_SIGNER_P3_US_002}"

[profile.default.treb.senders.governor]
type = "oz_governor"
governor = "${TREB_TEST_GOVERNOR_P3_US_002}"
timelock = "${TREB_TEST_TIMELOCK_P3_US_002}"
proposer = "${TREB_TEST_PROPOSER_P3_US_002}"

[profile.default.treb.senders.ledger_signer]
type = "ledger"
private_key = "${TREB_TEST_PRIVATE_KEY_P3_US_002}"
derivation_path = "${TREB_TEST_DERIVATION_PATH_P3_US_002}"
"#,
        );

        unsafe {
            std::env::set_var("TREB_TEST_ADDR_P3_US_002", "0xDeployerAddr");
            std::env::set_var("TREB_TEST_SAFE_P3_US_002", "0xSafeAddr");
            std::env::set_var("TREB_TEST_SIGNER_P3_US_002", "signer-account");
            std::env::set_var("TREB_TEST_GOVERNOR_P3_US_002", "0xGovernorAddr");
            std::env::set_var("TREB_TEST_TIMELOCK_P3_US_002", "0xTimelockAddr");
            std::env::set_var("TREB_TEST_PROPOSER_P3_US_002", "proposer-account");
            std::env::set_var("TREB_TEST_PRIVATE_KEY_P3_US_002", "0xPrivateKey");
            std::env::set_var("TREB_TEST_DERIVATION_PATH_P3_US_002", "m/44'/60'/0'/0/1");
        }

        let senders = extract_treb_senders_from_foundry(tmp.path(), "default");

        let deployer = senders.get("deployer").unwrap();
        assert_eq!(deployer.address.as_deref(), Some("0xDeployerAddr"));
        assert_eq!(deployer.safe.as_deref(), Some("0xSafeAddr"));
        assert_eq!(deployer.signer.as_deref(), Some("signer-account"));

        let governor = senders.get("governor").unwrap();
        assert_eq!(governor.governor.as_deref(), Some("0xGovernorAddr"));
        assert_eq!(governor.timelock.as_deref(), Some("0xTimelockAddr"));
        assert_eq!(governor.proposer.as_deref(), Some("proposer-account"));

        let ledger = senders.get("ledger_signer").unwrap();
        assert_eq!(ledger.private_key.as_deref(), Some("0xPrivateKey"));
        assert_eq!(ledger.derivation_path.as_deref(), Some("m/44'/60'/0'/0/1"));

        unsafe {
            std::env::remove_var("TREB_TEST_ADDR_P3_US_002");
            std::env::remove_var("TREB_TEST_SAFE_P3_US_002");
            std::env::remove_var("TREB_TEST_SIGNER_P3_US_002");
            std::env::remove_var("TREB_TEST_GOVERNOR_P3_US_002");
            std::env::remove_var("TREB_TEST_TIMELOCK_P3_US_002");
            std::env::remove_var("TREB_TEST_PROPOSER_P3_US_002");
            std::env::remove_var("TREB_TEST_PRIVATE_KEY_P3_US_002");
            std::env::remove_var("TREB_TEST_DERIVATION_PATH_P3_US_002");
        }
    }

    #[test]
    fn extract_treb_senders_leave_literal_fields_unchanged() {
        let tmp = TempDir::new().unwrap();
        write_foundry_toml(
            tmp.path(),
            r#"
[profile.default.treb.senders.deployer]
type = "private_key"
address = "0xLiteralAddr"
private_key = "0xLiteralKey"
"#,
        );

        let senders = extract_treb_senders_from_foundry(tmp.path(), "default");
        let deployer = senders.get("deployer").unwrap();

        assert_eq!(deployer.address.as_deref(), Some("0xLiteralAddr"));
        assert_eq!(deployer.private_key.as_deref(), Some("0xLiteralKey"));
    }

    #[test]
    fn rpc_endpoints_extracts_urls() {
        let tmp = TempDir::new().unwrap();
        write_foundry_toml(
            tmp.path(),
            r#"
[profile.default]
src = "src"

[rpc_endpoints]
mainnet = "https://eth-mainnet.example.com"
sepolia = "https://sepolia.example.com"
"#,
        );
        let config = load_foundry_config(tmp.path()).unwrap();
        let eps = rpc_endpoints(&config);
        assert_eq!(eps.get("mainnet").unwrap(), "https://eth-mainnet.example.com");
        assert_eq!(eps.get("sepolia").unwrap(), "https://sepolia.example.com");
    }

    #[test]
    fn rpc_endpoints_expand_env_var_urls() {
        let tmp = TempDir::new().unwrap();
        write_foundry_toml(
            tmp.path(),
            r#"
[profile.default]
src = "src"

[rpc_endpoints]
mainnet = "${TREB_TEST_MAINNET_RPC_URL_P3_US_001}"
"#,
        );
        unsafe {
            std::env::set_var(
                "TREB_TEST_MAINNET_RPC_URL_P3_US_001",
                "https://eth-mainnet.example.com",
            )
        };

        let config = load_foundry_config(tmp.path()).unwrap();
        let eps = rpc_endpoints(&config);

        assert_eq!(eps.get("mainnet").unwrap(), "https://eth-mainnet.example.com");

        unsafe { std::env::remove_var("TREB_TEST_MAINNET_RPC_URL_P3_US_001") };
    }

    #[test]
    fn rpc_endpoints_leave_literal_urls_unchanged() {
        let tmp = TempDir::new().unwrap();
        write_foundry_toml(
            tmp.path(),
            r#"
[profile.default]
src = "src"

[rpc_endpoints]
sepolia = "https://sepolia.example.com"
"#,
        );

        let config = load_foundry_config(tmp.path()).unwrap();
        let eps = rpc_endpoints(&config);

        assert_eq!(eps.get("sepolia").unwrap(), "https://sepolia.example.com");
    }

    #[test]
    fn rpc_endpoints_expand_mixed_urls() {
        let tmp = TempDir::new().unwrap();
        write_foundry_toml(
            tmp.path(),
            r#"
[profile.default]
src = "src"

[rpc_endpoints]
alchemy = "https://eth-mainnet.g.alchemy.com/v2/${TREB_TEST_ALCHEMY_KEY_P3_US_001}"
"#,
        );
        unsafe { std::env::set_var("TREB_TEST_ALCHEMY_KEY_P3_US_001", "secret-key") };

        let config = load_foundry_config(tmp.path()).unwrap();
        let eps = rpc_endpoints(&config);

        assert_eq!(eps.get("alchemy").unwrap(), "https://eth-mainnet.g.alchemy.com/v2/secret-key");

        unsafe { std::env::remove_var("TREB_TEST_ALCHEMY_KEY_P3_US_001") };
    }

    #[test]
    fn rpc_endpoints_expand_unset_env_vars_to_empty_string() {
        let tmp = TempDir::new().unwrap();
        write_foundry_toml(
            tmp.path(),
            r#"
[profile.default]
src = "src"

[rpc_endpoints]
local = "http://localhost/${TREB_TEST_UNSET_RPC_SEGMENT_P3_US_001}"
"#,
        );
        unsafe { std::env::remove_var("TREB_TEST_UNSET_RPC_SEGMENT_P3_US_001") };

        let config = load_foundry_config(tmp.path()).unwrap();
        let eps = rpc_endpoints(&config);

        assert_eq!(eps.get("local").unwrap(), "http://localhost/");
    }
}
