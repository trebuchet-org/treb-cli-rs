//! Domain enums shared across treb models.
//!
//! String representations match the Go implementation exactly.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// DeploymentType
// ---------------------------------------------------------------------------

/// The kind of deployment (singleton, proxy, library, etc.).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeploymentType {
    #[serde(rename = "SINGLETON")]
    Singleton,
    #[serde(rename = "PROXY")]
    Proxy,
    #[serde(rename = "LIBRARY")]
    Library,
    #[serde(rename = "UNKNOWN")]
    Unknown,
}

impl fmt::Display for DeploymentType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Singleton => write!(f, "SINGLETON"),
            Self::Proxy => write!(f, "PROXY"),
            Self::Library => write!(f, "LIBRARY"),
            Self::Unknown => write!(f, "UNKNOWN"),
        }
    }
}

impl FromStr for DeploymentType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "SINGLETON" => Ok(Self::Singleton),
            "PROXY" => Ok(Self::Proxy),
            "LIBRARY" => Ok(Self::Library),
            "UNKNOWN" => Ok(Self::Unknown),
            other => Err(format!("unknown DeploymentType: {other}")),
        }
    }
}

// ---------------------------------------------------------------------------
// DeploymentMethod
// ---------------------------------------------------------------------------

/// The EVM opcode used to deploy a contract.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeploymentMethod {
    #[serde(rename = "CREATE")]
    Create,
    #[serde(rename = "CREATE2")]
    Create2,
    #[serde(rename = "CREATE3")]
    Create3,
}

impl fmt::Display for DeploymentMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Create => write!(f, "CREATE"),
            Self::Create2 => write!(f, "CREATE2"),
            Self::Create3 => write!(f, "CREATE3"),
        }
    }
}

impl FromStr for DeploymentMethod {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "CREATE" => Ok(Self::Create),
            "CREATE2" => Ok(Self::Create2),
            "CREATE3" => Ok(Self::Create3),
            other => Err(format!("unknown DeploymentMethod: {other}")),
        }
    }
}

// ---------------------------------------------------------------------------
// TransactionStatus
// ---------------------------------------------------------------------------

/// Lifecycle status of a transaction.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransactionStatus {
    #[serde(rename = "SIMULATED")]
    Simulated,
    #[serde(rename = "QUEUED")]
    Queued,
    #[serde(rename = "EXECUTED")]
    Executed,
    #[serde(rename = "FAILED")]
    Failed,
}

impl fmt::Display for TransactionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Simulated => write!(f, "SIMULATED"),
            Self::Queued => write!(f, "QUEUED"),
            Self::Executed => write!(f, "EXECUTED"),
            Self::Failed => write!(f, "FAILED"),
        }
    }
}

impl FromStr for TransactionStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "SIMULATED" => Ok(Self::Simulated),
            "QUEUED" => Ok(Self::Queued),
            "EXECUTED" => Ok(Self::Executed),
            "FAILED" => Ok(Self::Failed),
            other => Err(format!("unknown TransactionStatus: {other}")),
        }
    }
}

// ---------------------------------------------------------------------------
// VerificationStatus
// ---------------------------------------------------------------------------

/// Status of contract source-code verification on a block explorer.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VerificationStatus {
    #[serde(rename = "UNVERIFIED")]
    Unverified,
    #[serde(rename = "VERIFIED")]
    Verified,
    #[serde(rename = "FAILED")]
    Failed,
    #[serde(rename = "PARTIAL")]
    Partial,
}

impl fmt::Display for VerificationStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unverified => write!(f, "UNVERIFIED"),
            Self::Verified => write!(f, "VERIFIED"),
            Self::Failed => write!(f, "FAILED"),
            Self::Partial => write!(f, "PARTIAL"),
        }
    }
}

impl FromStr for VerificationStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "UNVERIFIED" => Ok(Self::Unverified),
            "VERIFIED" => Ok(Self::Verified),
            "FAILED" => Ok(Self::Failed),
            "PARTIAL" => Ok(Self::Partial),
            other => Err(format!("unknown VerificationStatus: {other}")),
        }
    }
}

// ---------------------------------------------------------------------------
// ProposalStatus
// ---------------------------------------------------------------------------

/// Governance proposal lifecycle status (lowercase to match Go constants).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProposalStatus {
    #[serde(rename = "pending")]
    Pending,
    #[serde(rename = "active")]
    Active,
    #[serde(rename = "succeeded")]
    Succeeded,
    #[serde(rename = "queued")]
    Queued,
    #[serde(rename = "executed")]
    Executed,
    #[serde(rename = "canceled")]
    Canceled,
    #[serde(rename = "defeated")]
    Defeated,
}

impl fmt::Display for ProposalStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Active => write!(f, "active"),
            Self::Succeeded => write!(f, "succeeded"),
            Self::Queued => write!(f, "queued"),
            Self::Executed => write!(f, "executed"),
            Self::Canceled => write!(f, "canceled"),
            Self::Defeated => write!(f, "defeated"),
        }
    }
}

