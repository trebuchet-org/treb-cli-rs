//! Deployment extraction from decoded events.
//!
//! Converts [`ParsedEvent`] sequences into [`ExtractedDeployment`] and
//! [`ExtractedCollision`] structs for downstream registry recording.

use alloy_primitives::{Address, B256, Bytes};
use treb_core::types::enums::DeploymentMethod;

use crate::{
    artifacts::{ArtifactIndex, ArtifactMatch},
    events::decoder::{ParsedEvent, TrebEvent},
};

/// A deployment extracted from a [`ContractDeployed`] event.
#[derive(Debug)]
pub struct ExtractedDeployment {
    /// The deployed contract address.
    pub address: Address,
    /// The account that initiated the deployment.
    pub deployer: Address,
    /// The transaction identifier from the deployment script.
    pub transaction_id: B256,
    /// The contract artifact name (e.g., "Counter").
    pub contract_name: String,
    /// A human-readable label for this deployment.
    pub label: String,
    /// The creation strategy used (CREATE, CREATE2, CREATE3).
    pub strategy: DeploymentMethod,
    /// The salt used for deterministic deployment (zero if none).
    pub salt: B256,
    /// Hash of the runtime bytecode.
    pub bytecode_hash: B256,
    /// Hash of the init (creation) code.
    pub init_code_hash: B256,
    /// ABI-encoded constructor arguments.
    pub constructor_args: Bytes,
    /// Optional entropy string used in salt computation.
    pub entropy: String,
    /// Matched compilation artifact, if an [`ArtifactIndex`] was provided.
    pub artifact_match: Option<ArtifactMatch>,
}

/// A collision extracted from a [`DeploymentCollision`] event.
#[derive(Debug)]
pub struct ExtractedCollision {
    /// The address where a contract already exists.
    pub existing_address: Address,
    /// The artifact name of the contract that would have been deployed.
    pub contract_name: String,
    /// The label for the attempted deployment.
    pub label: String,
    /// The strategy that was attempted.
    pub strategy: DeploymentMethod,
    /// The salt from the attempted deployment.
    pub salt: B256,
    /// The bytecode hash of the attempted deployment.
    pub bytecode_hash: B256,
    /// The init code hash of the attempted deployment.
    pub init_code_hash: B256,
}

/// Parse a strategy string from the event into a [`DeploymentMethod`].
///
/// Recognised values (case-insensitive): `create`, `create2`, `create3`.
/// Unknown values default to [`DeploymentMethod::Create`].
fn parse_strategy(s: &str) -> DeploymentMethod {
    match s.to_uppercase().as_str() {
        "CREATE" => DeploymentMethod::Create,
        "CREATE2" => DeploymentMethod::Create2,
        "CREATE3" => DeploymentMethod::Create3,
        _ => DeploymentMethod::Create,
    }
}

/// Extract deployments from decoded events.
///
/// Iterates through `events` and converts every [`ContractDeployed`] into an
/// [`ExtractedDeployment`]. When an [`ArtifactIndex`] is provided, each
/// deployment's `contract_name` is looked up for an artifact match.
pub fn extract_deployments(
    events: &[ParsedEvent],
    artifacts: Option<&ArtifactIndex>,
) -> Vec<ExtractedDeployment> {
    events
        .iter()
        .filter_map(|event| {
            if let ParsedEvent::Treb(boxed) = event {
                if let TrebEvent::ContractDeployed(deployed) = boxed.as_ref() {
                    let artifact_match = artifacts.and_then(|idx| {
                        idx.find_by_name(&deployed.deployment.artifact).ok().flatten()
                    });

                    return Some(ExtractedDeployment {
                        address: deployed.location,
                        deployer: deployed.deployer,
                        transaction_id: deployed.transactionId,
                        contract_name: deployed.deployment.artifact.clone(),
                        label: deployed.deployment.label.clone(),
                        strategy: parse_strategy(&deployed.deployment.createStrategy),
                        salt: deployed.deployment.salt,
                        bytecode_hash: deployed.deployment.bytecodeHash,
                        init_code_hash: deployed.deployment.initCodeHash,
                        constructor_args: deployed.deployment.constructorArgs.clone(),
                        entropy: deployed.deployment.entropy.clone(),
                        artifact_match,
                    });
                }
            }
            None
        })
        .collect()
}

