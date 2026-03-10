use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde_json::Value;
use tempfile::TempDir;
use treb_core::types::{DeploymentMethod, DeploymentType, TransactionStatus};
use treb_registry::{
    DEPLOYMENTS_FILE, DeploymentStore, SAFE_TXS_FILE, SafeTransactionStore, TRANSACTIONS_FILE,
    TransactionStore,
};

const GO_COMPAT_FIXTURES_DIR: &str = "tests/fixtures/go-compat";
const DEPLOYMENTS_FIXTURE_COUNT: usize = 13;
const TRANSACTIONS_FIXTURE_COUNT: usize = 10;
const SAFE_TXS_FIXTURE_COUNT: usize = 8;

fn go_compat_fixture_path(file_name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(GO_COMPAT_FIXTURES_DIR).join(file_name)
}

fn seed_registry_file(dir: &TempDir, target_file: &str, fixture_file: &str) {
    let fixture_path = go_compat_fixture_path(fixture_file);
    let fixture = fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", fixture_path.display()));
    fs::write(dir.path().join(target_file), fixture)
        .unwrap_or_else(|e| panic!("failed to seed {target_file}: {e}"));
}

fn parse_utc(timestamp: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(timestamp).unwrap().with_timezone(&Utc)
}

fn load_fixture_json(file_name: &str) -> Value {
    let fixture_path = go_compat_fixture_path(file_name);
    let fixture = fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", fixture_path.display()));
    serde_json::from_str(&fixture)
        .unwrap_or_else(|e| panic!("failed to parse fixture {}: {e}", fixture_path.display()))
}

fn load_registry_json(dir: &TempDir, target_file: &str) -> Value {
    let path = dir.path().join(target_file);
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("failed to parse {} as JSON: {e}", path.display()))
}

fn assert_bare_registry_map(value: &Value, expected_entries: usize) {
    let object = value.as_object().expect("registry file should deserialize to a JSON object");
    assert_eq!(object.len(), expected_entries);
    assert!(!object.contains_key("_format"));
    assert!(!object.contains_key("entries"));
}

fn assert_go_compat_json(expected: &Value, actual: &Value, path: &str) {
    match (expected, actual) {
        (Value::Object(expected_obj), Value::Object(actual_obj)) => {
            let expected_keys: BTreeSet<&str> = expected_obj.keys().map(String::as_str).collect();
            let actual_keys: BTreeSet<&str> = actual_obj.keys().map(String::as_str).collect();
            assert_eq!(actual_keys, expected_keys, "object keys differ at {path}");

            for (key, expected_value) in expected_obj {
                let actual_value =
                    actual_obj.get(key).unwrap_or_else(|| panic!("missing key {key} at {path}"));
                let child_path = format!("{path}.{key}");
                assert_go_compat_json(expected_value, actual_value, &child_path);
            }
        }
        (Value::Array(expected_items), Value::Array(actual_items)) => {
            assert_eq!(actual_items.len(), expected_items.len(), "array length differs at {path}");

            for (index, (expected_item, actual_item)) in
                expected_items.iter().zip(actual_items.iter()).enumerate()
            {
                let child_path = format!("{path}[{index}]");
                assert_go_compat_json(expected_item, actual_item, &child_path);
            }
        }
        (Value::String(expected_str), Value::String(actual_str)) => {
            match (
                DateTime::parse_from_rfc3339(expected_str),
                DateTime::parse_from_rfc3339(actual_str),
            ) {
                (Ok(expected_ts), Ok(actual_ts)) => {
                    assert_eq!(
                        actual_ts.with_timezone(&Utc),
                        expected_ts.with_timezone(&Utc),
                        "timestamp differs at {path}"
                    );
                }
                _ => assert_eq!(actual_str, expected_str, "string differs at {path}"),
            }
        }
        _ => assert_eq!(actual, expected, "value differs at {path}"),
    }
}

