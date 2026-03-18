//! Configuration data types for treb.
//!
//! Defines all config structures used across treb: sender types, account
//! configs, namespace roles, treb.toml v1/v2 file formats, local config,
//! and the final resolved configuration. String representations of
//! `SenderType` match the Go implementation exactly.

use std::{collections::HashMap, fmt, path::PathBuf, str::FromStr};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// SenderType
// ---------------------------------------------------------------------------

/// The type of sender/signer used for transactions.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SenderType {
    #[serde(rename = "private_key")]
    PrivateKey,
    #[serde(rename = "ledger")]
    Ledger,
    #[serde(rename = "trezor")]
    Trezor,
    #[serde(rename = "safe")]
    Safe,
    #[serde(rename = "governance")]
    Governance,
}

impl fmt::Display for SenderType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PrivateKey => write!(f, "private_key"),
            Self::Ledger => write!(f, "ledger"),
            Self::Trezor => write!(f, "trezor"),
            Self::Safe => write!(f, "safe"),
            Self::Governance => write!(f, "governance"),
        }
    }
}

impl FromStr for SenderType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "private_key" => Ok(Self::PrivateKey),
            "ledger" => Ok(Self::Ledger),
            "trezor" => Ok(Self::Trezor),
            "safe" => Ok(Self::Safe),
            "governance" => Ok(Self::Governance),
            other => Err(format!("unknown SenderType: {other}")),
        }
    }
}

// ---------------------------------------------------------------------------
// SenderConfig
// ---------------------------------------------------------------------------

/// Configuration for a single sender/signer.
///
/// The `type_` field is serialized as `"type"` in TOML/JSON to match Go.
/// All fields except `type_` are optional — required fields depend on the
/// sender type and are validated separately.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SenderConfig {
    #[serde(rename = "type")]
    pub type_: Option<SenderType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safe: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derivation_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timelock: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposer_script: Option<String>,
}

// ---------------------------------------------------------------------------
// AccountConfig
// ---------------------------------------------------------------------------

/// Account configuration for treb.toml v2 `[accounts.*]` sections.
///
/// Same shape as `SenderConfig` — used in the v2 format where accounts are
/// defined globally and referenced by name from namespace sender roles.
pub type AccountConfig = SenderConfig;

// ---------------------------------------------------------------------------
// NamespaceRoles
// ---------------------------------------------------------------------------

/// Namespace configuration mapping role names to account names.
///
/// Used in treb.toml v2 `[namespace.*]` sections.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceRoles {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default)]
    pub senders: HashMap<String, String>,
}

// ---------------------------------------------------------------------------
// ForkConfig
// ---------------------------------------------------------------------------

/// Fork-mode configuration from treb.toml v2 `[fork]` section.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<String>,
}

// ---------------------------------------------------------------------------
// TrebFileConfigV2
// ---------------------------------------------------------------------------

/// Parsed treb.toml v2 format with `[accounts.*]`, `[namespace.*]`,
/// and `[fork]` sections.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrebFileConfigV2 {
    #[serde(default)]
    pub accounts: HashMap<String, AccountConfig>,
    #[serde(default)]
    pub namespace: HashMap<String, NamespaceRoles>,
    #[serde(default)]
    pub fork: ForkConfig,
}

// ---------------------------------------------------------------------------
// TrebFileConfigV1 (legacy read-only)
// ---------------------------------------------------------------------------

/// Parsed treb.toml v1 format with `[ns.*]` sections.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrebFileConfigV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slow: Option<bool>,
    #[serde(default)]
    pub ns: HashMap<String, NamespaceConfigV1>,
}

/// A single namespace in the v1 config format.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceConfigV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slow: Option<bool>,
    #[serde(default)]
    pub senders: HashMap<String, SenderConfig>,
}

// ---------------------------------------------------------------------------
// LocalConfig
// ---------------------------------------------------------------------------

/// Local user config stored in `.treb/config.local.json`.
///
/// Contains the user's default namespace and network selections.
/// Field names (`namespace`, `network`) match the Go CLI's LocalConfig
/// schema exactly — no camelCase rename is needed since both fields are
/// already single lowercase words.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalConfig {
    pub namespace: String,
    pub network: String,
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self { namespace: "default".to_string(), network: String::new() }
    }
}

// ---------------------------------------------------------------------------
// ResolvedConfig
// ---------------------------------------------------------------------------

/// The fully resolved configuration after merging all sources.
///
/// Produced by the layered config resolver, this struct contains the final
/// effective settings that downstream commands use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedConfig {
    pub namespace: String,
    pub network: Option<String>,
    pub profile: String,
    pub senders: HashMap<String, SenderConfig>,
    pub slow: bool,
    pub fork_setup: Option<String>,
    pub config_source: String,
    pub project_root: PathBuf,
}

// ---------------------------------------------------------------------------
// ResolvedSenders (used by v1→resolved conversion)
// ---------------------------------------------------------------------------

/// Intermediate result from resolving v1 namespace senders.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolvedSenders {
    pub profile: String,
    pub senders: HashMap<String, SenderConfig>,
    pub slow: bool,
}

