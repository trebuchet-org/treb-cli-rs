//! Integration tests for the event parsing pipeline with captured log fixtures.
//!
//! These tests validate the full event parsing flow (raw logs -> decode ->
//! extract deployments -> detect proxies -> link relationships) against
//! realistic log fixtures stored as JSON files.

use std::path::PathBuf;
use std::sync::Once;

use alloy_primitives::{address, b256, Address, Bytes, Log, LogData, B256};
use alloy_sol_types::SolEvent;
use serde::{Deserialize, Serialize};
use treb_core::types::enums::DeploymentMethod;
use treb_forge::events::{
    decode_events, detect_proxy_relationships, extract_collisions, extract_deployments,
    link_proxy_to_deployment, AdminChanged, ContractCreation_0, ContractDeployed,
    CreateXEvent, DeploymentCollision, DeploymentDetails, ParsedEvent, ProxyType, TrebEvent,
    Upgraded,
};

// ---------------------------------------------------------------------------
// Fixture JSON format
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct LogEntry {
    address: String,
    topics: Vec<String>,
    data: String,
}

#[derive(Serialize, Deserialize)]
struct Fixture {
    description: String,
    logs: Vec<LogEntry>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("events")
}

fn make_log(addr: Address, log_data: LogData) -> Log {
    Log::new(addr, log_data.topics().to_vec(), log_data.data.clone()).expect("valid log data")
}

fn log_to_entry(log: &Log) -> LogEntry {
    LogEntry {
        address: format!("{}", log.address),
        topics: log.data.topics().iter().map(|t| format!("{t}")).collect(),
        data: format!("0x{}", alloy_primitives::hex::encode(log.data.data.as_ref())),
    }
}

fn entry_to_log(entry: &LogEntry) -> Log {
    let addr: Address = entry.address.parse().expect("valid address");
    let topics: Vec<B256> = entry
        .topics
        .iter()
        .map(|t| t.parse().expect("valid topic"))
        .collect();
    let data_bytes: Vec<u8> =
        alloy_primitives::hex::decode(&entry.data).expect("valid hex data");
    Log::new(addr, topics, Bytes::from(data_bytes)).expect("valid log")
}

fn write_fixture(name: &str, fixture: &Fixture) {
    let dir = fixtures_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.json"));
    let json = serde_json::to_string_pretty(fixture).unwrap();
    std::fs::write(path, json).unwrap();
}

fn load_fixture(name: &str) -> Fixture {
    let path = fixtures_dir().join(format!("{name}.json"));
    let data = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("fixture file should exist: {}", path.display()));
    serde_json::from_str(&data).unwrap()
}

fn load_logs(name: &str) -> Vec<Log> {
    let fixture = load_fixture(name);
    fixture.logs.iter().map(entry_to_log).collect()
}

// ---------------------------------------------------------------------------
// Fixture generation (runs once via std::sync::Once)
// ---------------------------------------------------------------------------

static INIT: Once = Once::new();

fn ensure_fixtures() {
    INIT.call_once(|| {
        generate_simple_deploy();
        generate_create2_deploy();
        generate_proxy_deploy();
        generate_multi_deploy();
        generate_collision();
    });
}

fn generate_simple_deploy() {
    let event = ContractDeployed {
        deployer: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
        location: address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
        transactionId: b256!(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ),
        deployment: DeploymentDetails {
            artifact: "Counter".to_string(),
            label: "counter-v1".to_string(),
            entropy: String::new(),
            salt: B256::ZERO,
            bytecodeHash: b256!(
                "1111111111111111111111111111111111111111111111111111111111111111"
            ),
            initCodeHash: b256!(
                "2222222222222222222222222222222222222222222222222222222222222222"
            ),
            constructorArgs: Bytes::new(),
            createStrategy: "create".to_string(),
        },
    };
    let log = make_log(Address::ZERO, event.encode_log_data());

    write_fixture(
        "simple_deploy",
        &Fixture {
            description: "Simple CREATE deployment of Counter contract".to_string(),
            logs: vec![log_to_entry(&log)],
        },
    );
}

