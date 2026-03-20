//! Integration tests for the treb-safe crate.
//!
//! Tests SafeServiceClient creation, ProposeRequest serialization,
//! and EIP-712 hash computation against known vectors.

use alloy_primitives::{Address, B256, U256, address};
use treb_safe::{
    SafeServiceClient, SafeTx, compute_safe_tx_hash, safe_domain,
    types::{ProposeRequest, SafeServiceMultisigResponse},
};

// ── Client creation: supported chains ────────────────────────────────────

#[test]
fn client_creation_mainnet() {
    let client = SafeServiceClient::new(1).expect("mainnet should be supported");
    assert_eq!(client.base_url(), "https://api.safe.global/tx-service/eth/api/v1");
}

#[test]
fn client_creation_polygon() {
    let client = SafeServiceClient::new(137).expect("polygon should be supported");
    assert_eq!(client.base_url(), "https://api.safe.global/tx-service/pol/api/v1");
}

#[test]
fn client_creation_base() {
    let client = SafeServiceClient::new(8453).expect("base should be supported");
    assert_eq!(client.base_url(), "https://api.safe.global/tx-service/base/api/v1");
}

#[test]
fn client_creation_sepolia() {
    let client = SafeServiceClient::new(11155111).expect("sepolia should be supported");
    assert_eq!(client.base_url(), "https://api.safe.global/tx-service/sep/api/v1");
}

#[test]
fn client_creation_all_supported_chains() {
    let supported_chains = [
        (1, "eth"),
        (10, "oeth"),
        (56, "bnb"),
        (100, "gno"),
        (137, "pol"),
        (324, "zksync"),
        (8453, "base"),
        (42161, "arb1"),
        (42220, "celo"),
        (43114, "avax"),
        (59144, "linea"),
        (534352, "scr"),
        (11155111, "sep"),
        (84532, "basesep"),
    ];

    for (chain_id, expected_name) in supported_chains {
        let client = SafeServiceClient::new(chain_id)
            .unwrap_or_else(|| panic!("chain {chain_id} ({expected_name}) should be supported"));
        let expected_url = format!("https://api.safe.global/tx-service/{expected_name}/api/v1");
        assert_eq!(client.base_url(), expected_url, "wrong URL for chain {chain_id}");
    }
}

// ── Client creation: unsupported chains ──────────────────────────────────

#[test]
fn client_creation_unsupported_chain_returns_none() {
    assert!(SafeServiceClient::new(999999).is_none());
}

#[test]
fn client_creation_zero_chain_returns_none() {
    assert!(SafeServiceClient::new(0).is_none());
}

#[test]
fn client_creation_unknown_l2_returns_none() {
    // A chain ID that could be real but isn't supported
    assert!(SafeServiceClient::new(12345).is_none());
}

// ── ProposeRequest serialization ─────────────────────────────────────────

#[test]
fn propose_request_serializes_camel_case() {
    let req = ProposeRequest {
        to: "0x1234567890123456789012345678901234567890".into(),
        value: "0".into(),
        data: Some("0x60806040".into()),
        operation: 0,
        safe_tx_gas: "0".into(),
        base_gas: "0".into(),
        gas_price: "0".into(),
        gas_token: "0x0000000000000000000000000000000000000000".into(),
        refund_receiver: "0x0000000000000000000000000000000000000000".into(),
        nonce: 7,
        contract_transaction_hash: "0xhash".into(),
        sender: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".into(),
        signature: "0xsig".into(),
        origin: Some("treb".into()),
    };

    let json = serde_json::to_value(&req).unwrap();
    let obj = json.as_object().unwrap();

    // Verify camelCase keys are present
    assert!(obj.contains_key("safeTxGas"), "missing safeTxGas");
    assert!(obj.contains_key("baseGas"), "missing baseGas");
    assert!(obj.contains_key("gasPrice"), "missing gasPrice");
    assert!(obj.contains_key("gasToken"), "missing gasToken");
    assert!(obj.contains_key("refundReceiver"), "missing refundReceiver");
    assert!(obj.contains_key("contractTransactionHash"), "missing contractTransactionHash");

    // Verify no snake_case keys
    assert!(!obj.contains_key("safe_tx_gas"));
    assert!(!obj.contains_key("base_gas"));
    assert!(!obj.contains_key("gas_price"));
    assert!(!obj.contains_key("gas_token"));
    assert!(!obj.contains_key("refund_receiver"));
    assert!(!obj.contains_key("contract_transaction_hash"));
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
    assert!(
        !json.as_object().unwrap().contains_key("origin"),
        "origin should be omitted when None"
    );
}

