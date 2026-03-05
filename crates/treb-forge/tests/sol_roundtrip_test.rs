//! Integration tests verifying treb-sol bindings produce identical event data
//! through encode → decode roundtrips via `decode_events`.
//!
//! Each test constructs an event from treb-sol types, encodes it with
//! `SolEvent::encode_log_data`, passes the raw log through `decode_events`,
//! and asserts every field matches the original.

use alloy_primitives::{Address, B256, Bytes, Log, LogData, U256, address, b256};
use alloy_sol_types::SolEvent;
use treb_forge::events::{
    ContractCreation_0, ContractDeployed, CreateXEvent, DeploymentCollision, DeploymentDetails,
    ParsedEvent, ProxyEvent, SimulatedTransaction, Transaction, TransactionSimulated, TrebEvent,
    Upgraded, decode_events,
};

/// Helper: wrap encoded log data into a raw `Log`.
fn make_log(address: Address, log_data: LogData) -> Log {
    Log::new(address, log_data.topics().to_vec(), log_data.data.clone()).expect("valid log data")
}

#[test]
fn roundtrip_contract_deployed() {
    let deployer = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    let location = address!("5FbDB2315678afecb367f032d93F642f64180aa3");
    let tx_id = b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");

    let details = DeploymentDetails {
        artifact: "Counter".to_string(),
        label: "counter-v1".to_string(),
        entropy: "abc123".to_string(),
        salt: b256!("0000000000000000000000000000000000000000000000000000000000000001"),
        bytecodeHash: b256!("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"),
        initCodeHash: b256!("abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd"),
        constructorArgs: Bytes::from(vec![0x01, 0x02, 0x03]),
        createStrategy: "create2".to_string(),
    };

    let event = ContractDeployed {
        deployer,
        location,
        transactionId: tx_id,
        deployment: details,
    };

    let log = make_log(Address::ZERO, event.encode_log_data());
    let parsed = decode_events(&[log]);
    assert_eq!(parsed.len(), 1);

    match &parsed[0] {
        ParsedEvent::Treb(boxed) => match boxed.as_ref() {
            TrebEvent::ContractDeployed(d) => {
                assert_eq!(d.deployer, deployer);
                assert_eq!(d.location, location);
                assert_eq!(d.transactionId, tx_id);
                assert_eq!(d.deployment.artifact, "Counter");
                assert_eq!(d.deployment.label, "counter-v1");
                assert_eq!(d.deployment.entropy, "abc123");
                assert_eq!(d.deployment.salt, b256!("0000000000000000000000000000000000000000000000000000000000000001"));
                assert_eq!(d.deployment.bytecodeHash, b256!("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"));
                assert_eq!(d.deployment.initCodeHash, b256!("abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd"));
                assert_eq!(d.deployment.constructorArgs, Bytes::from(vec![0x01, 0x02, 0x03]));
                assert_eq!(d.deployment.createStrategy, "create2");
            }
            other => panic!("expected ContractDeployed, got {other:?}"),
        },
        other => panic!("expected Treb event, got {other:?}"),
    }
}

#[test]
fn roundtrip_transaction_simulated() {
    let sender = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    let target = address!("5FbDB2315678afecb367f032d93F642f64180aa3");
    let tx_id = b256!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    let call_data = Bytes::from(vec![0xaa, 0xbb, 0xcc]);
    let return_data = Bytes::from(vec![0xdd, 0xee]);

    let event = TransactionSimulated {
        transactions: vec![
            SimulatedTransaction {
                transactionId: tx_id,
                senderId: "deployer".to_string(),
                sender,
                returnData: return_data.clone(),
                transaction: Transaction {
                    to: target,
                    data: call_data.clone(),
                    value: U256::from(1000u64),
                },
            },
            SimulatedTransaction {
                transactionId: B256::ZERO,
                senderId: "treasury".to_string(),
                sender: Address::ZERO,
                returnData: Bytes::new(),
                transaction: Transaction {
                    to: Address::ZERO,
                    data: Bytes::new(),
                    value: U256::ZERO,
                },
            },
        ],
    };

    let log = make_log(Address::ZERO, event.encode_log_data());
    let parsed = decode_events(&[log]);
    assert_eq!(parsed.len(), 1);

    match &parsed[0] {
        ParsedEvent::Treb(boxed) => match boxed.as_ref() {
            TrebEvent::TransactionSimulated(d) => {
                assert_eq!(d.transactions.len(), 2);

                let tx0 = &d.transactions[0];
                assert_eq!(tx0.transactionId, tx_id);
                assert_eq!(tx0.senderId, "deployer");
                assert_eq!(tx0.sender, sender);
                assert_eq!(tx0.returnData, return_data);
                assert_eq!(tx0.transaction.to, target);
                assert_eq!(tx0.transaction.data, call_data);
                assert_eq!(tx0.transaction.value, U256::from(1000u64));

                let tx1 = &d.transactions[1];
                assert_eq!(tx1.transactionId, B256::ZERO);
                assert_eq!(tx1.senderId, "treasury");
                assert_eq!(tx1.sender, Address::ZERO);
                assert_eq!(tx1.returnData, Bytes::new());
                assert_eq!(tx1.transaction.to, Address::ZERO);
                assert_eq!(tx1.transaction.data, Bytes::new());
                assert_eq!(tx1.transaction.value, U256::ZERO);
            }
            other => panic!("expected TransactionSimulated, got {other:?}"),
        },
        other => panic!("expected Treb event, got {other:?}"),
    }
}