fn generate_create2_deploy() {
    let deployer = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    let location = address!("Cf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9");
    let salt = b256!("abcdef0000000000000000000000000000000000000000000000000000000001");

    let deployed = ContractDeployed {
        deployer,
        location,
        transactionId: b256!(
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        ),
        deployment: DeploymentDetails {
            artifact: "Token".to_string(),
            label: "token-v1".to_string(),
            entropy: "deploy-salt-1".to_string(),
            salt,
            bytecodeHash: b256!(
                "3333333333333333333333333333333333333333333333333333333333333333"
            ),
            initCodeHash: b256!(
                "4444444444444444444444444444444444444444444444444444444444444444"
            ),
            constructorArgs: Bytes::from(vec![0x00, 0x00, 0x00, 0x12]),
            createStrategy: "create2".to_string(),
        },
    };
    let log1 = make_log(Address::ZERO, deployed.encode_log_data());

    let creation = ContractCreation_0 {
        newContract: location,
        salt,
    };
    let log2 = make_log(Address::ZERO, creation.encode_log_data());

    write_fixture(
        "create2_deploy",
        &Fixture {
            description: "CREATE2 deployment with ContractCreation event from CreateX".to_string(),
            logs: vec![log_to_entry(&log1), log_to_entry(&log2)],
        },
    );
}

fn generate_proxy_deploy() {
    let deployer = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    let impl_addr = address!("9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0");
    let proxy_addr = address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512");
    let admin_addr = address!("70997970C51812dc3A010C7d01b50e0d17dc79C8");

    // Implementation deployment
    let impl_deployed = ContractDeployed {
        deployer,
        location: impl_addr,
        transactionId: b256!(
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
        ),
        deployment: DeploymentDetails {
            artifact: "MyLogic".to_string(),
            label: "logic-v1".to_string(),
            entropy: String::new(),
            salt: B256::ZERO,
            bytecodeHash: b256!(
                "5555555555555555555555555555555555555555555555555555555555555555"
            ),
            initCodeHash: b256!(
                "6666666666666666666666666666666666666666666666666666666666666666"
            ),
            constructorArgs: Bytes::new(),
            createStrategy: "create".to_string(),
        },
    };
    let log1 = make_log(Address::ZERO, impl_deployed.encode_log_data());

    // Proxy deployment
    let proxy_deployed = ContractDeployed {
        deployer,
        location: proxy_addr,
        transactionId: b256!(
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
        ),
        deployment: DeploymentDetails {
            artifact: "TransparentUpgradeableProxy".to_string(),
            label: "proxy-v1".to_string(),
            entropy: String::new(),
            salt: B256::ZERO,
            bytecodeHash: b256!(
                "7777777777777777777777777777777777777777777777777777777777777777"
            ),
            initCodeHash: b256!(
                "8888888888888888888888888888888888888888888888888888888888888888"
            ),
            constructorArgs: Bytes::from(vec![0xaa, 0xbb]),
            createStrategy: "create".to_string(),
        },
    };
    let log2 = make_log(Address::ZERO, proxy_deployed.encode_log_data());

    // Upgraded event emitted from the proxy address
    let upgraded = Upgraded {
        implementation: impl_addr,
    };
    let log3 = make_log(proxy_addr, upgraded.encode_log_data());

    // AdminChanged event emitted from the proxy address
    let admin_changed = AdminChanged {
        previousAdmin: Address::ZERO,
        newAdmin: admin_addr,
    };
    let log4 = make_log(proxy_addr, admin_changed.encode_log_data());

    write_fixture(
        "proxy_deploy",
        &Fixture {
            description: "Transparent proxy deployment with Upgraded and AdminChanged events"
                .to_string(),
            logs: vec![
                log_to_entry(&log1),
                log_to_entry(&log2),
                log_to_entry(&log3),
                log_to_entry(&log4),
            ],
        },
    );
}

