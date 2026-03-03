//! Golden file serde round-trip tests.
//!
//! These tests verify byte-level JSON compatibility between the Rust types
//! and Go-generated JSON fixtures from a real treb registry.
//!
//! Strategy: deserialize JSON → Rust type → re-serialize → parse both as
//! serde_json::Value → assert equality.

use std::collections::HashMap;

use treb_core::types::{Deployment, SafeTransaction, Transaction};

/// Deserialize `json_str` into `T`, re-serialize, and verify the two
/// `serde_json::Value` representations are identical.
fn assert_round_trip<T>(json_str: &str)
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let original: serde_json::Value =
        serde_json::from_str(json_str).expect("fixture is not valid JSON");
    let typed: T = serde_json::from_str(json_str).expect("failed to deserialize fixture into type");
    let reserialized_str = serde_json::to_string_pretty(&typed).expect("failed to re-serialize");
    let reserialized: serde_json::Value =
        serde_json::from_str(&reserialized_str).expect("re-serialized JSON is invalid");

    assert_eq!(
        original, reserialized,
        "round-trip mismatch:\n--- original ---\n{}\n--- reserialized ---\n{}",
        json_str, reserialized_str
    );
}

// ---------------------------------------------------------------------------
// Individual type round-trips
// ---------------------------------------------------------------------------

#[test]
fn deployment_round_trip() {
    let json = include_str!("fixtures/deployment.json");
    assert_round_trip::<Deployment>(json);
}

#[test]
fn deployment_with_proxy_round_trip() {
    let json = include_str!("fixtures/deployment_with_proxy.json");
    assert_round_trip::<Deployment>(json);
}

#[test]
fn transaction_round_trip() {
    let json = include_str!("fixtures/transaction.json");
    assert_round_trip::<Transaction>(json);
}

#[test]
fn safe_transaction_round_trip() {
    let json = include_str!("fixtures/safe_transaction.json");
    assert_round_trip::<SafeTransaction>(json);
}

// ---------------------------------------------------------------------------
// Map (registry format) round-trips
// ---------------------------------------------------------------------------

#[test]
fn deployments_map_round_trip() {
    let json = include_str!("fixtures/deployments_map.json");
    assert_round_trip::<HashMap<String, Deployment>>(json);
}

#[test]
fn transactions_map_round_trip() {
    let json = include_str!("fixtures/transactions_map.json");
    assert_round_trip::<HashMap<String, Transaction>>(json);
}

#[test]
fn safe_txs_map_round_trip() {
    let json = include_str!("fixtures/safe_txs_map.json");
    assert_round_trip::<HashMap<String, SafeTransaction>>(json);
}