#[test]
fn roundtrip_deployment_collision() {
    let existing = address!("Cf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9");
    let salt = b256!("abcdef0000000000000000000000000000000000000000000000000000000001");

    let event = DeploymentCollision {
        existingContract: existing,
        deployment: DeploymentDetails {
            artifact: "Token".to_string(),
            label: "token-v2".to_string(),
            entropy: "collision-entropy".to_string(),
            salt,
            bytecodeHash: b256!("9999999999999999999999999999999999999999999999999999999999999999"),
            initCodeHash: b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaab"),
            constructorArgs: Bytes::from(vec![0xff]),
            createStrategy: "create2".to_string(),
        },
    };

    let log = make_log(Address::ZERO, event.encode_log_data());
    let parsed = decode_events(&[log]);
    assert_eq!(parsed.len(), 1);

    match &parsed[0] {
        ParsedEvent::Treb(boxed) => match boxed.as_ref() {
            TrebEvent::DeploymentCollision(d) => {
                assert_eq!(d.existingContract, existing);
                assert_eq!(d.deployment.artifact, "Token");
                assert_eq!(d.deployment.label, "token-v2");
                assert_eq!(d.deployment.entropy, "collision-entropy");
                assert_eq!(d.deployment.salt, salt);
                assert_eq!(d.deployment.bytecodeHash, b256!("9999999999999999999999999999999999999999999999999999999999999999"));
                assert_eq!(d.deployment.initCodeHash, b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaab"));
                assert_eq!(d.deployment.constructorArgs, Bytes::from(vec![0xff]));
                assert_eq!(d.deployment.createStrategy, "create2");
            }
            other => panic!("expected DeploymentCollision, got {other:?}"),
        },
        other => panic!("expected Treb event, got {other:?}"),
    }
}

#[test]
fn roundtrip_contract_creation_with_salt() {
    let new_contract = address!("5FbDB2315678afecb367f032d93F642f64180aa3");
    let salt = b256!("1111111111111111111111111111111111111111111111111111111111111111");

    let event = ContractCreation_0 { newContract: new_contract, salt };
    let log = make_log(Address::ZERO, event.encode_log_data());
    let parsed = decode_events(&[log]);
    assert_eq!(parsed.len(), 1);

    match &parsed[0] {
        ParsedEvent::CreateX(CreateXEvent::ContractCreationWithSalt(d)) => {
            assert_eq!(d.newContract, new_contract);
            assert_eq!(d.salt, salt);
        }
        other => panic!("expected ContractCreationWithSalt, got {other:?}"),
    }
}

#[test]
fn roundtrip_upgraded() {
    let proxy_addr = address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512");
    let impl_addr = address!("9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0");

    let event = Upgraded { implementation: impl_addr };
    let log = make_log(proxy_addr, event.encode_log_data());
    let parsed = decode_events(&[log]);
    assert_eq!(parsed.len(), 1);

    match &parsed[0] {
        ParsedEvent::Proxy(ProxyEvent::Upgraded { proxy_address, implementation }) => {
            assert_eq!(*proxy_address, proxy_addr);
            assert_eq!(*implementation, impl_addr);
        }
        other => panic!("expected Upgraded proxy event, got {other:?}"),
    }
}

#[test]
fn roundtrip_all_events_in_single_batch() {
    let deployer = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    let location = address!("5FbDB2315678afecb367f032d93F642f64180aa3");
    let proxy_addr = address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512");
    let impl_addr = address!("9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0");
    let collision_addr = address!("Cf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9");
    let salt = b256!("1111111111111111111111111111111111111111111111111111111111111111");

    // 1. ContractDeployed
    let deployed = ContractDeployed {
        deployer,
        location,
        transactionId: B256::ZERO,
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
    let log1 = make_log(Address::ZERO, deployed.encode_log_data());

    // 2. TransactionSimulated
    let simulated = TransactionSimulated {
        transactions: vec![SimulatedTransaction {
            transactionId: B256::ZERO,
            senderId: "deployer".to_string(),
            sender: deployer,
            returnData: Bytes::new(),
            transaction: Transaction {
                to: location,
                data: Bytes::from(vec![0xaa]),
                value: U256::ZERO,
            },
        }],
    };
    let log2 = make_log(Address::ZERO, simulated.encode_log_data());

    // 3. DeploymentCollision
    let collision = DeploymentCollision {
        existingContract: collision_addr,
        deployment: DeploymentDetails {
            artifact: "Token".to_string(),
            label: "token".to_string(),
            entropy: String::new(),
            salt,
            bytecodeHash: B256::ZERO,
            initCodeHash: B256::ZERO,
            constructorArgs: Bytes::new(),
            createStrategy: "create2".to_string(),
        },
    };
    let log3 = make_log(Address::ZERO, collision.encode_log_data());

    // 4. ContractCreation_0 (with salt)
    let creation = ContractCreation_0 { newContract: location, salt };
    let log4 = make_log(Address::ZERO, creation.encode_log_data());

    // 5. Upgraded
    let upgraded = Upgraded { implementation: impl_addr };
    let log5 = make_log(proxy_addr, upgraded.encode_log_data());

    let parsed = decode_events(&[log1, log2, log3, log4, log5]);
    assert_eq!(parsed.len(), 5);

    assert!(matches!(&parsed[0], ParsedEvent::Treb(b) if matches!(b.as_ref(), TrebEvent::ContractDeployed(_))));
    assert!(matches!(&parsed[1], ParsedEvent::Treb(b) if matches!(b.as_ref(), TrebEvent::TransactionSimulated(_))));
    assert!(matches!(&parsed[2], ParsedEvent::Treb(b) if matches!(b.as_ref(), TrebEvent::DeploymentCollision(_))));
    assert!(matches!(&parsed[3], ParsedEvent::CreateX(CreateXEvent::ContractCreationWithSalt(_))));
    assert!(matches!(&parsed[4], ParsedEvent::Proxy(ProxyEvent::Upgraded { .. })));
}