fn generate_multi_deploy() {
    let deployer = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    let addr1 = address!("5FbDB2315678afecb367f032d93F642f64180aa3");
    let addr2 = address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512");
    let addr3 = address!("9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0");
    let impl_addr = address!("Cf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9");

    // Deployment 1: simple CREATE
    let d1 = ContractDeployed {
        deployer,
        location: addr1,
        transactionId: b256!(
            "1111111111111111111111111111111111111111111111111111111111111111"
        ),
        deployment: DeploymentDetails {
            artifact: "Counter".to_string(),
            label: "counter".to_string(),
            entropy: String::new(),
            salt: B256::ZERO,
            bytecodeHash: B256::ZERO,
            initCodeHash: B256::ZERO,
            constructorArgs: Bytes::new(),
            createStrategy: "create".to_string(),
        },
    };
    let log1 = make_log(Address::ZERO, d1.encode_log_data());

    // Deployment 2: CREATE2
    let d2 = ContractDeployed {
        deployer,
        location: addr2,
        transactionId: b256!(
            "2222222222222222222222222222222222222222222222222222222222222222"
        ),
        deployment: DeploymentDetails {
            artifact: "Token".to_string(),
            label: "token".to_string(),
            entropy: String::new(),
            salt: B256::ZERO,
            bytecodeHash: B256::ZERO,
            initCodeHash: B256::ZERO,
            constructorArgs: Bytes::new(),
            createStrategy: "create2".to_string(),
        },
    };
    let log2 = make_log(Address::ZERO, d2.encode_log_data());

    // Deployment 3: UUPS proxy
    let d3 = ContractDeployed {
        deployer,
        location: addr3,
        transactionId: b256!(
            "3333333333333333333333333333333333333333333333333333333333333333"
        ),
        deployment: DeploymentDetails {
            artifact: "MyProxy".to_string(),
            label: "proxy".to_string(),
            entropy: String::new(),
            salt: B256::ZERO,
            bytecodeHash: B256::ZERO,
            initCodeHash: B256::ZERO,
            constructorArgs: Bytes::new(),
            createStrategy: "create".to_string(),
        },
    };
    let log3 = make_log(Address::ZERO, d3.encode_log_data());

    // Upgraded event for addr3 (UUPS — no AdminChanged)
    let upgraded = Upgraded {
        implementation: impl_addr,
    };
    let log4 = make_log(addr3, upgraded.encode_log_data());

    // An unknown log (unrecognised topic)
    let unknown_topic =
        b256!("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");
    let log5 = Log::new(Address::ZERO, vec![unknown_topic], Bytes::new()).expect("valid log");

    write_fixture(
        "multi_deploy",
        &Fixture {
            description: "Multiple deployments with UUPS proxy and unknown log".to_string(),
            logs: vec![
                log_to_entry(&log1),
                log_to_entry(&log2),
                log_to_entry(&log3),
                log_to_entry(&log4),
                log_to_entry(&log5),
            ],
        },
    );
}

fn generate_collision() {
    let collision_event = DeploymentCollision {
        existingContract: address!("Cf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9"),
        deployment: DeploymentDetails {
            artifact: "Token".to_string(),
            label: "token-v2".to_string(),
            entropy: String::new(),
            salt: b256!("abcdef0000000000000000000000000000000000000000000000000000000001"),
            bytecodeHash: b256!(
                "9999999999999999999999999999999999999999999999999999999999999999"
            ),
            initCodeHash: b256!(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaab"
            ),
            constructorArgs: Bytes::new(),
            createStrategy: "create2".to_string(),
        },
    };
    let log = make_log(Address::ZERO, collision_event.encode_log_data());

    write_fixture(
        "collision",
        &Fixture {
            description: "Deployment collision with existing contract".to_string(),
            logs: vec![log_to_entry(&log)],
        },
    );
}

// ---------------------------------------------------------------------------
// Integration tests
// ---------------------------------------------------------------------------

/// Simple deploy round-trip: raw logs -> decode -> extract deployment.
#[test]
fn simple_deploy_round_trip() {
    ensure_fixtures();
    let logs = load_logs("simple_deploy");

    // Decode
    let events = decode_events(&logs);
    assert_eq!(events.len(), 1);

    match &events[0] {
        ParsedEvent::Treb(boxed) => match boxed.as_ref() {
            TrebEvent::ContractDeployed(d) => {
                assert_eq!(
                    d.location,
                    address!("5FbDB2315678afecb367f032d93F642f64180aa3")
                );
                assert_eq!(d.deployment.label, "counter-v1");
                assert_eq!(d.deployment.artifact, "Counter");
                assert_eq!(d.deployment.createStrategy, "create");
            }
            other => panic!("expected ContractDeployed, got {other:?}"),
        },
        other => panic!("expected Treb event, got {other:?}"),
    }

    // Extract
    let deployments = extract_deployments(&events, None);
    assert_eq!(deployments.len(), 1);
    assert_eq!(
        deployments[0].address,
        address!("5FbDB2315678afecb367f032d93F642f64180aa3")
    );
    assert_eq!(deployments[0].label, "counter-v1");
    assert_eq!(deployments[0].contract_name, "Counter");
    assert_eq!(deployments[0].strategy, DeploymentMethod::Create);
}