fn set_string_field(value: &mut Value, entry_id: &str, field: &str, updated_value: &str) {
    value
        .get_mut(entry_id)
        .unwrap_or_else(|| panic!("missing fixture entry {entry_id}"))
        .get_mut(field)
        .unwrap_or_else(|| panic!("missing fixture field {field} on {entry_id}"))
        .clone_from(&Value::String(updated_value.to_string()));
}

fn collect_rs_files(dir: &Path) -> Vec<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    let mut files = Vec::new();

    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current)
            .unwrap_or_else(|e| panic!("failed to read directory {}: {e}", current.display()))
        {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }

    files.sort();
    files
}

#[test]
fn go_compat_deployments_deserialize() {
    let dir = TempDir::new().unwrap();
    seed_registry_file(&dir, DEPLOYMENTS_FILE, "deployments.json");

    let mut store = DeploymentStore::new(dir.path());
    store.load().unwrap();

    assert_eq!(store.count(), DEPLOYMENTS_FIXTURE_COUNT);

    let fpmm_factory = store
        .get("mainnet/42220/FPMMFactory:v3.0.0")
        .expect("FPMMFactory fixture entry should load");
    assert_eq!(fpmm_factory.chain_id, 42220);
    assert_eq!(fpmm_factory.address, "0x959597fD009876e6f53EbdB2F1c1Bc3f994579dF");
    assert_eq!(fpmm_factory.deployment_type, DeploymentType::Singleton);
    assert_eq!(fpmm_factory.deployment_strategy.method, DeploymentMethod::Create3);
    assert_eq!(
        fpmm_factory.tags.as_ref().unwrap(),
        &vec!["core".to_string(), "v3-release".to_string()]
    );

    let gbpm_proxy = store
        .get("mainnet/143/TransparentUpgradeableProxy:GBPm")
        .expect("GBPm proxy fixture entry should load");
    assert_eq!(gbpm_proxy.chain_id, 143);
    assert_eq!(gbpm_proxy.address, "0x39bb4E0a204412bB98e821d25e7d955e69d40Fd1");
    assert_eq!(gbpm_proxy.deployment_type, DeploymentType::Proxy);
    assert_eq!(gbpm_proxy.deployment_strategy.method, DeploymentMethod::Create3);
    assert_eq!(gbpm_proxy.created_at, parse_utc("2026-03-09T18:05:36.422290725+01:00"));
    let proxy_info = gbpm_proxy.proxy_info.as_ref().expect("proxy info should deserialize");
    assert_eq!(proxy_info.proxy_type, "UUPS");
    assert_eq!(proxy_info.implementation, "0x6A8ff60A89F3f359Fa16F45076d6DD1712B5e62e");

    let linked_list_library = store
        .get("virtual/42220/AddressSortedLinkedListWithMedian")
        .expect("library fixture entry should load");
    assert_eq!(linked_list_library.deployment_type, DeploymentType::Library);
    assert_eq!(linked_list_library.deployment_strategy.method, DeploymentMethod::Create2);
}