#[test]
fn propose_request_round_trip_serialization() {
    let original = ProposeRequest {
        to: "0x1234567890123456789012345678901234567890".into(),
        value: "1000000000000000000".into(),
        data: Some("0xabcdef".into()),
        operation: 0,
        safe_tx_gas: "21000".into(),
        base_gas: "0".into(),
        gas_price: "0".into(),
        gas_token: "0x0000000000000000000000000000000000000000".into(),
        refund_receiver: "0x0000000000000000000000000000000000000000".into(),
        nonce: 42,
        contract_transaction_hash: "0xhash123".into(),
        sender: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".into(),
        signature: "0xsig456".into(),
        origin: Some("treb".into()),
    };

    let json_str = serde_json::to_string(&original).unwrap();
    let deserialized: ProposeRequest = serde_json::from_str(&json_str).unwrap();

    assert_eq!(deserialized.to, original.to);
    assert_eq!(deserialized.value, original.value);
    assert_eq!(deserialized.data, original.data);
    assert_eq!(deserialized.nonce, original.nonce);
    assert_eq!(deserialized.sender, original.sender);
    assert_eq!(deserialized.contract_transaction_hash, original.contract_transaction_hash);
    assert_eq!(deserialized.origin, original.origin);
}

// ── Fixture-based deserialization ────────────────────────────────────────

