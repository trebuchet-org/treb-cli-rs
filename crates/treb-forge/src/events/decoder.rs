//! Event log decoder for treb deployment scripts.
//!
//! Decodes raw EVM `Log` entries against known event signatures into typed
//! Rust enums. Unrecognized logs produce `ParsedEvent::Unknown`; malformed
//! known events are logged as warnings and skipped.

use alloy_primitives::{Address, Log};
use alloy_sol_types::SolEvent;

use crate::events::abi::{
    AdminChanged, BeaconUpgraded, ContractCreation_0, ContractCreation_1, ContractDeployed,
    Create3ProxyContractCreation, DeploymentCollision, GovernorProposalCreated,
    SafeTransactionExecuted, SafeTransactionQueued, TransactionSimulated, Upgraded,
};

/// A decoded treb-specific event.
#[derive(Debug, Clone)]
pub enum TrebEvent {
    TransactionSimulated(TransactionSimulated),
    ContractDeployed(ContractDeployed),
    SafeTransactionQueued(SafeTransactionQueued),
    SafeTransactionExecuted(SafeTransactionExecuted),
    DeploymentCollision(DeploymentCollision),
    GovernorProposalCreated(GovernorProposalCreated),
}

/// A decoded CreateX factory event.
#[derive(Debug, Clone)]
pub enum CreateXEvent {
    /// ContractCreation with salt (CREATE2).
    ContractCreationWithSalt(ContractCreation_0),
    /// ContractCreation without salt.
    ContractCreationWithoutSalt(ContractCreation_1),
    /// CREATE3 proxy contract creation.
    Create3ProxyContractCreation(Create3ProxyContractCreation),
}

/// A decoded ERC-1967 proxy event, including the emitting contract address.
#[derive(Debug, Clone)]
pub enum ProxyEvent {
    Upgraded { proxy_address: Address, implementation: Address },
    AdminChanged { proxy_address: Address, previous_admin: Address, new_admin: Address },
    BeaconUpgraded { proxy_address: Address, beacon: Address },
}

/// A parsed event from forge script execution.
#[derive(Debug, Clone)]
pub enum ParsedEvent {
    Treb(Box<TrebEvent>),
    CreateX(CreateXEvent),
    Proxy(ProxyEvent),
    Unknown(Log),
}

/// Decode raw EVM logs into structured [`ParsedEvent`] values.
///
/// Iterates through logs in emission order, matching `topic[0]` against
/// known event signatures. Unrecognized logs produce [`ParsedEvent::Unknown`].
/// Malformed known events are logged as warnings and skipped.
pub fn decode_events(logs: &[Log]) -> Vec<ParsedEvent> {
    logs.iter()
        .filter_map(|log| {
            let sig = match log.topics().first() {
                Some(sig) => *sig,
                None => return Some(ParsedEvent::Unknown(log.clone())),
            };

            // --- ITrebEvents ---
            if sig == TransactionSimulated::SIGNATURE_HASH {
                return decode_or_skip(log, "TransactionSimulated", |l| {
                    TransactionSimulated::decode_log_data(&l.data)
                        .map(|e| ParsedEvent::Treb(Box::new(TrebEvent::TransactionSimulated(e))))
                });
            }
            if sig == ContractDeployed::SIGNATURE_HASH {
                return decode_or_skip(log, "ContractDeployed", |l| {
                    ContractDeployed::decode_log_data(&l.data)
                        .map(|e| ParsedEvent::Treb(Box::new(TrebEvent::ContractDeployed(e))))
                });
            }
            if sig == SafeTransactionQueued::SIGNATURE_HASH {
                return decode_or_skip(log, "SafeTransactionQueued", |l| {
                    SafeTransactionQueued::decode_log_data(&l.data)
                        .map(|e| ParsedEvent::Treb(Box::new(TrebEvent::SafeTransactionQueued(e))))
                });
            }
            if sig == SafeTransactionExecuted::SIGNATURE_HASH {
                return decode_or_skip(log, "SafeTransactionExecuted", |l| {
                    SafeTransactionExecuted::decode_log_data(&l.data)
                        .map(|e| ParsedEvent::Treb(Box::new(TrebEvent::SafeTransactionExecuted(e))))
                });
            }
            if sig == DeploymentCollision::SIGNATURE_HASH {
                return decode_or_skip(log, "DeploymentCollision", |l| {
                    DeploymentCollision::decode_log_data(&l.data)
                        .map(|e| ParsedEvent::Treb(Box::new(TrebEvent::DeploymentCollision(e))))
                });
            }
            if sig == GovernorProposalCreated::SIGNATURE_HASH {
                return decode_or_skip(log, "GovernorProposalCreated", |l| {
                    GovernorProposalCreated::decode_log_data(&l.data)
                        .map(|e| ParsedEvent::Treb(Box::new(TrebEvent::GovernorProposalCreated(e))))
                });
            }

            // --- ICreateX ---
            if sig == ContractCreation_0::SIGNATURE_HASH {
                return decode_or_skip(log, "ContractCreation(with salt)", |l| {
                    ContractCreation_0::decode_log_data(&l.data)
                        .map(|e| ParsedEvent::CreateX(CreateXEvent::ContractCreationWithSalt(e)))
                });
            }
            if sig == ContractCreation_1::SIGNATURE_HASH {
                return decode_or_skip(log, "ContractCreation(without salt)", |l| {
                    ContractCreation_1::decode_log_data(&l.data)
                        .map(|e| ParsedEvent::CreateX(CreateXEvent::ContractCreationWithoutSalt(e)))
                });
            }
            if sig == Create3ProxyContractCreation::SIGNATURE_HASH {
                return decode_or_skip(log, "Create3ProxyContractCreation", |l| {
                    Create3ProxyContractCreation::decode_log_data(&l.data).map(|e| {
                        ParsedEvent::CreateX(CreateXEvent::Create3ProxyContractCreation(e))
                    })
                });
            }

            // --- ERC-1967 Proxy Events ---
            if sig == Upgraded::SIGNATURE_HASH {
                return decode_or_skip(log, "Upgraded", |l| {
                    Upgraded::decode_log_data(&l.data).map(|e| {
                        ParsedEvent::Proxy(ProxyEvent::Upgraded {
                            proxy_address: l.address,
                            implementation: e.implementation,
                        })
                    })
                });
            }
            if sig == AdminChanged::SIGNATURE_HASH {
                return decode_or_skip(log, "AdminChanged", |l| {
                    AdminChanged::decode_log_data(&l.data).map(|e| {
                        ParsedEvent::Proxy(ProxyEvent::AdminChanged {
                            proxy_address: l.address,
                            previous_admin: e.previousAdmin,
                            new_admin: e.newAdmin,
                        })
                    })
                });
            }
            if sig == BeaconUpgraded::SIGNATURE_HASH {
                return decode_or_skip(log, "BeaconUpgraded", |l| {
                    BeaconUpgraded::decode_log_data(&l.data).map(|e| {
                        ParsedEvent::Proxy(ProxyEvent::BeaconUpgraded {
                            proxy_address: l.address,
                            beacon: e.beacon,
                        })
                    })
                });
            }

            Some(ParsedEvent::Unknown(log.clone()))
        })
        .collect()
}