#[test]
fn go_compat_transactions_deserialize() {
    let dir = TempDir::new().unwrap();
    seed_registry_file(&dir, TRANSACTIONS_FILE, "transactions.json");

    let mut store = TransactionStore::new(dir.path());
    store.load().unwrap();

    assert_eq!(store.count(), TRANSACTIONS_FIXTURE_COUNT);

    let simulated = store
        .get("tx-internal-01ecf419ab2fc8e6498e8f57eecb94f1ac3b164d33848cdc862f44e7ac285477")
        .expect("simulated transaction fixture entry should load");
    assert_eq!(simulated.chain_id, 8453);
    assert_eq!(simulated.status, TransactionStatus::Simulated);
    assert_eq!(simulated.hash, "");
    assert_eq!(simulated.block_number, 0);
    assert_eq!(simulated.created_at, parse_utc("2025-08-07T12:42:27.417239+02:00"));
    assert_eq!(
        simulated.deployments,
        vec!["virtual/8453/ConstantProductPricingModule:v2.6.5".to_string()]
    );

    let deployed = store
        .get("tx-0x6d2704e94640fee89e0fb0d06ec94c19cb86c5ad558151d880512daea3984f37")
        .expect("CREATE deployment transaction fixture entry should load");
    assert_eq!(deployed.status, TransactionStatus::Executed);
    assert_eq!(deployed.block_number, 18059898);
    assert_eq!(deployed.operations.len(), 1);
    assert_eq!(deployed.operations[0].operation_type, "DEPLOY");
    assert_eq!(deployed.operations[0].method, "CREATE");
    assert_eq!(
        deployed.operations[0].result.get("address").and_then(|value| value.as_str()),
        Some("0xdbd4ea7ce0b15c9d57dc3fa47713477e4ef4fdcb")
    );

    let safe_batch = store
        .get("tx-0x21be0ff4e7e55b871b6b7359c9d247e93d00c05d42252863fab9f8053c91b047")
        .expect("safe transaction fixture entry should load");
    assert_eq!(safe_batch.chain_id, 42220);
    assert_eq!(safe_batch.status, TransactionStatus::Executed);
    assert_eq!(safe_batch.created_at, parse_utc("2025-09-02T15:23:18.508068+02:00"));
    let safe_context = safe_batch.safe_context.as_ref().expect("safe context should deserialize");
    assert_eq!(safe_context.safe_address, "0x32CB58b145d3f7e28c45cE4B2Cc31fa94248b23F");
    assert_eq!(
        safe_context.safe_tx_hash,
        "0x524b488d552102aff7396f95ce6753bc2bf66e4009a4c76034639a39c31a5e0e"
    );
    assert_eq!(safe_context.batch_index, 0);
}

#[test]
fn go_compat_safe_txs_deserialize() {
    let dir = TempDir::new().unwrap();
    seed_registry_file(&dir, SAFE_TXS_FILE, "safe-txs.json");

    let mut store = SafeTransactionStore::new(dir.path());
    store.load().unwrap();

    assert_eq!(store.count(), SAFE_TXS_FIXTURE_COUNT);

    let executed = store
        .get("0x524b488d552102aff7396f95ce6753bc2bf66e4009a4c76034639a39c31a5e0e")
        .expect("executed safe transaction fixture entry should load");
    assert_eq!(executed.chain_id, 42220);
    assert_eq!(executed.status, TransactionStatus::Executed);
    assert_eq!(
        executed.transaction_ids,
        vec!["tx-0x21be0ff4e7e55b871b6b7359c9d247e93d00c05d42252863fab9f8053c91b047".to_string()]
    );
    assert_eq!(executed.executed_at, Some(parse_utc("2025-09-02T15:23:18.508068+02:00")));
    assert_eq!(
        executed.execution_tx_hash,
        "0x21be0ff4e7e55b871b6b7359c9d247e93d00c05d42252863fab9f8053c91b047"
    );

    let base_safe = store
        .get("0x0360f6716adfaad9c5ee9ec6f8f4a5ad3d3c44e6b9c846028cb59027a768e1db")
        .expect("timezone-offset safe transaction fixture entry should load");
    assert_eq!(base_safe.chain_id, 8453);
    assert_eq!(base_safe.status, TransactionStatus::Executed);
    assert_eq!(base_safe.proposed_at, parse_utc("2025-08-28T12:13:02.073365+03:00"));

    let queued = store
        .get("0x4bc1c1b19989b6fef589847284fb74116d588bf0dd21a58c9cd15b8957eefa1f")
        .expect("queued safe transaction fixture entry should load");
    assert_eq!(queued.chain_id, 42220);
    assert_eq!(queued.status, TransactionStatus::Queued);
    assert_eq!(queued.proposed_at, parse_utc("2026-03-03T21:17:21.862708613+01:00"));
    assert!(queued.transaction_ids.is_empty());
    assert!(queued.executed_at.is_none());
    assert_eq!(queued.execution_tx_hash, "");
}