/// CREATE2 deploy with salt: verifies ContractCreation event is decoded.
#[test]
fn create2_deploy_with_salt_decoded() {
    ensure_fixtures();
    let logs = load_logs("create2_deploy");

    let events = decode_events(&logs);
    assert_eq!(events.len(), 2);

    // First: ContractDeployed
    match &events[0] {
        ParsedEvent::Treb(boxed) => match boxed.as_ref() {
            TrebEvent::ContractDeployed(d) => {
                assert_eq!(d.deployment.createStrategy, "create2");
                assert_eq!(
                    d.deployment.salt,
                    b256!("abcdef0000000000000000000000000000000000000000000000000000000001")
                );
                assert_eq!(d.deployment.artifact, "Token");
            }
            other => panic!("expected ContractDeployed, got {other:?}"),
        },
        other => panic!("expected Treb event, got {other:?}"),
    }

    // Second: ContractCreation (with salt)
    match &events[1] {
        ParsedEvent::CreateX(CreateXEvent::ContractCreationWithSalt(c)) => {
            assert_eq!(
                c.newContract,
                address!("Cf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9")
            );
            assert_eq!(
                c.salt,
                b256!("abcdef0000000000000000000000000000000000000000000000000000000001")
            );
        }
        other => panic!("expected ContractCreationWithSalt, got {other:?}"),
    }

    // Extract deployment and verify strategy
    let deployments = extract_deployments(&events, None);
    assert_eq!(deployments.len(), 1);
    assert_eq!(deployments[0].strategy, DeploymentMethod::Create2);
    assert_eq!(
        deployments[0].salt,
        b256!("abcdef0000000000000000000000000000000000000000000000000000000001")
    );
    assert_eq!(deployments[0].entropy, "deploy-salt-1");
}

/// Proxy relationship detection: identifies Transparent type with correct
/// implementation and admin addresses.
#[test]
fn proxy_relationship_transparent_detection() {
    ensure_fixtures();
    let logs = load_logs("proxy_deploy");

    let events = decode_events(&logs);
    assert_eq!(events.len(), 4);

    let proxy_addr = address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512");
    let impl_addr = address!("9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0");
    let admin_addr = address!("70997970C51812dc3A010C7d01b50e0d17dc79C8");

    // Detect proxy relationships
    let relationships = detect_proxy_relationships(&events);
    assert_eq!(relationships.len(), 1);

    let rel = relationships
        .get(&proxy_addr)
        .expect("proxy relationship should exist");
    assert_eq!(rel.proxy_type, ProxyType::Transparent);
    assert_eq!(rel.implementation, Some(impl_addr));
    assert_eq!(rel.admin, Some(admin_addr));

    // Also verify deployments are extractable
    let deployments = extract_deployments(&events, None);
    assert_eq!(deployments.len(), 2);
    assert_eq!(deployments[0].contract_name, "MyLogic");
    assert_eq!(deployments[1].contract_name, "TransparentUpgradeableProxy");

    // Link proxy to its deployment
    let idx = link_proxy_to_deployment(rel, &deployments);
    assert_eq!(idx, Some(1));
    assert_eq!(deployments[idx.unwrap()].address, proxy_addr);
}

