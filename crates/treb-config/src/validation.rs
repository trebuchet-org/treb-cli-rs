//! Config validation and error reporting.
//!
//! Validates a `ResolvedConfig` and its individual senders, returning
//! actionable errors and warnings that tell users exactly what to fix.

use std::collections::HashMap;

use treb_core::error::{Result, TrebError};

use crate::{ConfigWarning, ResolvedConfig, SenderConfig, SenderType};

/// Validate the full resolved config, checking all sender cross-references
/// and field requirements.
///
/// Returns `Ok(warnings)` if the config is usable (possibly with warnings),
/// or `Err` if there are hard errors that prevent operation.
pub fn validate_config(config: &ResolvedConfig) -> Result<Vec<ConfigWarning>> {
    let mut warnings = Vec::new();

    for (name, sender) in &config.senders {
        warnings.extend(validate_sender(name, sender, &config.senders, &config.config_source)?);
    }

    Ok(warnings)
}

/// Validate a single sender configuration.
///
/// Checks type-specific field requirements and cross-references to other
/// senders. Returns warnings for non-fatal issues and errors for things
/// that will prevent operation.
pub fn validate_sender(
    name: &str,
    sender: &SenderConfig,
    all_senders: &HashMap<String, SenderConfig>,
    config_source: &str,
) -> Result<Vec<ConfigWarning>> {
    let mut warnings = Vec::new();
    let location = format!("[senders.{}] in {}", name, config_source);

    let Some(ref sender_type) = sender.type_ else {
        return Err(TrebError::Config(format!(
            "{location}: sender '{name}' has no 'type' field. \
             Add `type = \"private_key\"` (or ledger, trezor, safe, governance) to fix."
        )));
    };

    match sender_type {
        SenderType::PrivateKey => {
            if sender.private_key.is_none() {
                return Err(TrebError::Config(format!(
                    "{location}: private_key sender '{name}' is missing the 'private_key' field. \
                     Add `private_key = \"0x...\"` or `private_key = \"${{ENV_VAR}}\"` to fix."
                )));
            }
        }
        SenderType::Ledger => {
            if sender.address.is_none() {
                warnings.push(ConfigWarning {
                    message: format!(
                        "ledger sender '{name}' has no 'address' field — \
                         the address will need to be provided at runtime. \
                         Add `address = \"0x...\"` to suppress this warning."
                    ),
                    location: location.clone(),
                });
            }
            if sender.derivation_path.is_none() {
                warnings.push(ConfigWarning {
                    message: format!(
                        "ledger sender '{name}' has no 'derivation_path' field — \
                         the default path will be used. \
                         Add `derivation_path = \"m/44'/60'/0'/0/0\"` to suppress this warning."
                    ),
                    location: location.clone(),
                });
            }
        }
        SenderType::Trezor => {
            if sender.address.is_none() {
                warnings.push(ConfigWarning {
                    message: format!(
                        "trezor sender '{name}' has no 'address' field — \
                         the address will need to be provided at runtime. \
                         Add `address = \"0x...\"` to suppress this warning."
                    ),
                    location: location.clone(),
                });
            }
            if sender.derivation_path.is_none() {
                warnings.push(ConfigWarning {
                    message: format!(
                        "trezor sender '{name}' has no 'derivation_path' field — \
                         the default path will be used. \
                         Add `derivation_path = \"m/44'/60'/0'/0/0\"` to suppress this warning."
                    ),
                    location: location.clone(),
                });
            }
        }
        SenderType::Safe => {
            if sender.safe.is_none() {
                return Err(TrebError::Config(format!(
                    "{location}: safe sender '{name}' is missing the 'safe' field. \
                     Add `safe = \"0xSafeAddress\"` to fix."
                )));
            }
            match &sender.signer {
                None => {
                    return Err(TrebError::Config(format!(
                        "{location}: safe sender '{name}' is missing the 'signer' field. \
                         Add `signer = \"<sender_name>\"` referencing an existing sender to fix."
                    )));
                }
                Some(signer_name) => {
                    if !all_senders.contains_key(signer_name) {
                        return Err(TrebError::Config(format!(
                            "{location}: safe sender '{name}' references signer '{signer_name}' \
                             which does not exist. Define a sender named '{signer_name}' or \
                             update the signer field to reference an existing sender."
                        )));
                    }
                }
            }
        }
        SenderType::Governance => {
            if sender.address.is_none() {
                return Err(TrebError::Config(format!(
                    "{location}: governance sender '{name}' is missing the 'address' field. \
                     Add `address = \"0xTimelockOrGovernorAddress\"` to fix."
                )));
            }
            match &sender.proposer {
                None => {
                    return Err(TrebError::Config(format!(
                        "{location}: governance sender '{name}' is missing the 'proposer' field. \
                         Add `proposer = \"<sender_name>\"` referencing an existing sender to fix."
                    )));
                }
                Some(proposer_name) => {
                    if !all_senders.contains_key(proposer_name) {
                        return Err(TrebError::Config(format!(
                            "{location}: governance sender '{name}' references proposer \
                             '{proposer_name}' which does not exist. Define a sender named \
                             '{proposer_name}' or update the proposer field to reference an \
                             existing sender."
                        )));
                    }
                }
            }
        }
    }

    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn make_resolved(senders: HashMap<String, SenderConfig>) -> ResolvedConfig {
        ResolvedConfig {
            namespace: "default".to_string(),
            network: None,
            profile: "default".to_string(),
            senders,
            slow: false,
            fork_setup: None,
            config_source: "treb.toml (v2)".to_string(),
            project_root: PathBuf::from("/tmp/test"),
        }
    }

    // ---- Valid configs pass ----

    #[test]
    fn valid_private_key_sender_passes() {
        let mut senders = HashMap::new();
        senders.insert(
            "deployer".to_string(),
            SenderConfig {
                type_: Some(SenderType::PrivateKey),
                private_key: Some("0xkey".to_string()),
                ..Default::default()
            },
        );
        let config = make_resolved(senders);
        let warnings = validate_config(&config).unwrap();
        assert!(warnings.is_empty());
    }

    #[test]
    fn valid_safe_sender_passes() {
        let mut senders = HashMap::new();
        senders.insert(
            "deployer".to_string(),
            SenderConfig {
                type_: Some(SenderType::PrivateKey),
                private_key: Some("0xkey".to_string()),
                ..Default::default()
            },
        );
        senders.insert(
            "multisig".to_string(),
            SenderConfig {
                type_: Some(SenderType::Safe),
                safe: Some("0xSafeAddr".to_string()),
                signer: Some("deployer".to_string()),
                ..Default::default()
            },
        );
        let config = make_resolved(senders);
        let warnings = validate_config(&config).unwrap();
        assert!(warnings.is_empty());
    }

    #[test]
    fn valid_governance_sender_passes() {
        let mut senders = HashMap::new();
        senders.insert(
            "deployer".to_string(),
            SenderConfig {
                type_: Some(SenderType::PrivateKey),
                private_key: Some("0xkey".to_string()),
                ..Default::default()
            },
        );
        senders.insert(
            "gov".to_string(),
            SenderConfig {
                type_: Some(SenderType::Governance),
                address: Some("0xTimelockAddr".to_string()),
                proposer: Some("deployer".to_string()),
                ..Default::default()
            },
        );
        let config = make_resolved(senders);
        let warnings = validate_config(&config).unwrap();
        assert!(warnings.is_empty());
    }

    #[test]
    fn valid_ledger_with_all_fields_no_warnings() {
        let mut senders = HashMap::new();
        senders.insert(
            "hw".to_string(),
            SenderConfig {
                type_: Some(SenderType::Ledger),
                address: Some("0xAddr".to_string()),
                derivation_path: Some("m/44'/60'/0'/0/0".to_string()),
                ..Default::default()
            },
        );
        let config = make_resolved(senders);
        let warnings = validate_config(&config).unwrap();
        assert!(warnings.is_empty());
    }

    #[test]
    fn empty_senders_passes() {
        let config = make_resolved(HashMap::new());
        let warnings = validate_config(&config).unwrap();
        assert!(warnings.is_empty());
    }

    // ---- Errors ----

    #[test]
    fn private_key_without_key_errors() {
        let mut senders = HashMap::new();
        senders.insert(
            "deployer".to_string(),
            SenderConfig { type_: Some(SenderType::PrivateKey), ..Default::default() },
        );
        let config = make_resolved(senders);
        let err = validate_config(&config).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("deployer"), "should mention sender name: {msg}");
        assert!(msg.contains("private_key"), "should mention missing field: {msg}");
        assert!(msg.contains("fix"), "should contain fix instructions: {msg}");
    }

    #[test]
    fn safe_without_safe_field_errors() {
        let mut senders = HashMap::new();
        senders.insert(
            "multisig".to_string(),
            SenderConfig {
                type_: Some(SenderType::Safe),
                signer: Some("deployer".to_string()),
                ..Default::default()
            },
        );
        let config = make_resolved(senders);
        let err = validate_config(&config).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("multisig"), "should mention sender name: {msg}");
        assert!(msg.contains("safe"), "should mention missing field: {msg}");
        assert!(msg.contains("fix"), "should contain fix instructions: {msg}");
    }

    #[test]
    fn safe_without_signer_errors() {
        let mut senders = HashMap::new();
        senders.insert(
            "multisig".to_string(),
            SenderConfig {
                type_: Some(SenderType::Safe),
                safe: Some("0xSafeAddr".to_string()),
                ..Default::default()
            },
        );
        let config = make_resolved(senders);
        let err = validate_config(&config).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("multisig"), "should mention sender name: {msg}");
        assert!(msg.contains("signer"), "should mention missing field: {msg}");
        assert!(msg.contains("fix"), "should contain fix instructions: {msg}");
    }

    #[test]
    fn safe_with_missing_signer_reference_errors() {
        let mut senders = HashMap::new();
        senders.insert(
            "multisig".to_string(),
            SenderConfig {
                type_: Some(SenderType::Safe),
                safe: Some("0xSafeAddr".to_string()),
                signer: Some("nonexistent".to_string()),
                ..Default::default()
            },
        );
        let config = make_resolved(senders);
        let err = validate_config(&config).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("multisig"), "should mention sender name: {msg}");
        assert!(msg.contains("nonexistent"), "should mention missing reference: {msg}");
        assert!(msg.contains("does not exist"), "should explain the issue: {msg}");
    }

    #[test]
    fn governance_without_address_field_errors() {
        let mut senders = HashMap::new();
        senders.insert(
            "gov".to_string(),
            SenderConfig {
                type_: Some(SenderType::Governance),
                proposer: Some("deployer".to_string()),
                ..Default::default()
            },
        );
        let config = make_resolved(senders);
        let err = validate_config(&config).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("gov"), "should mention sender name: {msg}");
        assert!(msg.contains("address"), "should mention missing field: {msg}");
        assert!(msg.contains("fix"), "should contain fix instructions: {msg}");
    }

    #[test]
    fn governance_without_proposer_errors() {
        let mut senders = HashMap::new();
        senders.insert(
            "gov".to_string(),
            SenderConfig {
                type_: Some(SenderType::Governance),
                address: Some("0xTimelockAddr".to_string()),
                ..Default::default()
            },
        );
        let config = make_resolved(senders);
        let err = validate_config(&config).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("gov"), "should mention sender name: {msg}");
        assert!(msg.contains("proposer"), "should mention missing field: {msg}");
        assert!(msg.contains("fix"), "should contain fix instructions: {msg}");
    }

    #[test]
    fn governance_with_missing_proposer_reference_errors() {
        let mut senders = HashMap::new();
        senders.insert(
            "gov".to_string(),
            SenderConfig {
                type_: Some(SenderType::Governance),
                address: Some("0xTimelockAddr".to_string()),
                proposer: Some("ghost".to_string()),
                ..Default::default()
            },
        );
        let config = make_resolved(senders);
        let err = validate_config(&config).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("gov"), "should mention sender name: {msg}");
        assert!(msg.contains("ghost"), "should mention missing reference: {msg}");
        assert!(msg.contains("does not exist"), "should explain the issue: {msg}");
    }

    #[test]
    fn sender_without_type_errors() {
        let mut senders = HashMap::new();
        senders.insert("broken".to_string(), SenderConfig::default());
        let config = make_resolved(senders);
        let err = validate_config(&config).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("broken"), "should mention sender name: {msg}");
        assert!(msg.contains("type"), "should mention missing type: {msg}");
        assert!(msg.contains("fix"), "should contain fix instructions: {msg}");
    }

    // ---- Warnings ----

    #[test]
    fn ledger_without_address_warns() {
        let mut senders = HashMap::new();
        senders.insert(
            "hw".to_string(),
            SenderConfig { type_: Some(SenderType::Ledger), ..Default::default() },
        );
        let config = make_resolved(senders);
        let warnings = validate_config(&config).unwrap();
        assert!(!warnings.is_empty(), "should have warnings");
        let msgs: Vec<&str> = warnings.iter().map(|w| w.message.as_str()).collect();
        assert!(
            msgs.iter().any(|m| m.contains("address")),
            "should warn about missing address: {msgs:?}"
        );
        assert!(
            msgs.iter().any(|m| m.contains("derivation_path")),
            "should warn about missing derivation_path: {msgs:?}"
        );
        // Warnings include location info.
        assert!(
            warnings.iter().all(|w| w.location.contains("hw")),
            "warning locations should mention sender name"
        );
    }

    #[test]
    fn trezor_without_address_warns() {
        let mut senders = HashMap::new();
        senders.insert(
            "hw".to_string(),
            SenderConfig { type_: Some(SenderType::Trezor), ..Default::default() },
        );
        let config = make_resolved(senders);
        let warnings = validate_config(&config).unwrap();
        assert!(!warnings.is_empty(), "should have warnings for trezor");
        assert!(
            warnings.iter().any(|w| w.message.contains("address")),
            "should warn about missing address"
        );
    }

    // ---- Error messages contain fix instructions ----

    #[test]
    fn error_messages_contain_source_and_fix() {
        let mut senders = HashMap::new();
        senders.insert(
            "deployer".to_string(),
            SenderConfig { type_: Some(SenderType::PrivateKey), ..Default::default() },
        );
        let config = make_resolved(senders);
        let err = validate_config(&config).unwrap_err();
        let msg = err.to_string();
        // Should contain the config source.
        assert!(msg.contains("treb.toml (v2)"), "error should mention config source: {msg}");
        // Should contain the sender section.
        assert!(msg.contains("[senders.deployer]"), "error should mention section: {msg}");
        // Should contain fix instructions.
        assert!(msg.contains("Add"), "error should contain fix instructions: {msg}");
    }

    #[test]
    fn warning_locations_contain_source() {
        let mut senders = HashMap::new();
        senders.insert(
            "hw".to_string(),
            SenderConfig { type_: Some(SenderType::Ledger), ..Default::default() },
        );
        let config = make_resolved(senders);
        let warnings = validate_config(&config).unwrap();
        assert!(
            warnings.iter().all(|w| w.location.contains("treb.toml (v2)")),
            "warning locations should contain config source"
        );
    }
}
