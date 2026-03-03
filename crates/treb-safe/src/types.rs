//! Safe Transaction Service request/response types and chain URL mapping.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Safe Transaction Service response types
// ---------------------------------------------------------------------------

/// Top-level paginated response from the Safe Transaction Service
/// `/multisig-transactions/` endpoint.
#[derive(Debug, Deserialize)]
pub struct SafeServiceMultisigResponse {
    pub count: u64,
    pub next: Option<String>,
    pub previous: Option<String>,
    pub results: Vec<SafeServiceTx>,
}

/// A single multisig transaction as returned by the Safe Transaction Service.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeServiceTx {
    pub safe_tx_hash: String,
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub data: Option<String>,
    #[serde(default)]
    pub operation: u8,
    #[serde(default)]
    pub nonce: u64,
    #[serde(default)]
    pub safe_tx_gas: String,
    #[serde(default)]
    pub base_gas: String,
    #[serde(default)]
    pub gas_price: String,
    #[serde(default)]
    pub gas_token: String,
    #[serde(default)]
    pub refund_receiver: String,
    #[serde(default)]
    pub is_executed: bool,
    #[serde(default)]
    pub transaction_hash: Option<String>,
    #[serde(default)]
    pub execution_date: Option<DateTime<Utc>>,
    #[serde(default)]
    pub confirmations: Vec<SafeServiceConfirmation>,
}

/// A signer confirmation from the Safe Transaction Service.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeServiceConfirmation {
    pub owner: String,
    pub signature: String,
    pub submission_date: DateTime<Utc>,
    #[serde(default)]
    pub signature_type: Option<String>,
}

// ---------------------------------------------------------------------------
// Propose request
// ---------------------------------------------------------------------------

/// Request body for proposing a new transaction to the Safe Transaction Service.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposeRequest {
    pub to: String,
    pub value: String,
    pub data: Option<String>,
    pub operation: u8,
    pub safe_tx_gas: String,
    pub base_gas: String,
    pub gas_price: String,
    pub gas_token: String,
    pub refund_receiver: String,
    pub nonce: u64,
    pub contract_transaction_hash: String,
    pub sender: String,
    pub signature: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

// ---------------------------------------------------------------------------
// Chain URL mapping
// ---------------------------------------------------------------------------

/// Map a chain ID to the Safe Transaction Service chain name segment.
///
/// The Safe Transaction Service uses chain-specific subdomains:
/// `https://safe-transaction-{chain_name}.safe.global/api/v1/...`
fn service_chain_name(chain_id: u64) -> Option<&'static str> {
    match chain_id {
        1 => Some("mainnet"),
        10 => Some("optimism"),
        56 => Some("bsc"),
        100 => Some("gnosis-chain"),
        137 => Some("polygon"),
        324 => Some("zksync"),
        8453 => Some("base"),
        42161 => Some("arbitrum"),
        42220 => Some("celo"),
        43114 => Some("avalanche"),
        59144 => Some("linea"),
        534352 => Some("scroll"),
        11155111 => Some("sepolia"),
        84532 => Some("base-sepolia"),
        _ => None,
    }
}