impl FromStr for ProposalStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "active" => Ok(Self::Active),
            "succeeded" => Ok(Self::Succeeded),
            "queued" => Ok(Self::Queued),
            "executed" => Ok(Self::Executed),
            "canceled" => Ok(Self::Canceled),
            "defeated" => Ok(Self::Defeated),
            other => Err(format!("unknown ProposalStatus: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- DeploymentType ----

    #[test]
    fn deployment_type_serde_round_trip() {
        for (variant, expected) in [
            (DeploymentType::Singleton, "\"SINGLETON\""),
            (DeploymentType::Proxy, "\"PROXY\""),
            (DeploymentType::Library, "\"LIBRARY\""),
            (DeploymentType::Unknown, "\"UNKNOWN\""),
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, expected);
            let parsed: DeploymentType = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn deployment_type_display_and_from_str() {
        for (variant, s) in [
            (DeploymentType::Singleton, "SINGLETON"),
            (DeploymentType::Proxy, "PROXY"),
            (DeploymentType::Library, "LIBRARY"),
            (DeploymentType::Unknown, "UNKNOWN"),
        ] {
            assert_eq!(variant.to_string(), s);
            assert_eq!(s.parse::<DeploymentType>().unwrap(), variant);
        }
        assert!("INVALID".parse::<DeploymentType>().is_err());
    }

    // ---- DeploymentMethod ----

    #[test]
    fn deployment_method_serde_round_trip() {
        for (variant, expected) in [
            (DeploymentMethod::Create, "\"CREATE\""),
            (DeploymentMethod::Create2, "\"CREATE2\""),
            (DeploymentMethod::Create3, "\"CREATE3\""),
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, expected);
            let parsed: DeploymentMethod = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn deployment_method_display_and_from_str() {
        for (variant, s) in [
            (DeploymentMethod::Create, "CREATE"),
            (DeploymentMethod::Create2, "CREATE2"),
            (DeploymentMethod::Create3, "CREATE3"),
        ] {
            assert_eq!(variant.to_string(), s);
            assert_eq!(s.parse::<DeploymentMethod>().unwrap(), variant);
        }
        assert!("INVALID".parse::<DeploymentMethod>().is_err());
    }

    // ---- TransactionStatus ----

    #[test]
    fn transaction_status_serde_round_trip() {
        for (variant, expected) in [
            (TransactionStatus::Simulated, "\"SIMULATED\""),
            (TransactionStatus::Queued, "\"QUEUED\""),
            (TransactionStatus::Executed, "\"EXECUTED\""),
            (TransactionStatus::Failed, "\"FAILED\""),
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, expected);
            let parsed: TransactionStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn transaction_status_display_and_from_str() {
        for (variant, s) in [
            (TransactionStatus::Simulated, "SIMULATED"),
            (TransactionStatus::Queued, "QUEUED"),
            (TransactionStatus::Executed, "EXECUTED"),
            (TransactionStatus::Failed, "FAILED"),
        ] {
            assert_eq!(variant.to_string(), s);
            assert_eq!(s.parse::<TransactionStatus>().unwrap(), variant);
        }
        assert!("INVALID".parse::<TransactionStatus>().is_err());
    }

    // ---- VerificationStatus ----

    #[test]
    fn verification_status_serde_round_trip() {
        for (variant, expected) in [
            (VerificationStatus::Unverified, "\"UNVERIFIED\""),
            (VerificationStatus::Verified, "\"VERIFIED\""),
            (VerificationStatus::Failed, "\"FAILED\""),
            (VerificationStatus::Partial, "\"PARTIAL\""),
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, expected);
            let parsed: VerificationStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn verification_status_display_and_from_str() {
        for (variant, s) in [
            (VerificationStatus::Unverified, "UNVERIFIED"),
            (VerificationStatus::Verified, "VERIFIED"),
            (VerificationStatus::Failed, "FAILED"),
            (VerificationStatus::Partial, "PARTIAL"),
        ] {
            assert_eq!(variant.to_string(), s);
            assert_eq!(s.parse::<VerificationStatus>().unwrap(), variant);
        }
        assert!("INVALID".parse::<VerificationStatus>().is_err());
    }

    // ---- ProposalStatus ----

    #[test]
    fn proposal_status_serde_round_trip() {
        for (variant, expected) in [
            (ProposalStatus::Pending, "\"pending\""),
            (ProposalStatus::Active, "\"active\""),
            (ProposalStatus::Succeeded, "\"succeeded\""),
            (ProposalStatus::Queued, "\"queued\""),
            (ProposalStatus::Executed, "\"executed\""),
            (ProposalStatus::Canceled, "\"canceled\""),
            (ProposalStatus::Defeated, "\"defeated\""),
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, expected);
            let parsed: ProposalStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn proposal_status_display_and_from_str() {
        for (variant, s) in [
            (ProposalStatus::Pending, "pending"),
            (ProposalStatus::Active, "active"),
            (ProposalStatus::Succeeded, "succeeded"),
            (ProposalStatus::Queued, "queued"),
            (ProposalStatus::Executed, "executed"),
            (ProposalStatus::Canceled, "canceled"),
            (ProposalStatus::Defeated, "defeated"),
        ] {
            assert_eq!(variant.to_string(), s);
            assert_eq!(s.parse::<ProposalStatus>().unwrap(), variant);
        }
        assert!("INVALID".parse::<ProposalStatus>().is_err());
    }
}
