//! Transaction model and nested types.
//!
//! Field names and serialization semantics match the Go implementation
//! at `treb-cli/internal/domain/models/transaction.go`.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::enums::TransactionStatus;

// ---------------------------------------------------------------------------
// Transaction
// ---------------------------------------------------------------------------

/// A blockchain transaction record.
///
/// Go registry fixtures can encode `createdAt` with a non-UTC RFC3339 offset.
/// Rust normalizes that timestamp into `DateTime<Utc>` and writes it back as
/// the same instant in UTC, so compatibility checks should compare instants
/// rather than raw timestamp strings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transaction {
    pub id: String,
    #[serde(rename = "chainId")]
    pub chain_id: u64,
    pub hash: String,
    pub status: TransactionStatus,
    #[serde(default, skip_serializing_if = "is_zero_u64", rename = "blockNumber")]
    pub block_number: u64,
    pub sender: String,
    pub nonce: u64,
    pub deployments: Vec<String>,
    pub operations: Vec<Operation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safe_context: Option<SafeContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broadcast_file: Option<String>,
    pub environment: String,
    pub created_at: DateTime<Utc>,
}

fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}

// ---------------------------------------------------------------------------
// Operation
// ---------------------------------------------------------------------------

/// An operation within a transaction (DEPLOY, CALL, etc.).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Operation {
    #[serde(rename = "type")]
    pub operation_type: String,
    pub target: String,
    pub method: String,
    pub result: HashMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// SafeContext
// ---------------------------------------------------------------------------

/// Safe-specific transaction information.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeContext {
    pub safe_address: String,
    pub safe_tx_hash: String,
    pub batch_index: i64,
    pub proposer_address: String,
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn sample_transaction() -> Transaction {
        Transaction {
            id: "tx-0x1234abcd".into(),
            chain_id: 1,
            hash: "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef".into(),
            status: TransactionStatus::Executed,
            block_number: 0,
            sender: "0xSenderAddress".into(),
            nonce: 42,
            deployments: vec!["production/1/Counter:v1".into()],
            operations: vec![Operation {
                operation_type: "DEPLOY".into(),
                target: "0xTargetAddress".into(),
                method: "CREATE".into(),
                result: {
                    let mut m = HashMap::new();
                    m.insert(
                        "address".into(),
                        serde_json::Value::String("0xDeployedAddress".into()),
                    );
                    m
                },
            }],
            safe_context: None,
            broadcast_file: None,
            environment: "production".into(),
            created_at: Utc.with_ymd_and_hms(2025, 1, 15, 10, 30, 0).unwrap(),
        }
    }

    #[test]
    fn transaction_camel_case_field_names() {
        let tx = sample_transaction();
        let json = serde_json::to_value(&tx).unwrap();
        let obj = json.as_object().unwrap();

        assert!(obj.contains_key("id"));
        assert!(obj.contains_key("chainId"));
        assert!(obj.contains_key("hash"));
        assert!(obj.contains_key("status"));
        assert!(obj.contains_key("sender"));
        assert!(obj.contains_key("nonce"));
        assert!(obj.contains_key("deployments"));
        assert!(obj.contains_key("operations"));
        assert!(obj.contains_key("environment"));
        assert!(obj.contains_key("createdAt"));

        // Verify no snake_case keys leaked
        assert!(!obj.contains_key("chain_id"));
        assert!(!obj.contains_key("block_number"));
        assert!(!obj.contains_key("safe_context"));
        assert!(!obj.contains_key("created_at"));
    }

    #[test]
    fn block_number_omitted_when_zero() {
        let tx = sample_transaction();
        let json = serde_json::to_value(&tx).unwrap();
        assert!(
            !json.as_object().unwrap().contains_key("blockNumber"),
            "blockNumber should be omitted when 0"
        );
    }

    #[test]
    fn block_number_present_when_nonzero() {
        let mut tx = sample_transaction();
        tx.block_number = 12345678;
        let json = serde_json::to_value(&tx).unwrap();
        assert_eq!(json["blockNumber"], serde_json::json!(12345678));
    }

    #[test]
    fn deployments_empty_array_not_null() {
        let mut tx = sample_transaction();
        tx.deployments = vec![];
        let json = serde_json::to_value(&tx).unwrap();
        assert_eq!(json["deployments"], serde_json::json!([]));
    }

    #[test]
    fn operations_empty_array_not_null() {
        let mut tx = sample_transaction();
        tx.operations = vec![];
        let json = serde_json::to_value(&tx).unwrap();
        assert_eq!(json["operations"], serde_json::json!([]));
    }

    #[test]
    fn safe_context_omitted_when_none() {
        let tx = sample_transaction();
        let json = serde_json::to_value(&tx).unwrap();
        assert!(
            !json.as_object().unwrap().contains_key("safeContext"),
            "safeContext should be omitted when None"
        );
    }

    #[test]
    fn safe_context_present_when_some() {
        let mut tx = sample_transaction();
        tx.safe_context = Some(SafeContext {
            safe_address: "0xSafeAddress".into(),
            safe_tx_hash: "0xSafeTxHash".into(),
            batch_index: 0,
            proposer_address: "0xProposerAddress".into(),
        });
        let json = serde_json::to_value(&tx).unwrap();
        let ctx = &json["safeContext"];
        assert_eq!(ctx["safeAddress"], "0xSafeAddress");
        assert_eq!(ctx["safeTxHash"], "0xSafeTxHash");
        assert_eq!(ctx["batchIndex"], 0);
        assert_eq!(ctx["proposerAddress"], "0xProposerAddress");
    }

    #[test]
    fn operation_type_field_renamed() {
        let op = Operation {
            operation_type: "DEPLOY".into(),
            target: "0xTarget".into(),
            method: "CREATE".into(),
            result: HashMap::new(),
        };
        let json = serde_json::to_value(&op).unwrap();
        let obj = json.as_object().unwrap();
        assert!(obj.contains_key("type"));
        assert!(!obj.contains_key("operation_type"));
    }

    #[test]
    fn transaction_serde_round_trip() {
        let tx = sample_transaction();
        let json_str = serde_json::to_string_pretty(&tx).unwrap();
        let deserialized: Transaction = serde_json::from_str(&json_str).unwrap();
        assert_eq!(tx, deserialized);
    }

    #[test]
    fn transaction_with_safe_context_round_trip() {
        let mut tx = sample_transaction();
        tx.block_number = 99999;
        tx.safe_context = Some(SafeContext {
            safe_address: "0xSafeAddress".into(),
            safe_tx_hash: "0xSafeTxHash".into(),
            batch_index: 3,
            proposer_address: "0xProposerAddress".into(),
        });

        let json_str = serde_json::to_string_pretty(&tx).unwrap();
        let deserialized: Transaction = serde_json::from_str(&json_str).unwrap();
        assert_eq!(tx, deserialized);
    }
}
