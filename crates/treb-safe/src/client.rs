//! Safe Transaction Service HTTP client.

use std::time::Duration;

use reqwest::Client;
use treb_core::error::TrebError;

use crate::types::{
    ProposeRequest, SafeInfoResponse, SafeServiceMultisigResponse, SafeServiceTx,
};

/// HTTP client for the Safe Transaction Service API.
pub struct SafeServiceClient {
    http: Client,
    base_url: String,
}

impl SafeServiceClient {
    /// Create a new client for the given chain ID.
    ///
    /// Returns `None` if the chain ID is not supported by the Safe Transaction
    /// Service.
    pub fn new(chain_id: u64) -> Option<Self> {
        let base_url = crate::types::service_url(chain_id)?;
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build reqwest client");
        Some(Self { http, base_url })
    }

    /// Return the resolved base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    // ── Propose ──────────────────────────────────────────────────────────

    /// Propose a new transaction to the Safe Transaction Service.
    ///
    /// Endpoint: `POST /safes/{safe_address}/multisig-transactions/`
    pub async fn propose_transaction(
        &self,
        safe_address: &str,
        request: &ProposeRequest,
    ) -> Result<(), TrebError> {
        let url = format!(
            "{}/safes/{}/multisig-transactions/",
            self.base_url, safe_address
        );
        let resp = self
            .http
            .post(&url)
            .json(request)
            .send()
            .await
            .map_err(|e| TrebError::Safe(format!("propose request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(TrebError::Safe(format!(
                "propose returned {status}: {body}"
            )));
        }
        Ok(())
    }

    // ── Pending transactions ─────────────────────────────────────────────

    /// Fetch pending (non-executed) multisig transactions for a Safe.
    ///
    /// Endpoint: `GET /safes/{safe_address}/multisig-transactions/?executed=false`
    pub async fn get_pending_transactions(
        &self,
        safe_address: &str,
    ) -> Result<SafeServiceMultisigResponse, TrebError> {
        let url = format!(
            "{}/safes/{}/multisig-transactions/?executed=false",
            self.base_url, safe_address
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| TrebError::Safe(format!("pending transactions request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(TrebError::Safe(format!(
                "pending transactions returned {status}: {body}"
            )));
        }

        let body = resp.text().await.unwrap_or_default();
        serde_json::from_str(&body).map_err(|e| {
            TrebError::Safe(format!("failed to parse pending transactions response: {e}"))
        })
    }

    // ── Individual transaction ───────────────────────────────────────────

    /// Fetch a single multisig transaction by its Safe transaction hash.
    ///
    /// Endpoint: `GET /multisig-transactions/{safe_tx_hash}/`
    pub async fn get_transaction(
        &self,
        safe_tx_hash: &str,
    ) -> Result<SafeServiceTx, TrebError> {
        let url = format!(
            "{}/multisig-transactions/{}/",
            self.base_url, safe_tx_hash
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| TrebError::Safe(format!("get transaction request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(TrebError::Safe(format!(
                "get transaction returned {status}: {body}"
            )));
        }

        let body = resp.text().await.unwrap_or_default();
        serde_json::from_str(&body).map_err(|e| {
            TrebError::Safe(format!("failed to parse transaction response: {e}"))
        })
    }

    // ── Nonce ────────────────────────────────────────────────────────────

    /// Retrieve the current nonce for a Safe.
    ///
    /// Endpoint: `GET /safes/{safe_address}/`
    pub async fn get_nonce(
        &self,
        safe_address: &str,
    ) -> Result<u64, TrebError> {
        let url = format!("{}/safes/{}/", self.base_url, safe_address);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| TrebError::Safe(format!("safe info request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(TrebError::Safe(format!(
                "safe info returned {status}: {body}"
            )));
        }

        let body = resp.text().await.unwrap_or_default();
        let info: SafeInfoResponse = serde_json::from_str(&body).map_err(|e| {
            TrebError::Safe(format!("failed to parse safe info response: {e}"))
        })?;
        Ok(info.nonce)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── Client construction ──────────────────────────────────────────────

    #[test]
    fn new_supported_chain() {
        let client = SafeServiceClient::new(1).expect("mainnet should be supported");
        assert_eq!(
            client.base_url(),
            "https://safe-transaction-mainnet.safe.global/api/v1"
        );
    }

    #[test]
    fn new_unsupported_chain() {
        assert!(SafeServiceClient::new(999999).is_none());
    }

    #[test]
    fn new_all_supported_chains() {
        let chains = [1, 10, 56, 100, 137, 324, 8453, 42161, 42220, 43114, 59144, 534352, 11155111, 84532];
        for chain_id in chains {
            assert!(
                SafeServiceClient::new(chain_id).is_some(),
                "chain {chain_id} should be supported"
            );
        }
    }

    // ── Fixture: 200 propose response ────────────────────────────────────

    #[test]
    fn propose_request_fixture_serialization() {
        // Verify that a ProposeRequest serializes correctly for the API
        let req = ProposeRequest {
            to: "0xTargetContract".into(),
            value: "0".into(),
            data: Some("0xdeploydata".into()),
            operation: 0,
            safe_tx_gas: "0".into(),
            base_gas: "0".into(),
            gas_price: "0".into(),
            gas_token: "0x0000000000000000000000000000000000000000".into(),
            refund_receiver: "0x0000000000000000000000000000000000000000".into(),
            nonce: 7,
            contract_transaction_hash: "0xsafetxhash".into(),
            sender: "0xSignerAddr".into(),
            signature: "0xSignatureBytes".into(),
            origin: Some("treb".into()),
        };

        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["nonce"], 7);
        assert_eq!(json["sender"], "0xSignerAddr");
        assert_eq!(json["contractTransactionHash"], "0xsafetxhash");
        assert!(json.as_object().unwrap().contains_key("safeTxGas"));
    }

    // ── Fixture: 422 error response ──────────────────────────────────────

    #[test]
    fn error_422_response_body_parsed() {
        // Simulate a 422 error response body from the Safe Transaction Service
        let body = r#"{"code":422,"message":"Safe transaction with nonce=7 already exists","arguments":["nonce"]}"#;

        // We just need to verify the body is valid JSON (the client includes it
        // in the error message)
        let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(parsed["code"], 422);
        assert!(parsed["message"].as_str().unwrap().contains("already exists"));
    }

    // ── Fixture: pending transactions ────────────────────────────────────

    #[test]
    fn pending_transactions_fixture_deserialization() {
        let json = r#"{
            "count": 1,
            "next": null,
            "previous": null,
            "results": [
                {
                    "safeTxHash": "0xpendingabc",
                    "to": "0xTarget",
                    "value": "0",
                    "data": "0x1234",
                    "operation": 0,
                    "nonce": 10,
                    "safeTxGas": "0",
                    "baseGas": "0",
                    "gasPrice": "0",
                    "gasToken": "0x0000000000000000000000000000000000000000",
                    "refundReceiver": "0x0000000000000000000000000000000000000000",
                    "isExecuted": false,
                    "transactionHash": null,
                    "confirmations": [
                        {
                            "owner": "0xOwner1",
                            "signature": "0xsig1",
                            "submissionDate": "2025-02-01T12:00:00Z"
                        }
                    ]
                }
            ]
        }"#;

        let resp: SafeServiceMultisigResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.count, 1);
        assert_eq!(resp.results.len(), 1);
        let tx = &resp.results[0];
        assert_eq!(tx.safe_tx_hash, "0xpendingabc");
        assert!(!tx.is_executed);
        assert!(tx.transaction_hash.is_none());
        assert_eq!(tx.nonce, 10);
        assert_eq!(tx.confirmations.len(), 1);
    }

    // ── Fixture: single transaction ──────────────────────────────────────

    #[test]
    fn single_transaction_fixture_deserialization() {
        let json = r#"{
            "safeTxHash": "0xexecuteddef",
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
            "transactionHash": "0xonchaintx",
            "executionDate": "2025-01-15T10:30:00Z",
            "confirmations": [
                {
                    "owner": "0xOwner1",
                    "signature": "0xsig1",
                    "submissionDate": "2025-01-14T08:00:00Z",
                    "signatureType": "EOA"
                },
                {
                    "owner": "0xOwner2",
                    "signature": "0xsig2",
                    "submissionDate": "2025-01-14T09:00:00Z",
                    "signatureType": "EOA"
                }
            ]
        }"#;

        let tx: SafeServiceTx = serde_json::from_str(json).unwrap();
        assert_eq!(tx.safe_tx_hash, "0xexecuteddef");
        assert!(tx.is_executed);
        assert_eq!(tx.transaction_hash.as_deref(), Some("0xonchaintx"));
        assert_eq!(tx.nonce, 42);
        assert_eq!(tx.confirmations.len(), 2);
        assert!(tx.execution_date.is_some());
    }

    // ── Fixture: safe info nonce ─────────────────────────────────────────

    #[test]
    fn safe_info_nonce_fixture_deserialization() {
        let json = r#"{
            "address": "0x1234567890abcdef1234567890abcdef12345678",
            "nonce": 15,
            "threshold": 2,
            "owners": [
                "0xOwner1",
                "0xOwner2",
                "0xOwner3"
            ]
        }"#;

        let info: SafeInfoResponse = serde_json::from_str(json).unwrap();
        assert_eq!(info.nonce, 15);
        assert_eq!(info.threshold, 2);
        assert_eq!(info.owners.len(), 3);
        assert_eq!(
            info.address,
            "0x1234567890abcdef1234567890abcdef12345678"
        );
    }

    #[test]
    fn safe_info_minimal_fixture() {
        // Safe info with only required fields
        let json = r#"{
            "address": "0xSafe",
            "nonce": 0
        }"#;

        let info: SafeInfoResponse = serde_json::from_str(json).unwrap();
        assert_eq!(info.nonce, 0);
        assert!(info.owners.is_empty());
    }
}
