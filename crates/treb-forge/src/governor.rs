//! On-chain Governor contract state polling client.
//!
//! Queries a Governor contract's `state(uint256 proposalId)` view function
//! via `eth_call` JSON-RPC and maps the uint8 return value to [`ProposalStatus`].

use alloy_primitives::U256;
use treb_core::TrebError;
use treb_core::types::ProposalStatus;

/// Function selector for `state(uint256)`: first 4 bytes of keccak256("state(uint256)").
const STATE_SELECTOR: [u8; 4] = [0x3e, 0x4f, 0x49, 0xe6];

/// Build the ABI-encoded calldata for `state(uint256 proposalId)`.
///
/// Layout: 4-byte selector + 32-byte ABI-encoded uint256.
fn build_state_calldata(proposal_id: &U256) -> Vec<u8> {
    let mut calldata = Vec::with_capacity(36);
    calldata.extend_from_slice(&STATE_SELECTOR);
    calldata.extend_from_slice(&proposal_id.to_be_bytes::<32>());
    calldata
}

/// Map an on-chain OZ Governor `ProposalState` uint8 value (0–7) to [`ProposalStatus`].
///
/// OZ mapping:
/// - 0 = Pending
/// - 1 = Active
/// - 2 = Canceled
/// - 3 = Defeated
/// - 4 = Succeeded
/// - 5 = Queued
/// - 6 = Expired  → mapped to Defeated (terminal non-executed)
/// - 7 = Executed
pub fn map_onchain_state(state: u8) -> Result<ProposalStatus, TrebError> {
    match state {
        0 => Ok(ProposalStatus::Pending),
        1 => Ok(ProposalStatus::Active),
        2 => Ok(ProposalStatus::Canceled),
        3 => Ok(ProposalStatus::Defeated),
        4 => Ok(ProposalStatus::Succeeded),
        5 => Ok(ProposalStatus::Queued),
        6 => Ok(ProposalStatus::Defeated), // Expired → Defeated
        7 => Ok(ProposalStatus::Executed),
        n => Err(TrebError::Governor(format!("unknown proposal state: {n}"))),
    }
}

/// Query the on-chain Governor contract for a proposal's current state.
///
/// Sends an `eth_call` to `governor_address` with `state(uint256 proposalId)` calldata
/// and maps the uint8 return value to [`ProposalStatus`].
pub async fn query_proposal_state(
    client: &reqwest::Client,
    rpc_url: &str,
    governor_address: &str,
    proposal_id: &str,
) -> Result<ProposalStatus, TrebError> {
    // Parse proposal_id as a decimal U256.
    let id = U256::from_str_radix(proposal_id, 10).map_err(|e| {
        TrebError::Governor(format!("invalid proposal ID '{proposal_id}': {e}"))
    })?;

    let calldata = build_state_calldata(&id);
    let data_hex = format!("0x{}", hex::encode(&calldata));

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "eth_call",
        "params": [
            {
                "to": governor_address,
                "data": data_hex,
            },
            "latest"
        ],
        "id": 1,
    });

    let resp = client
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| TrebError::Governor(format!("RPC request to {rpc_url} failed: {e}")))?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| TrebError::Governor(format!("failed to parse RPC response: {e}")))?;

    if let Some(err) = json.get("error") {
        return Err(TrebError::Governor(format!(
            "Governor state() call reverted: {err}"
        )));
    }

    let result_hex = json
        .get("result")
        .and_then(|v| v.as_str())
        .ok_or_else(|| TrebError::Governor("missing 'result' in RPC response".into()))?;

    // Result is a 32-byte ABI-encoded uint8. Parse the last byte.
    let result_bytes = hex::decode(result_hex.strip_prefix("0x").unwrap_or(result_hex))
        .map_err(|e| TrebError::Governor(format!("invalid hex in RPC result: {e}")))?;

    if result_bytes.len() != 32 {
        return Err(TrebError::Governor(format!(
            "unexpected result length: expected 32 bytes, got {}",
            result_bytes.len()
        )));
    }

    let state_value = result_bytes[31];
    map_onchain_state(state_value)
}

/// Check if a [`ProposalStatus`] is terminal (no further state transitions expected).
pub fn is_terminal(status: &ProposalStatus) -> bool {
    matches!(
        status,
        ProposalStatus::Executed | ProposalStatus::Canceled | ProposalStatus::Defeated
    )
}

// Use the hex crate from alloy for encoding/decoding.
use alloy_primitives::hex;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_state_calldata_encodes_correctly() {
        // Example: proposal_id = 42
        let id = U256::from(42u64);
        let calldata = build_state_calldata(&id);

        assert_eq!(calldata.len(), 36);
        // First 4 bytes: function selector
        assert_eq!(&calldata[0..4], &STATE_SELECTOR);
        // Last 32 bytes: ABI-encoded uint256(42)
        let mut expected_param = [0u8; 32];
        expected_param[31] = 42;
        assert_eq!(&calldata[4..36], &expected_param);
    }

    #[test]
    fn build_state_calldata_large_proposal_id() {
        // A large proposal ID typical of OZ Governor (hash-derived)
        let id = U256::from_str_radix(
            "48798382349827398472398472398472389472398472398472398472398472398",
            10,
        )
        .unwrap();
        let calldata = build_state_calldata(&id);

        assert_eq!(calldata.len(), 36);
        assert_eq!(&calldata[0..4], &STATE_SELECTOR);
        // Verify the uint256 encoding matches
        assert_eq!(&calldata[4..36], &id.to_be_bytes::<32>());
    }

    #[test]
    fn map_onchain_state_all_values() {
        let expected = [
            (0, ProposalStatus::Pending),
            (1, ProposalStatus::Active),
            (2, ProposalStatus::Canceled),
            (3, ProposalStatus::Defeated),
            (4, ProposalStatus::Succeeded),
            (5, ProposalStatus::Queued),
            (6, ProposalStatus::Defeated), // Expired → Defeated
            (7, ProposalStatus::Executed),
        ];

        for (state_val, expected_status) in &expected {
            let result = map_onchain_state(*state_val).unwrap();
            assert_eq!(
                result, *expected_status,
                "state {state_val} should map to {expected_status}"
            );
        }
    }

    #[test]
    fn map_onchain_state_invalid_value() {
        let result = map_onchain_state(8);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unknown proposal state: 8"), "got: {err}");
    }

    #[test]
    fn is_terminal_correct() {
        assert!(is_terminal(&ProposalStatus::Executed));
        assert!(is_terminal(&ProposalStatus::Canceled));
        assert!(is_terminal(&ProposalStatus::Defeated));

        assert!(!is_terminal(&ProposalStatus::Pending));
        assert!(!is_terminal(&ProposalStatus::Active));
        assert!(!is_terminal(&ProposalStatus::Succeeded));
        assert!(!is_terminal(&ProposalStatus::Queued));
    }
}