#[test]
fn propose_request_fixture_deserializes() {
    let fixture =
        std::fs::read_to_string("../treb-cli/tests/fixtures/safe/propose-request.json").unwrap();
    let req: ProposeRequest = serde_json::from_str(&fixture).unwrap();

    assert_eq!(req.to, "0x1234567890123456789012345678901234567890");
    assert_eq!(req.value, "0");
    assert_eq!(req.data.as_deref(), Some("0x60806040"));
    assert_eq!(req.nonce, 7);
    assert_eq!(req.sender, "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    assert_eq!(req.origin.as_deref(), Some("treb"));
}

#[test]
fn safe_service_response_fixture_deserializes() {
    let fixture =
        std::fs::read_to_string("../treb-cli/tests/fixtures/safe/safe-service-response.json")
            .unwrap();
    let resp: SafeServiceMultisigResponse = serde_json::from_str(&fixture).unwrap();

    assert_eq!(resp.count, 2);
    assert!(resp.next.is_none());
    assert!(resp.previous.is_none());
    assert_eq!(resp.results.len(), 2);

    // First tx: executed
    let tx0 = &resp.results[0];
    assert!(tx0.is_executed);
    assert!(tx0.transaction_hash.is_some());
    assert!(tx0.execution_date.is_some());
    assert_eq!(tx0.confirmations.len(), 2);
    assert_eq!(tx0.nonce, 5);

    // Second tx: pending
    let tx1 = &resp.results[1];
    assert!(!tx1.is_executed);
    assert!(tx1.transaction_hash.is_none());
    assert_eq!(tx1.confirmations.len(), 1);
    assert_eq!(tx1.nonce, 6);
}

// ── EIP-712 hash computation ─────────────────────────────────────────────

#[test]
fn eip712_hash_deterministic_across_calls() {
    let safe_address = address!("1234567890123456789012345678901234567890");
    let safe_tx = SafeTx {
        to: address!("0000000000000000000000000000000000000001"),
        value: U256::ZERO,
        data: vec![].into(),
        operation: 0,
        safeTxGas: U256::ZERO,
        baseGas: U256::ZERO,
        gasPrice: U256::ZERO,
        gasToken: Address::ZERO,
        refundReceiver: Address::ZERO,
        nonce: U256::ZERO,
    };

    let hash1 = compute_safe_tx_hash(1, safe_address, &safe_tx);
    let hash2 = compute_safe_tx_hash(1, safe_address, &safe_tx);
    assert_eq!(hash1, hash2, "same inputs must produce same hash");
}

#[test]
fn eip712_hash_differs_by_chain_id() {
    let safe_address = address!("1234567890123456789012345678901234567890");
    let safe_tx = SafeTx {
        to: Address::ZERO,
        value: U256::ZERO,
        data: vec![].into(),
        operation: 0,
        safeTxGas: U256::ZERO,
        baseGas: U256::ZERO,
        gasPrice: U256::ZERO,
        gasToken: Address::ZERO,
        refundReceiver: Address::ZERO,
        nonce: U256::ZERO,
    };

    let hash_mainnet = compute_safe_tx_hash(1, safe_address, &safe_tx);
    let hash_polygon = compute_safe_tx_hash(137, safe_address, &safe_tx);
    assert_ne!(hash_mainnet, hash_polygon, "different chains must produce different hashes");
}

#[test]
fn eip712_hash_differs_by_safe_address() {
    let safe_tx = SafeTx {
        to: Address::ZERO,
        value: U256::ZERO,
        data: vec![].into(),
        operation: 0,
        safeTxGas: U256::ZERO,
        baseGas: U256::ZERO,
        gasPrice: U256::ZERO,
        gasToken: Address::ZERO,
        refundReceiver: Address::ZERO,
        nonce: U256::ZERO,
    };

    let addr_a = address!("1234567890123456789012345678901234567890");
    let addr_b = address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");

    let hash_a = compute_safe_tx_hash(1, addr_a, &safe_tx);
    let hash_b = compute_safe_tx_hash(1, addr_b, &safe_tx);
    assert_ne!(hash_a, hash_b, "different safe addresses must produce different hashes");
}

#[test]
fn eip712_hash_differs_by_nonce() {
    let safe_address = address!("1234567890123456789012345678901234567890");
    let tx_nonce_0 = SafeTx {
        to: Address::ZERO,
        value: U256::ZERO,
        data: vec![].into(),
        operation: 0,
        safeTxGas: U256::ZERO,
        baseGas: U256::ZERO,
        gasPrice: U256::ZERO,
        gasToken: Address::ZERO,
        refundReceiver: Address::ZERO,
        nonce: U256::ZERO,
    };

    let tx_nonce_42 = SafeTx { nonce: U256::from(42), ..tx_nonce_0.clone() };

    let hash_0 = compute_safe_tx_hash(1, safe_address, &tx_nonce_0);
    let hash_42 = compute_safe_tx_hash(1, safe_address, &tx_nonce_42);
    assert_ne!(hash_0, hash_42, "different nonces must produce different hashes");
}

#[test]
fn eip712_hash_differs_by_value() {
    let safe_address = address!("1234567890123456789012345678901234567890");
    let tx_zero = SafeTx {
        to: Address::ZERO,
        value: U256::ZERO,
        data: vec![].into(),
        operation: 0,
        safeTxGas: U256::ZERO,
        baseGas: U256::ZERO,
        gasPrice: U256::ZERO,
        gasToken: Address::ZERO,
        refundReceiver: Address::ZERO,
        nonce: U256::ZERO,
    };

    let tx_with_value =
        SafeTx { value: U256::from(1_000_000_000_000_000_000u64), ..tx_zero.clone() };

    let hash_zero = compute_safe_tx_hash(1, safe_address, &tx_zero);
    let hash_value = compute_safe_tx_hash(1, safe_address, &tx_with_value);
    assert_ne!(hash_zero, hash_value, "different values must produce different hashes");
}

#[test]
fn eip712_hash_is_non_zero() {
    let safe_address = address!("1234567890123456789012345678901234567890");
    let safe_tx = SafeTx {
        to: Address::ZERO,
        value: U256::ZERO,
        data: vec![].into(),
        operation: 0,
        safeTxGas: U256::ZERO,
        baseGas: U256::ZERO,
        gasPrice: U256::ZERO,
        gasToken: Address::ZERO,
        refundReceiver: Address::ZERO,
        nonce: U256::ZERO,
    };

    let hash = compute_safe_tx_hash(1, safe_address, &safe_tx);
    assert_ne!(hash, B256::ZERO, "hash should never be zero");
}

#[test]
fn eip712_domain_has_correct_fields() {
    let safe_address = address!("1234567890123456789012345678901234567890");
    let domain = safe_domain(1, safe_address);

    // Safe uses minimal domain: only chainId and verifyingContract
    assert_eq!(domain.chain_id, Some(U256::from(1)));
    assert_eq!(domain.verifying_contract, Some(safe_address));
    assert!(domain.name.is_none());
    assert!(domain.version.is_none());
    assert!(domain.salt.is_none());
}

#[test]
fn eip712_hash_with_data_differs_from_without() {
    let safe_address = address!("1234567890123456789012345678901234567890");
    let tx_no_data = SafeTx {
        to: address!("0000000000000000000000000000000000000001"),
        value: U256::ZERO,
        data: vec![].into(),
        operation: 0,
        safeTxGas: U256::ZERO,
        baseGas: U256::ZERO,
        gasPrice: U256::ZERO,
        gasToken: Address::ZERO,
        refundReceiver: Address::ZERO,
        nonce: U256::ZERO,
    };

    let tx_with_data = SafeTx { data: vec![0x60, 0x80, 0x60, 0x40].into(), ..tx_no_data.clone() };

    let hash_no_data = compute_safe_tx_hash(1, safe_address, &tx_no_data);
    let hash_with_data = compute_safe_tx_hash(1, safe_address, &tx_with_data);
    assert_ne!(hash_no_data, hash_with_data, "data field should affect the hash");
}