/// Full pipeline: decode -> extract deployments -> extract proxies -> link.
#[test]
fn full_pipeline_end_to_end() {
    ensure_fixtures();
    let logs = load_logs("multi_deploy");

    // Step 1: Decode all logs
    let events = decode_events(&logs);
    assert_eq!(events.len(), 5, "3 deploys + 1 upgraded + 1 unknown");

    // Verify types in order
    assert!(matches!(
        &events[0],
        ParsedEvent::Treb(b) if matches!(b.as_ref(), TrebEvent::ContractDeployed(_))
    ));
    assert!(matches!(
        &events[1],
        ParsedEvent::Treb(b) if matches!(b.as_ref(), TrebEvent::ContractDeployed(_))
    ));
    assert!(matches!(
        &events[2],
        ParsedEvent::Treb(b) if matches!(b.as_ref(), TrebEvent::ContractDeployed(_))
    ));
    assert!(matches!(&events[3], ParsedEvent::Proxy(_)));
    assert!(matches!(&events[4], ParsedEvent::Unknown(_)));

    // Step 2: Extract deployments
    let deployments = extract_deployments(&events, None);
    assert_eq!(deployments.len(), 3);
    assert_eq!(deployments[0].contract_name, "Counter");
    assert_eq!(deployments[0].strategy, DeploymentMethod::Create);
    assert_eq!(deployments[1].contract_name, "Token");
    assert_eq!(deployments[1].strategy, DeploymentMethod::Create2);
    assert_eq!(deployments[2].contract_name, "MyProxy");

    // Step 3: Extract collisions (none in this fixture)
    let collisions = extract_collisions(&events);
    assert!(collisions.is_empty());

    // Step 4: Detect proxy relationships
    let relationships = detect_proxy_relationships(&events);
    assert_eq!(relationships.len(), 1);

    let proxy_addr = address!("9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0");
    let rel = relationships
        .get(&proxy_addr)
        .expect("proxy relationship should exist");
    assert_eq!(rel.proxy_type, ProxyType::UUPS);
    assert_eq!(
        rel.implementation,
        Some(address!("Cf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9"))
    );

    // Step 5: Link proxy to deployment
    let idx = link_proxy_to_deployment(rel, &deployments);
    assert_eq!(idx, Some(2));
    assert_eq!(deployments[idx.unwrap()].contract_name, "MyProxy");
    assert_eq!(deployments[idx.unwrap()].address, proxy_addr);
}

/// Collision fixture round-trip: decode and extract collision data.
#[test]
fn collision_fixture_round_trip() {
    ensure_fixtures();
    let logs = load_logs("collision");

    let events = decode_events(&logs);
    assert_eq!(events.len(), 1);

    // No deployments from a collision event
    let deployments = extract_deployments(&events, None);
    assert!(deployments.is_empty());

    // One collision
    let collisions = extract_collisions(&events);
    assert_eq!(collisions.len(), 1);
    assert_eq!(
        collisions[0].existing_address,
        address!("Cf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9")
    );
    assert_eq!(collisions[0].contract_name, "Token");
    assert_eq!(collisions[0].label, "token-v2");
    assert_eq!(collisions[0].strategy, DeploymentMethod::Create2);
    assert_eq!(
        collisions[0].salt,
        b256!("abcdef0000000000000000000000000000000000000000000000000000000001")
    );

    // No proxy relationships from collision
    let relationships = detect_proxy_relationships(&events);
    assert!(relationships.is_empty());
}

/// Verify fixture files are valid JSON and round-trip correctly.
#[test]
fn fixture_files_are_valid_json() {
    ensure_fixtures();
    let names = [
        "simple_deploy",
        "create2_deploy",
        "proxy_deploy",
        "multi_deploy",
        "collision",
    ];
    for name in &names {
        let fixture = load_fixture(name);
        assert!(!fixture.description.is_empty(), "{name} should have a description");
        assert!(!fixture.logs.is_empty(), "{name} should have logs");

        // Verify each log entry round-trips through serialization
        for (i, entry) in fixture.logs.iter().enumerate() {
            let log = entry_to_log(entry);
            let re_entry = log_to_entry(&log);
            assert_eq!(
                entry.address.to_lowercase(),
                re_entry.address.to_lowercase(),
                "{name} log[{i}] address should round-trip"
            );
            assert_eq!(
                entry.topics.len(),
                re_entry.topics.len(),
                "{name} log[{i}] topic count should match"
            );
            assert_eq!(
                entry.data.to_lowercase(),
                re_entry.data.to_lowercase(),
                "{name} log[{i}] data should round-trip"
            );
        }
    }
}