/// Extract collision information from decoded events.
///
/// Iterates through `events` and converts every [`DeploymentCollision`] into
/// an [`ExtractedCollision`].
pub fn extract_collisions(events: &[ParsedEvent]) -> Vec<ExtractedCollision> {
    events
        .iter()
        .filter_map(|event| {
            if let ParsedEvent::Treb(boxed) = event {
                if let TrebEvent::DeploymentCollision(collision) = boxed.as_ref() {
                    return Some(ExtractedCollision {
                        existing_address: collision.existingContract,
                        contract_name: collision.deployment.artifact.clone(),
                        label: collision.deployment.label.clone(),
                        strategy: parse_strategy(&collision.deployment.createStrategy),
                        salt: collision.deployment.salt,
                        bytecode_hash: collision.deployment.bytecodeHash,
                        init_code_hash: collision.deployment.initCodeHash,
                    });
                }
            }
            None
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{LogData, address, b256};
    use alloy_sol_types::SolEvent;

    use crate::events::{
        abi::{ContractDeployed, DeploymentDetails},
        decoder::decode_events,
    };

    /// Helper: create a raw Log from an event type's encoded log data.
    fn make_log(address: Address, log_data: LogData) -> alloy_primitives::Log {
        alloy_primitives::Log::new(address, log_data.topics().to_vec(), log_data.data.clone())
            .expect("valid log data")
    }

    fn sample_deployed_event() -> ContractDeployed {
        ContractDeployed {
            deployer: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
            location: address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
            transactionId: b256!(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            ),
            deployment: DeploymentDetails {
                artifact: "Counter".to_string(),
                label: "counter-v1".to_string(),
                entropy: "abc123".to_string(),
                salt: b256!("0000000000000000000000000000000000000000000000000000000000000001"),
                bytecodeHash: b256!(
                    "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
                ),
                initCodeHash: b256!(
                    "abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd"
                ),
                constructorArgs: Bytes::from(vec![0x01, 0x02, 0x03]),
                createStrategy: "create2".to_string(),
            },
        }
    }

    #[test]
    fn extract_deployment_with_all_fields() {
        let event = sample_deployed_event();
        let log = make_log(Address::ZERO, event.encode_log_data());
        let parsed = decode_events(&[log]);

        let deployments = extract_deployments(&parsed, None);
        assert_eq!(deployments.len(), 1);

        let d = &deployments[0];
        assert_eq!(d.address, address!("5FbDB2315678afecb367f032d93F642f64180aa3"));
        assert_eq!(d.deployer, address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
        assert_eq!(
            d.transaction_id,
            b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(d.contract_name, "Counter");
        assert_eq!(d.label, "counter-v1");
        assert_eq!(d.strategy, DeploymentMethod::Create2);
        assert_eq!(
            d.salt,
            b256!("0000000000000000000000000000000000000000000000000000000000000001")
        );
        assert_eq!(
            d.bytecode_hash,
            b256!("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef")
        );
        assert_eq!(
            d.init_code_hash,
            b256!("abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd")
        );
        assert_eq!(d.constructor_args, Bytes::from(vec![0x01, 0x02, 0x03]));
        assert_eq!(d.entropy, "abc123");
        assert!(d.artifact_match.is_none());
    }

    #[test]
    fn extract_collision() {
        let collision_event = crate::events::abi::DeploymentCollision {
            existingContract: address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512"),
            deployment: DeploymentDetails {
                artifact: "Token".to_string(),
                label: "token-v1".to_string(),
                entropy: String::new(),
                salt: b256!("2222222222222222222222222222222222222222222222222222222222222222"),
                bytecodeHash: b256!(
                    "3333333333333333333333333333333333333333333333333333333333333333"
                ),
                initCodeHash: b256!(
                    "4444444444444444444444444444444444444444444444444444444444444444"
                ),
                constructorArgs: Bytes::new(),
                createStrategy: "create2".to_string(),
            },
        };
        let log = make_log(Address::ZERO, collision_event.encode_log_data());
        let parsed = decode_events(&[log]);

        let collisions = extract_collisions(&parsed);
        assert_eq!(collisions.len(), 1);

        let c = &collisions[0];
        assert_eq!(c.existing_address, address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512"));
        assert_eq!(c.contract_name, "Token");
        assert_eq!(c.label, "token-v1");
        assert_eq!(c.strategy, DeploymentMethod::Create2);
        assert_eq!(
            c.salt,
            b256!("2222222222222222222222222222222222222222222222222222222222222222")
        );
    }

    #[test]
    fn unknown_strategy_defaults_to_create() {
        assert_eq!(parse_strategy("create"), DeploymentMethod::Create);
        assert_eq!(parse_strategy("create2"), DeploymentMethod::Create2);
        assert_eq!(parse_strategy("create3"), DeploymentMethod::Create3);
        assert_eq!(parse_strategy("CREATE"), DeploymentMethod::Create);
        assert_eq!(parse_strategy("CREATE2"), DeploymentMethod::Create2);
        assert_eq!(parse_strategy("CREATE3"), DeploymentMethod::Create3);
        assert_eq!(parse_strategy("unknown"), DeploymentMethod::Create);
        assert_eq!(parse_strategy(""), DeploymentMethod::Create);
        assert_eq!(parse_strategy("something_else"), DeploymentMethod::Create);
    }

    #[test]
    fn artifact_match_none_when_index_is_none() {
        let event = sample_deployed_event();
        let log = make_log(Address::ZERO, event.encode_log_data());
        let parsed = decode_events(&[log]);

        let deployments = extract_deployments(&parsed, None);
        assert_eq!(deployments.len(), 1);
        assert!(deployments[0].artifact_match.is_none());
    }

    #[test]
    fn extract_from_mixed_events_only_picks_deployments() {
        let deployed = sample_deployed_event();
        let log1 = make_log(Address::ZERO, deployed.encode_log_data());

        // An Upgraded proxy event — should be ignored by extract_deployments
        let upgraded = crate::events::abi::Upgraded {
            implementation: address!("9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0"),
        };
        let log2 = make_log(
            address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512"),
            upgraded.encode_log_data(),
        );

        // An unknown log
        let unknown_topic =
            b256!("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");
        let log3 = alloy_primitives::Log::new(Address::ZERO, vec![unknown_topic], Bytes::new())
            .expect("valid log");

        let parsed = decode_events(&[log1, log2, log3]);
        assert_eq!(parsed.len(), 3);

        let deployments = extract_deployments(&parsed, None);
        assert_eq!(deployments.len(), 1);
        assert_eq!(deployments[0].contract_name, "Counter");

        let collisions = extract_collisions(&parsed);
        assert!(collisions.is_empty());
    }

    #[test]
    fn extract_multiple_deployments_preserves_order() {
        let event1 = ContractDeployed {
            deployer: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
            location: address!("5FbDB2315678afecb367f032d93F642f64180aa3"),
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
        let event2 = ContractDeployed {
            deployer: address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
            location: address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512"),
            transactionId: B256::ZERO,
            deployment: DeploymentDetails {
                artifact: "Token".to_string(),
                label: "token".to_string(),
                entropy: String::new(),
                salt: B256::ZERO,
                bytecodeHash: B256::ZERO,
                initCodeHash: B256::ZERO,
                constructorArgs: Bytes::new(),
                createStrategy: "create3".to_string(),
            },
        };

        let log1 = make_log(Address::ZERO, event1.encode_log_data());
        let log2 = make_log(Address::ZERO, event2.encode_log_data());
        let parsed = decode_events(&[log1, log2]);

        let deployments = extract_deployments(&parsed, None);
        assert_eq!(deployments.len(), 2);
        assert_eq!(deployments[0].contract_name, "Counter");
        assert_eq!(deployments[0].strategy, DeploymentMethod::Create);
        assert_eq!(deployments[1].contract_name, "Token");
        assert_eq!(deployments[1].strategy, DeploymentMethod::Create3);
    }
}
