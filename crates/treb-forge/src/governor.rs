//! On-chain Governor contract state polling client.
//!
//! Queries a Governor contract's `state(uint256 proposalId)` view function
//! via alloy provider `call()` and maps the uint8 return value to [`ProposalStatus`].

use alloy_primitives::{Address, Bytes, U256};
use alloy_provider::Provider;
use alloy_rpc_types::{TransactionInput, TransactionRequest};
use treb_core::{TrebError, types::ProposalStatus};

/// Function selector for `state(uint256)`: first 4 bytes of keccak256("state(uint256)").
const STATE_SELECTOR: [u8; 4] = [0x3e, 0x4f, 0x49, 0xe6];

/// Build the ABI-encoded calldata for `state(uint256 proposalId)`.
///
/// Layout: 4-byte selector + 32-byte ABI-encoded uint256.
fn build_state_calldata(proposal_id: &U256) -> Bytes {
    let mut calldata = Vec::with_capacity(36);
    calldata.extend_from_slice(&STATE_SELECTOR);
    calldata.extend_from_slice(&proposal_id.to_be_bytes::<32>());
    calldata.into()
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
    rpc_url: &str,
    governor_address: &str,
    proposal_id: &str,
) -> Result<ProposalStatus, TrebError> {
    // Parse proposal_id as a decimal U256.
    let id = U256::from_str_radix(proposal_id, 10)
        .map_err(|e| TrebError::Governor(format!("invalid proposal ID '{proposal_id}': {e}")))?;

    let gov_addr: Address = governor_address
        .parse()
        .map_err(|e| TrebError::Governor(format!("invalid governor address '{governor_address}': {e}")))?;

    let provider = crate::provider::build_http_provider(rpc_url)
        .map_err(|e| TrebError::Governor(format!("failed to build RPC provider: {e}")))?;

    let calldata = build_state_calldata(&id);
    let tx = TransactionRequest::default()
        .to(gov_addr)
        .input(TransactionInput::new(calldata));

    let result = provider.call(tx).await.map_err(|e| {
        let err_str = e.to_string();
        let prefix = if err_str.to_ascii_lowercase().contains("revert") {
            "Governor state() call reverted"
        } else {
            "RPC error calling Governor state()"
        };
        TrebError::Governor(format!("{prefix}: {err_str}"))
    })?;

    if result.len() != 32 {
        return Err(TrebError::Governor(format!(
            "unexpected result length: expected 32 bytes, got {}",
            result.len()
        )));
    }

    let state_value = result[31];
    map_onchain_state(state_value)
}

/// Check if a [`ProposalStatus`] is terminal (no further state transitions expected).
pub fn is_terminal(status: &ProposalStatus) -> bool {
    matches!(status, ProposalStatus::Executed | ProposalStatus::Canceled | ProposalStatus::Defeated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    // Use the hex crate from alloy for encoding/decoding.
    use alloy_primitives::hex;

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

    fn spawn_http_server(status_line: &str, body: &str) -> Option<String> {
        let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return None,
            Err(err) => panic!("bind test HTTP server: {err}"),
        };
        let port = listener.local_addr().unwrap().port();
        let status_line = status_line.to_string();
        let body = body.to_string();

        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0_u8; 4096];
            let _ = stream.read(&mut buf);

            let response = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
        });

        Some(format!("http://127.0.0.1:{port}"))
    }

    fn encoded_state_result(state: u8) -> String {
        let mut encoded = [0_u8; 32];
        encoded[31] = state;
        format!("0x{}", hex::encode(encoded))
    }

    #[tokio::test]
    async fn query_proposal_state_returns_result_payload() {
        let Some(rpc_url) = spawn_http_server(
            "200 OK",
            &format!(r#"{{"jsonrpc":"2.0","id":1,"result":"{}"}}"#, encoded_state_result(5)),
        ) else {
            return;
        };

        let status = query_proposal_state(
            &rpc_url,
            "0x0000000000000000000000000000000000000001",
            "42",
        )
        .await
        .unwrap();

        assert_eq!(status, ProposalStatus::Queued);
    }

    #[tokio::test]
    async fn query_proposal_state_preserves_non_revert_rpc_errors() {
        let Some(rpc_url) = spawn_http_server(
            "200 OK",
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32005,"message":"rate limit exceeded"}}"#,
        ) else {
            return;
        };

        let err = query_proposal_state(
            &rpc_url,
            "0x0000000000000000000000000000000000000001",
            "42",
        )
        .await
        .unwrap_err()
        .to_string();

        assert!(err.contains("RPC error calling Governor state()"), "got: {err}");
        assert!(err.contains("rate limit exceeded"), "got: {err}");
        assert!(!err.contains("call reverted"), "got: {err}");
    }

    #[tokio::test]
    async fn query_proposal_state_marks_revert_payloads_as_reverts() {
        let Some(rpc_url) = spawn_http_server(
            "200 OK",
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":3,"message":"execution reverted: unknown proposal id"}}"#,
        ) else {
            return;
        };

        let err = query_proposal_state(
            &rpc_url,
            "0x0000000000000000000000000000000000000001",
            "42",
        )
        .await
        .unwrap_err()
        .to_string();

        assert!(err.contains("Governor state() call reverted"), "got: {err}");
        assert!(err.contains("execution reverted"), "got: {err}");
    }

    #[tokio::test]
    async fn query_proposal_state_rpc_error_for_non_success_responses() {
        let Some(rpc_url) = spawn_http_server(
            "429 Too Many Requests",
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32005,"message":"rate limit exceeded"}}"#,
        ) else {
            return;
        };

        let err = query_proposal_state(
            &rpc_url,
            "0x0000000000000000000000000000000000000001",
            "42",
        )
        .await
        .unwrap_err()
        .to_string();

        // Alloy surfaces the JSON-RPC error from the response body;
        // the HTTP status code is not included in the error.
        assert!(err.contains("RPC error calling Governor state()"), "got: {err}");
        assert!(err.contains("rate limit exceeded"), "got: {err}");
    }
}