/// Return the base URL for the Safe Transaction Service for the given chain ID,
/// or `None` if the chain is not supported.
///
/// # Examples
///
/// ```
/// use treb_safe::service_url;
/// assert_eq!(
///     service_url(1),
///     Some("https://safe-transaction-mainnet.safe.global/api/v1".into()),
/// );
/// assert_eq!(service_url(999999), None);
/// ```
pub fn service_url(chain_id: u64) -> Option<String> {
    service_chain_name(chain_id)
        .map(|name| format!("https://safe-transaction-{name}.safe.global/api/v1"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── ProposeRequest serialization ────────────────────────────────────

    #[test]
    fn propose_request_serializes_to_expected_json() {
        let req = ProposeRequest {
            to: "0xto".into(),
            value: "0".into(),
            data: Some("0x1234".into()),
            operation: 0,
            safe_tx_gas: "0".into(),
            base_gas: "0".into(),
            gas_price: "0".into(),
            gas_token: "0x0000000000000000000000000000000000000000".into(),
            refund_receiver: "0x0000000000000000000000000000000000000000".into(),
            nonce: 42,
            contract_transaction_hash: "0xhash".into(),
            sender: "0xsender".into(),
            signature: "0xsig".into(),
            origin: Some("treb".into()),
        };

        let json = serde_json::to_value(&req).unwrap();
        let obj = json.as_object().unwrap();

        // Verify camelCase field names
        assert!(obj.contains_key("to"));
        assert!(obj.contains_key("value"));
        assert!(obj.contains_key("data"));
        assert!(obj.contains_key("operation"));
        assert!(obj.contains_key("safeTxGas"));
        assert!(obj.contains_key("baseGas"));
        assert!(obj.contains_key("gasPrice"));
        assert!(obj.contains_key("gasToken"));
        assert!(obj.contains_key("refundReceiver"));
        assert!(obj.contains_key("nonce"));
        assert!(obj.contains_key("contractTransactionHash"));
        assert!(obj.contains_key("sender"));
        assert!(obj.contains_key("signature"));
        assert!(obj.contains_key("origin"));

        // Verify no snake_case keys
        assert!(!obj.contains_key("safe_tx_gas"));
        assert!(!obj.contains_key("base_gas"));
        assert!(!obj.contains_key("gas_price"));
        assert!(!obj.contains_key("gas_token"));
        assert!(!obj.contains_key("refund_receiver"));
        assert!(!obj.contains_key("contract_transaction_hash"));

        // Verify values
        assert_eq!(json["nonce"], 42);
        assert_eq!(json["to"], "0xto");
        assert_eq!(json["operation"], 0);
    }

    #[test]
    fn propose_request_origin_omitted_when_none() {
        let req = ProposeRequest {
            to: "0xto".into(),
            value: "0".into(),
            data: None,
            operation: 0,
            safe_tx_gas: "0".into(),
            base_gas: "0".into(),
            gas_price: "0".into(),
            gas_token: "0x0000000000000000000000000000000000000000".into(),
            refund_receiver: "0x0000000000000000000000000000000000000000".into(),
            nonce: 0,
            contract_transaction_hash: "0xhash".into(),
            sender: "0xsender".into(),
            signature: "0xsig".into(),
            origin: None,
        };

        let json = serde_json::to_value(&req).unwrap();
        assert!(!json.as_object().unwrap().contains_key("origin"));
    }

    // ── SafeServiceMultisigResponse deserialization ─────────────────────

    #[test]
    fn deserialize_multisig_response() {
        let json = r#"{
            "count": 2,
            "next": "https://example.com/next",
            "previous": null,
            "results": [
                {
                    "safeTxHash": "0xabc123",
                    "to": "0xTarget",
                    "value": "1000000000000000000",
                    "data": "0xabcdef",
                    "operation": 0,
                    "nonce": 42,
                    "safeTxGas": "0",
                    "baseGas": "0",
                    "gasPrice": "0",
                    "gasToken": "0x0000000000000000000000000000000000000000",
                    "refundReceiver": "0x0000000000000000000000000000000000000000",
                    "isExecuted": true,
                    "transactionHash": "0xdef456",
                    "executionDate": "2025-01-15T10:30:00Z",
                    "confirmations": [
                        {
                            "owner": "0x1111111111111111111111111111111111111111",
                            "signature": "0xsig1",
                            "submissionDate": "2025-01-14T08:00:00Z",
                            "signatureType": "EOA"
                        }
                    ]
                },
                {
                    "safeTxHash": "0xpending999",
                    "nonce": 43,
                    "isExecuted": false,
                    "confirmations": []
                }
            ]
        }"#;

        let resp: SafeServiceMultisigResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.count, 2);
        assert_eq!(resp.next, Some("https://example.com/next".into()));
        assert!(resp.previous.is_none());
        assert_eq!(resp.results.len(), 2);

        let tx0 = &resp.results[0];
        assert_eq!(tx0.safe_tx_hash, "0xabc123");
        assert_eq!(tx0.to, "0xTarget");
        assert_eq!(tx0.nonce, 42);
        assert!(tx0.is_executed);
        assert_eq!(tx0.transaction_hash.as_deref(), Some("0xdef456"));
        assert!(tx0.execution_date.is_some());
        assert_eq!(tx0.confirmations.len(), 1);
        assert_eq!(
            tx0.confirmations[0].owner,
            "0x1111111111111111111111111111111111111111"
        );

        let tx1 = &resp.results[1];
        assert_eq!(tx1.safe_tx_hash, "0xpending999");
        assert!(!tx1.is_executed);
        assert!(tx1.transaction_hash.is_none());
        assert!(tx1.confirmations.is_empty());
    }

    #[test]
    fn deserialize_multisig_response_empty() {
        let json = r#"{ "count": 0, "next": null, "previous": null, "results": [] }"#;
        let resp: SafeServiceMultisigResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.count, 0);
        assert!(resp.results.is_empty());
    }

    // ── SafeServiceConfirmation camelCase deserialization ───────────────

    #[test]
    fn deserialize_confirmation_camel_case() {
        let json = r#"{
            "owner": "0xOwnerAddr",
            "signature": "0xdeadbeef",
            "submissionDate": "2025-06-15T14:30:00Z",
            "signatureType": "APPROVED_HASH"
        }"#;

        let conf: SafeServiceConfirmation = serde_json::from_str(json).unwrap();
        assert_eq!(conf.owner, "0xOwnerAddr");
        assert_eq!(conf.signature, "0xdeadbeef");
        assert_eq!(conf.signature_type.as_deref(), Some("APPROVED_HASH"));
        assert!(conf.submission_date.timestamp() > 0);
    }

    #[test]
    fn deserialize_confirmation_without_signature_type() {
        let json = r#"{
            "owner": "0xOwner",
            "signature": "0xsig",
            "submissionDate": "2025-01-01T00:00:00Z"
        }"#;

        let conf: SafeServiceConfirmation = serde_json::from_str(json).unwrap();
        assert!(conf.signature_type.is_none());
    }

    // ── service_url mapping ────────────────────────────────────────────

    #[test]
    fn service_url_mainnet() {
        assert_eq!(
            service_url(1).unwrap(),
            "https://safe-transaction-mainnet.safe.global/api/v1"
        );
    }

    #[test]
    fn service_url_polygon() {
        assert_eq!(
            service_url(137).unwrap(),
            "https://safe-transaction-polygon.safe.global/api/v1"
        );
    }

    #[test]
    fn service_url_base() {
        assert_eq!(
            service_url(8453).unwrap(),
            "https://safe-transaction-base.safe.global/api/v1"
        );
    }

    #[test]
    fn service_url_sepolia() {
        assert_eq!(
            service_url(11155111).unwrap(),
            "https://safe-transaction-sepolia.safe.global/api/v1"
        );
    }

    #[test]
    fn service_url_unknown_chain() {
        assert!(service_url(999999).is_none());
    }

    #[test]
    fn service_url_all_supported_chains() {
        let supported = [
            (1, "mainnet"),
            (10, "optimism"),
            (56, "bsc"),
            (100, "gnosis-chain"),
            (137, "polygon"),
            (324, "zksync"),
            (8453, "base"),
            (42161, "arbitrum"),
            (42220, "celo"),
            (43114, "avalanche"),
            (59144, "linea"),
            (534352, "scroll"),
            (11155111, "sepolia"),
            (84532, "base-sepolia"),
        ];

        for (chain_id, expected_name) in supported {
            let url = service_url(chain_id)
                .unwrap_or_else(|| panic!("service_url({chain_id}) should return Some"));
            let expected =
                format!("https://safe-transaction-{expected_name}.safe.global/api/v1");
            assert_eq!(url, expected, "chain_id={chain_id}");
        }
    }
}