/// Attempt to decode a log; on failure, warn and return `None` (skip).
fn decode_or_skip(
    log: &Log,
    event_name: &str,
    decode: impl FnOnce(&Log) -> Result<ParsedEvent, alloy_sol_types::Error>,
) -> Option<ParsedEvent> {
    match decode(log) {
        Ok(event) => Some(event),
        Err(e) => {
            eprintln!("warning: malformed {event_name} log, skipping: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{B256, Bytes, LogData, U256, address, b256};
    use alloy_sol_types::SolEvent;

    use crate::events::abi::{DeploymentDetails, SimulatedTransaction, Transaction};

    /// Helper: create a raw Log from an event type's encoded log data.
    fn make_log(address: Address, log_data: LogData) -> Log {
        Log::new(address, log_data.topics().to_vec(), log_data.data.clone())
            .expect("valid log data")
    }

    #[test]
    fn decode_contract_deployed_with_all_fields() {
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
            deployment: details.clone(),
        };

        let log_data = event.encode_log_data();
        let log = make_log(Address::ZERO, log_data);

        let parsed = decode_events(&[log]);
        assert_eq!(parsed.len(), 1);

        match &parsed[0] {
            ParsedEvent::Treb(boxed) => match boxed.as_ref() {
                TrebEvent::ContractDeployed(decoded) => {
                    assert_eq!(decoded.deployer, deployer);
                    assert_eq!(decoded.location, location);
                    assert_eq!(decoded.transactionId, tx_id);
                    assert_eq!(decoded.deployment.artifact, "Counter");
                    assert_eq!(decoded.deployment.label, "counter-v1");
                    assert_eq!(decoded.deployment.entropy, "abc123");
                    assert_eq!(decoded.deployment.createStrategy, "create2");
                    assert_eq!(
                        decoded.deployment.constructorArgs,
                        Bytes::from(vec![0x01, 0x02, 0x03])
                    );
                }
                other => panic!("expected ContractDeployed, got {other:?}"),
            },
            other => panic!("expected Treb event, got {other:?}"),
        }
    }

    #[test]
    fn decode_mixed_logs_in_emission_order() {
        let deployer = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        let location = address!("5FbDB2315678afecb367f032d93F642f64180aa3");
        let proxy_addr = address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512");
        let impl_addr = address!("9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0");

        // 1. ContractDeployed event
        let deployed = ContractDeployed {
            deployer,
            location,
            transactionId: B256::ZERO,
            deployment: DeploymentDetails {
                artifact: "Token".to_string(),
                label: "token".to_string(),
                entropy: String::new(),
                salt: B256::ZERO,
                bytecodeHash: B256::ZERO,
                initCodeHash: B256::ZERO,
                constructorArgs: Bytes::new(),
                createStrategy: "create".to_string(),
            },
        };
        let log1 = make_log(Address::ZERO, deployed.encode_log_data());

        // 2. Upgraded proxy event (emitted from proxy_addr)
        let upgraded = Upgraded { implementation: impl_addr };
        let log2 = make_log(proxy_addr, upgraded.encode_log_data());

        // 3. Unknown log (random topic)
        let unknown_topic =
            b256!("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");
        let log3 = Log::new(Address::ZERO, vec![unknown_topic], Bytes::new()).expect("valid log");

        let parsed = decode_events(&[log1, log2, log3]);
        assert_eq!(parsed.len(), 3);

        // Verify order and types
        assert!(
            matches!(&parsed[0], ParsedEvent::Treb(b) if matches!(b.as_ref(), TrebEvent::ContractDeployed(_))),
            "first should be ContractDeployed"
        );

        match &parsed[1] {
            ParsedEvent::Proxy(ProxyEvent::Upgraded { proxy_address, implementation }) => {
                assert_eq!(*proxy_address, proxy_addr);
                assert_eq!(*implementation, impl_addr);
            }
            other => panic!("expected Upgraded proxy event, got {other:?}"),
        }

        assert!(matches!(&parsed[2], ParsedEvent::Unknown(_)), "third should be Unknown");
    }

    #[test]
    fn decode_createx_with_salt() {
        let new_contract = address!("5FbDB2315678afecb367f032d93F642f64180aa3");
        let salt = b256!("1111111111111111111111111111111111111111111111111111111111111111");

        let event = ContractCreation_0 { newContract: new_contract, salt };
        let log = make_log(Address::ZERO, event.encode_log_data());

        let parsed = decode_events(&[log]);
        assert_eq!(parsed.len(), 1);

        match &parsed[0] {
            ParsedEvent::CreateX(CreateXEvent::ContractCreationWithSalt(decoded)) => {
                assert_eq!(decoded.newContract, new_contract);
                assert_eq!(decoded.salt, salt);
            }
            other => panic!("expected ContractCreationWithSalt, got {other:?}"),
        }
    }

    #[test]
    fn decode_admin_changed_includes_proxy_address() {
        let proxy_addr = address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512");
        let prev_admin = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        let new_admin = address!("70997970C51812dc3A010C7d01b50e0d17dc79C8");

        let event = AdminChanged { previousAdmin: prev_admin, newAdmin: new_admin };
        let log = make_log(proxy_addr, event.encode_log_data());

        let parsed = decode_events(&[log]);
        assert_eq!(parsed.len(), 1);

        match &parsed[0] {
            ParsedEvent::Proxy(ProxyEvent::AdminChanged {
                proxy_address,
                previous_admin,
                new_admin: na,
            }) => {
                assert_eq!(*proxy_address, proxy_addr);
                assert_eq!(*previous_admin, prev_admin);
                assert_eq!(*na, new_admin);
            }
            other => panic!("expected AdminChanged, got {other:?}"),
        }
    }

    #[test]
    fn decode_beacon_upgraded_includes_proxy_address() {
        let proxy_addr = address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512");
        let beacon = address!("9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0");

        let event = BeaconUpgraded { beacon };
        let log = make_log(proxy_addr, event.encode_log_data());

        let parsed = decode_events(&[log]);
        assert_eq!(parsed.len(), 1);

        match &parsed[0] {
            ParsedEvent::Proxy(ProxyEvent::BeaconUpgraded { proxy_address, beacon: b }) => {
                assert_eq!(*proxy_address, proxy_addr);
                assert_eq!(*b, beacon);
            }
            other => panic!("expected BeaconUpgraded, got {other:?}"),
        }
    }

    #[test]
    fn empty_logs_returns_empty_vec() {
        let parsed = decode_events(&[]);
        assert!(parsed.is_empty());
    }

    #[test]
    fn log_without_topics_is_unknown() {
        let log = Log::new(Address::ZERO, vec![], Bytes::new()).expect("valid log");
        let parsed = decode_events(&[log]);
        assert_eq!(parsed.len(), 1);
        assert!(matches!(&parsed[0], ParsedEvent::Unknown(_)));
    }

    #[test]
    fn decode_transaction_simulated() {
        let event = TransactionSimulated {
            transactions: vec![SimulatedTransaction {
                transactionId: B256::ZERO,
                senderId: "deployer".to_string(),
                sender: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
                returnData: Bytes::new(),
                transaction: Transaction {
                    to: address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
                    data: Bytes::from(vec![0xaa]),
                    value: U256::from(1000u64),
                },
            }],
        };
        let log = make_log(Address::ZERO, event.encode_log_data());

        let parsed = decode_events(&[log]);
        assert_eq!(parsed.len(), 1);

        match &parsed[0] {
            ParsedEvent::Treb(boxed) => match boxed.as_ref() {
                TrebEvent::TransactionSimulated(decoded) => {
                    assert_eq!(decoded.transactions.len(), 1);
                    assert_eq!(decoded.transactions[0].senderId, "deployer");
                }
                other => panic!("expected TransactionSimulated, got {other:?}"),
            },
            other => panic!("expected Treb event, got {other:?}"),
        }
    }
}
