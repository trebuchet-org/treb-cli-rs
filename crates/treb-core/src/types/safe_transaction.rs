//! Safe transaction model and nested types.
//!
//! Field names and serialization semantics match the Go implementation
//! at `treb-cli/internal/domain/models/safe_transaction.go`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::enums::TransactionStatus;

// ---------------------------------------------------------------------------
// SafeTransaction
// ---------------------------------------------------------------------------

/// A Safe multisig transaction record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeTransaction {
    pub safe_tx_hash: String,
    pub safe_address: String,
    #[serde(rename = "chainId")]
    pub chain_id: u64,
    pub status: TransactionStatus,
    pub nonce: u64,
    pub transactions: Vec<SafeTxData>,
    pub transaction_ids: Vec<String>,
    pub proposed_by: String,
    pub proposed_at: DateTime<Utc>,
    pub confirmations: Vec<Confirmation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub execution_tx_hash: String,
}

// ---------------------------------------------------------------------------
// SafeTxData
// ---------------------------------------------------------------------------

/// A single transaction in a Safe batch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SafeTxData {
    pub to: String,
    pub value: String,
    pub data: String,
    pub operation: u8,
}

// ---------------------------------------------------------------------------
// Confirmation
// ---------------------------------------------------------------------------

/// A confirmation on a Safe transaction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Confirmation {
    pub signer: String,
    pub signature: String,
    pub confirmed_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn sample_safe_transaction() -> SafeTransaction {
        SafeTransaction {
            safe_tx_hash: "0xsafetxhash123".into(),
            safe_address: "0xSafeAddress".into(),
            chain_id: 1,
            status: TransactionStatus::Queued,
            nonce: 5,
            transactions: vec![SafeTxData {
                to: "0xTarget".into(),
                value: "0".into(),
                data: "0x1234".into(),
                operation: 0,
            }],
            transaction_ids: vec!["tx-001".into()],
            proposed_by: "0xProposer".into(),
            proposed_at: Utc.with_ymd_and_hms(2025, 3, 1, 12, 0, 0).unwrap(),
            confirmations: vec![Confirmation {
                signer: "0xSigner1".into(),
                signature: "0xsig1".into(),
                confirmed_at: Utc.with_ymd_and_hms(2025, 3, 1, 12, 5, 0).unwrap(),
            }],
            executed_at: None,
            execution_tx_hash: String::new(),
        }
    }

    #[test]
    fn safe_transaction_camel_case_field_names() {
        let stx = sample_safe_transaction();
        let json = serde_json::to_value(&stx).unwrap();
        let obj = json.as_object().unwrap();

        assert!(obj.contains_key("safeTxHash"));
        assert!(obj.contains_key("safeAddress"));
        assert!(obj.contains_key("chainId"));
        assert!(obj.contains_key("status"));
        assert!(obj.contains_key("nonce"));
        assert!(obj.contains_key("transactions"));
        assert!(obj.contains_key("transactionIds"));
        assert!(obj.contains_key("proposedBy"));
        assert!(obj.contains_key("proposedAt"));
        assert!(obj.contains_key("confirmations"));

        // Verify no snake_case keys leaked
        assert!(!obj.contains_key("safe_tx_hash"));
        assert!(!obj.contains_key("safe_address"));
        assert!(!obj.contains_key("chain_id"));
        assert!(!obj.contains_key("transaction_ids"));
        assert!(!obj.contains_key("proposed_by"));
        assert!(!obj.contains_key("proposed_at"));
        assert!(!obj.contains_key("executed_at"));
        assert!(!obj.contains_key("execution_tx_hash"));
    }

    #[test]
    fn vec_fields_empty_array_not_null() {
        let mut stx = sample_safe_transaction();
        stx.transactions = vec![];
        stx.transaction_ids = vec![];
        stx.confirmations = vec![];
        let json = serde_json::to_value(&stx).unwrap();
        assert_eq!(json["transactions"], serde_json::json!([]));
        assert_eq!(json["transactionIds"], serde_json::json!([]));
        assert_eq!(json["confirmations"], serde_json::json!([]));
    }

    #[test]
    fn executed_at_omitted_when_none() {
        let stx = sample_safe_transaction();
        let json = serde_json::to_value(&stx).unwrap();
        assert!(
            !json.as_object().unwrap().contains_key("executedAt"),
            "executedAt should be omitted when None"
        );
    }

    #[test]
    fn executed_at_present_when_some() {
        let mut stx = sample_safe_transaction();
        stx.executed_at = Some(Utc.with_ymd_and_hms(2025, 3, 2, 8, 0, 0).unwrap());
        let json = serde_json::to_value(&stx).unwrap();
        assert!(json.as_object().unwrap().contains_key("executedAt"));
    }

    #[test]
    fn execution_tx_hash_omitted_when_empty() {
        let stx = sample_safe_transaction();
        let json = serde_json::to_value(&stx).unwrap();
        assert!(
            !json.as_object().unwrap().contains_key("executionTxHash"),
            "executionTxHash should be omitted when empty"
        );
    }

    #[test]
    fn execution_tx_hash_present_when_populated() {
        let mut stx = sample_safe_transaction();
        stx.execution_tx_hash = "0xexechash".into();
        let json = serde_json::to_value(&stx).unwrap();
        assert_eq!(json["executionTxHash"], "0xexechash");
    }

    #[test]
    fn safe_tx_data_field_names() {
        let txd = SafeTxData {
            to: "0xTarget".into(),
            value: "1000".into(),
            data: "0xabcd".into(),
            operation: 1,
        };
        let json = serde_json::to_value(&txd).unwrap();
        let obj = json.as_object().unwrap();
        assert!(obj.contains_key("to"));
        assert!(obj.contains_key("value"));
        assert!(obj.contains_key("data"));
        assert!(obj.contains_key("operation"));
        assert_eq!(obj.len(), 4);
    }

    #[test]
    fn confirmation_camel_case_field_names() {
        let conf = Confirmation {
            signer: "0xSigner".into(),
            signature: "0xsig".into(),
            confirmed_at: Utc.with_ymd_and_hms(2025, 3, 1, 12, 0, 0).unwrap(),
        };
        let json = serde_json::to_value(&conf).unwrap();
        let obj = json.as_object().unwrap();
        assert!(obj.contains_key("signer"));
        assert!(obj.contains_key("signature"));
        assert!(obj.contains_key("confirmedAt"));
        assert!(!obj.contains_key("confirmed_at"));
    }

    #[test]
    fn safe_transaction_serde_round_trip() {
        let stx = sample_safe_transaction();
        let json_str = serde_json::to_string_pretty(&stx).unwrap();
        let deserialized: SafeTransaction = serde_json::from_str(&json_str).unwrap();
        assert_eq!(stx, deserialized);
    }

    #[test]
    fn safe_transaction_with_execution_round_trip() {
        let mut stx = sample_safe_transaction();
        stx.executed_at = Some(Utc.with_ymd_and_hms(2025, 3, 2, 8, 0, 0).unwrap());
        stx.execution_tx_hash = "0xexechash".into();
        stx.confirmations.push(Confirmation {
            signer: "0xSigner2".into(),
            signature: "0xsig2".into(),
            confirmed_at: Utc.with_ymd_and_hms(2025, 3, 1, 13, 0, 0).unwrap(),
        });

        let json_str = serde_json::to_string_pretty(&stx).unwrap();
        let deserialized: SafeTransaction = serde_json::from_str(&json_str).unwrap();
        assert_eq!(stx, deserialized);
    }
}