// ---------------------------------------------------------------------------
// TrebConfigFormat
// ---------------------------------------------------------------------------

/// Detected format of a treb.toml file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrebConfigFormat {
    /// No treb.toml file found.
    None,
    /// Legacy v1 format with `[ns.*]` sections.
    V1,
    /// Current v2 format with `[accounts.*]`/`[namespace.*]`/`[fork]` sections.
    V2,
}

// ---------------------------------------------------------------------------
// ConfigWarning
// ---------------------------------------------------------------------------

/// A non-fatal warning from config validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigWarning {
    pub message: String,
    pub location: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- SenderType serde round-trip ----

    #[test]
    fn sender_type_serde_round_trip() {
        for (variant, expected) in [
            (SenderType::PrivateKey, "\"private_key\""),
            (SenderType::Ledger, "\"ledger\""),
            (SenderType::Trezor, "\"trezor\""),
            (SenderType::Safe, "\"safe\""),
            (SenderType::Governance, "\"governance\""),
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, expected);
            let parsed: SenderType = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn sender_type_toml_round_trip() {
        // TOML wraps values; test via a wrapper struct.
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Wrapper {
            t: SenderType,
        }
        for variant in [
            SenderType::PrivateKey,
            SenderType::Ledger,
            SenderType::Trezor,
            SenderType::Safe,
            SenderType::Governance,
        ] {
            let w = Wrapper { t: variant.clone() };
            let s = toml::to_string(&w).unwrap();
            let parsed: Wrapper = toml::from_str(&s).unwrap();
            assert_eq!(parsed.t, variant);
        }
    }

    // ---- SenderType Display / FromStr ----

    #[test]
    fn sender_type_display_and_from_str() {
        for (variant, s) in [
            (SenderType::PrivateKey, "private_key"),
            (SenderType::Ledger, "ledger"),
            (SenderType::Trezor, "trezor"),
            (SenderType::Safe, "safe"),
            (SenderType::Governance, "governance"),
        ] {
            assert_eq!(variant.to_string(), s);
            assert_eq!(s.parse::<SenderType>().unwrap(), variant);
        }
        assert!("INVALID".parse::<SenderType>().is_err());
    }

    // ---- SenderConfig serde ----

    #[test]
    fn sender_config_json_round_trip() {
        let cfg = SenderConfig {
            type_: Some(SenderType::PrivateKey),
            address: Some("0x1234".to_string()),
            private_key: Some("0xabcd".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: SenderConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, cfg);
    }

    #[test]
    fn sender_config_toml_round_trip() {
        let cfg = SenderConfig {
            type_: Some(SenderType::Safe),
            safe: Some("0xsafe".to_string()),
            signer: Some("deployer".to_string()),
            ..Default::default()
        };
        let toml_str = toml::to_string(&cfg).unwrap();
        let parsed: SenderConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed, cfg);
    }

    // ---- LocalConfig defaults ----

    #[test]
    fn local_config_default() {
        let cfg = LocalConfig::default();
        assert_eq!(cfg.namespace, "default");
        assert_eq!(cfg.network, "");
    }

    #[test]
    fn local_config_json_format() {
        let cfg = LocalConfig::default();
        let json = serde_json::to_string_pretty(&cfg).unwrap();
        // Verify the JSON contains the expected keys.
        assert!(json.contains("\"namespace\""));
        assert!(json.contains("\"network\""));
    }

    /// Verify that config.local.json field names match the Go CLI schema.
    ///
    /// Go's LocalConfig uses exactly `{"namespace": "...", "network": "..."}`.
    /// This test ensures the Rust struct serializes with identical JSON keys.
    #[test]
    fn local_config_field_names_match_go_schema() {
        let cfg = LocalConfig { namespace: "prod".to_string(), network: "mainnet".to_string() };
        let value = serde_json::to_value(&cfg).unwrap();
        let obj = value.as_object().unwrap();

        // Must have exactly the Go-compatible keys.
        assert!(obj.contains_key("namespace"), "missing 'namespace' key");
        assert!(obj.contains_key("network"), "missing 'network' key");

        // Must NOT have any extra keys (Rust-only fields would break Go).
        assert_eq!(obj.len(), 2, "expected exactly 2 keys, got: {obj:?}");

        // Values round-trip correctly.
        assert_eq!(obj["namespace"], "prod");
        assert_eq!(obj["network"], "mainnet");
    }

    // ---- TrebFileConfigV2 defaults ----

    #[test]
    fn treb_file_config_v2_default_parses_empty_toml() {
        let cfg: TrebFileConfigV2 = toml::from_str("").unwrap();
        assert!(cfg.accounts.is_empty());
        assert!(cfg.namespace.is_empty());
        assert_eq!(cfg.fork, ForkConfig::default());
    }

    // ---- TrebFileConfigV1 defaults ----

    #[test]
    fn treb_file_config_v1_default_parses_empty_toml() {
        let cfg: TrebFileConfigV1 = toml::from_str("").unwrap();
        assert!(cfg.slow.is_none());
        assert!(cfg.ns.is_empty());
    }
}