#[test]
fn go_compat_deployments_round_trip() {
    let mut expected = load_fixture_json("deployments.json");
    let dir = TempDir::new().unwrap();
    seed_registry_file(&dir, DEPLOYMENTS_FILE, "deployments.json");

    let mut store = DeploymentStore::new(dir.path());
    store.load().unwrap();

    let entry_id = "mainnet/42220/FPMMFactory:v3.0.0";
    let mut deployment = store.remove(entry_id).expect("fixture deployment should load");
    deployment
        .tags
        .as_mut()
        .expect("fixture deployment should have tags")
        .push("rust-round-trip".to_string());
    store.insert(deployment).unwrap();

    let tags = expected
        .get_mut(entry_id)
        .and_then(Value::as_object_mut)
        .and_then(|entry| entry.get_mut("tags"))
        .and_then(Value::as_array_mut)
        .expect("fixture deployment tags should exist");
    tags.push(Value::String("rust-round-trip".to_string()));

    let saved = load_registry_json(&dir, DEPLOYMENTS_FILE);
    assert_bare_registry_map(&saved, DEPLOYMENTS_FIXTURE_COUNT);
    assert_go_compat_json(&expected, &saved, "$");
}

#[test]
fn go_compat_transactions_round_trip() {
    let mut expected = load_fixture_json("transactions.json");
    let dir = TempDir::new().unwrap();
    seed_registry_file(&dir, TRANSACTIONS_FILE, "transactions.json");

    let mut store = TransactionStore::new(dir.path());
    store.load().unwrap();

    let entry_id = "tx-internal-01ecf419ab2fc8e6498e8f57eecb94f1ac3b164d33848cdc862f44e7ac285477";
    let mut transaction = store.get(entry_id).expect("fixture transaction should load").clone();
    transaction.environment = "virtual-rust".to_string();
    store.update(transaction).unwrap();
    set_string_field(&mut expected, entry_id, "environment", "virtual-rust");

    let saved = load_registry_json(&dir, TRANSACTIONS_FILE);
    assert_bare_registry_map(&saved, TRANSACTIONS_FIXTURE_COUNT);
    assert_go_compat_json(&expected, &saved, "$");
}

#[test]
fn go_compat_safe_txs_round_trip() {
    let mut expected = load_fixture_json("safe-txs.json");
    let dir = TempDir::new().unwrap();
    seed_registry_file(&dir, SAFE_TXS_FILE, "safe-txs.json");

    let mut store = SafeTransactionStore::new(dir.path());
    store.load().unwrap();

    let entry_id = "0x4bc1c1b19989b6fef589847284fb74116d588bf0dd21a58c9cd15b8957eefa1f";
    let mut safe_tx = store.get(entry_id).expect("fixture safe transaction should load").clone();
    safe_tx.proposed_by = "0x1111111111111111111111111111111111111111".to_string();
    store.update(safe_tx).unwrap();
    set_string_field(
        &mut expected,
        entry_id,
        "proposedBy",
        "0x1111111111111111111111111111111111111111",
    );

    let saved = load_registry_json(&dir, SAFE_TXS_FILE);
    assert_bare_registry_map(&saved, SAFE_TXS_FIXTURE_COUNT);
    assert_go_compat_json(&expected, &saved, "$");
}

#[test]
fn registry_models_do_not_use_deny_unknown_fields() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let source_dirs =
        [repo_root.join("crates/treb-core/src/types"), repo_root.join("crates/treb-registry/src")];

    for source_dir in source_dirs {
        for path in collect_rs_files(&source_dir) {
            let contents = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("failed to read source file {}: {e}", path.display()));
            assert!(
                !contents.contains("deny_unknown_fields"),
                "{} should not use #[serde(deny_unknown_fields)] for Go compatibility",
                path.display()
            );
        }
    }
}
