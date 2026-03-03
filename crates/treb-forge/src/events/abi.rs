//! ABI definitions for treb event types using alloy's `sol!` macro.
//!
//! Defines type-safe Rust bindings for:
//! - `ITrebEvents` — treb's custom deployment and transaction events
//! - `ICreateX` — CreateX factory contract events
//! - `ProxyEvents` — ERC-1967 proxy standard events

use alloy_sol_types::sol;

sol! {
    /// Details about a deployed contract, passed as a parameter to deployment events.
    struct DeploymentDetails {
        string artifact;
        string label;
        string entropy;
        bytes32 salt;
        bytes32 bytecodeHash;
        bytes32 initCodeHash;
        bytes constructorArgs;
        string createStrategy;
    }

    /// A simulated transaction with its metadata.
    struct SimulatedTransaction {
        bytes32 transactionId;
        string senderId;
        address sender;
        bytes returnData;
        Transaction transaction;
    }

    /// A transaction's core fields (target, calldata, value).
    struct Transaction {
        address to;
        bytes data;
        uint256 value;
    }

    /// Emitted when a transaction is simulated during script execution.
    event TransactionSimulated(
        SimulatedTransaction[] transactions
    );

    /// Emitted when a contract is deployed via treb's deployment framework.
    event ContractDeployed(
        address indexed deployer,
        address indexed location,
        bytes32 indexed transactionId,
        DeploymentDetails deployment
    );

    /// Emitted when a Safe multisig transaction is queued.
    event SafeTransactionQueued(
        bytes32 indexed safeTxHash,
        address indexed safe,
        address indexed proposer,
        bytes32[] transactionIds
    );

    /// Emitted when a Safe multisig transaction is executed.
    event SafeTransactionExecuted(
        bytes32 indexed safeTxHash,
        address indexed safe,
        address indexed executor,
        bytes32[] transactionIds
    );

    /// Emitted when a deployment would collide with an existing contract.
    event DeploymentCollision(
        address indexed existingContract,
        DeploymentDetails deployment
    );

    /// Emitted when a Governor proposal is created from a deployment script.
    event GovernorProposalCreated(
        uint256 indexed proposalId,
        address indexed governor,
        address indexed proposer,
        bytes32[] transactionIds
    );
}

sol! {
    /// Emitted by CreateX on contract creation with a salt (CREATE2).
    #[sol(rpc)]
    event ContractCreation(address indexed newContract, bytes32 indexed salt);
    /// Emitted by CreateX on contract creation without a salt.
    #[sol(rpc)]
    event ContractCreation(address indexed newContract);
    /// Emitted by CreateX on CREATE3 proxy contract creation.
    #[sol(rpc)]
    event Create3ProxyContractCreation(address indexed newContract, bytes32 indexed salt);
}

sol! {
    /// Emitted when a proxy's implementation is upgraded.
    event Upgraded(address indexed implementation);

    /// Emitted when a transparent proxy's admin is changed.
    event AdminChanged(address previousAdmin, address newAdmin);

    /// Emitted when a beacon proxy's beacon is upgraded.
    event BeaconUpgraded(address indexed beacon);
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, B256, Bytes, U256, address, b256};
    use alloy_sol_types::SolEvent;

    #[test]
    fn construct_deployment_details_and_access_fields() {
        let details = DeploymentDetails {
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
        };

        assert_eq!(details.artifact, "Counter");
        assert_eq!(details.label, "counter-v1");
        assert_eq!(details.entropy, "abc123");
        assert_eq!(details.createStrategy, "create2");
        assert_eq!(details.constructorArgs, Bytes::from(vec![0x01, 0x02, 0x03]));
    }

    #[test]
    fn construct_simulated_transaction_and_access_fields() {
        let tx = SimulatedTransaction {
            transactionId: B256::ZERO,
            senderId: "deployer".to_string(),
            sender: Address::ZERO,
            returnData: Bytes::new(),
            transaction: Transaction {
                to: address!("0000000000000000000000000000000000000001"),
                data: Bytes::from(vec![0xaa, 0xbb]),
                value: U256::from(1000u64),
            },
        };

        assert_eq!(tx.senderId, "deployer");
        assert_eq!(
            tx.transaction.to,
            address!("0000000000000000000000000000000000000001")
        );
        assert_eq!(tx.transaction.value, U256::from(1000u64));
    }

    #[test]
    fn contract_deployed_event_has_signature_hash() {
        // Verify that the sol! macro generated a valid SIGNATURE_HASH
        let hash = ContractDeployed::SIGNATURE_HASH;
        assert_ne!(hash, B256::ZERO, "SIGNATURE_HASH should not be zero");
    }

    #[test]
    fn transaction_simulated_event_has_signature_hash() {
        let hash = TransactionSimulated::SIGNATURE_HASH;
        assert_ne!(hash, B256::ZERO);
    }

    #[test]
    fn safe_transaction_queued_event_has_signature_hash() {
        let hash = SafeTransactionQueued::SIGNATURE_HASH;
        assert_ne!(hash, B256::ZERO);
    }

    #[test]
    fn safe_transaction_executed_event_has_signature_hash() {
        let hash = SafeTransactionExecuted::SIGNATURE_HASH;
        assert_ne!(hash, B256::ZERO);
    }

    #[test]
    fn deployment_collision_event_has_signature_hash() {
        let hash = DeploymentCollision::SIGNATURE_HASH;
        assert_ne!(hash, B256::ZERO);
    }

    #[test]
    fn governor_proposal_created_event_has_signature_hash() {
        let hash = GovernorProposalCreated::SIGNATURE_HASH;
        assert_ne!(hash, B256::ZERO);
    }

    #[test]
    fn createx_contract_creation_with_salt_has_signature_hash() {
        let hash = ContractCreation_0::SIGNATURE_HASH;
        assert_ne!(hash, B256::ZERO);
    }

    #[test]
    fn createx_contract_creation_without_salt_has_signature_hash() {
        let hash = ContractCreation_1::SIGNATURE_HASH;
        assert_ne!(hash, B256::ZERO);
    }

    #[test]
    fn createx_create3_proxy_has_signature_hash() {
        let hash = Create3ProxyContractCreation::SIGNATURE_HASH;
        assert_ne!(hash, B256::ZERO);
    }

    #[test]
    fn proxy_upgraded_event_has_signature_hash() {
        let hash = Upgraded::SIGNATURE_HASH;
        assert_ne!(hash, B256::ZERO);
    }

    #[test]
    fn proxy_admin_changed_event_has_signature_hash() {
        let hash = AdminChanged::SIGNATURE_HASH;
        assert_ne!(hash, B256::ZERO);
    }

    #[test]
    fn proxy_beacon_upgraded_event_has_signature_hash() {
        let hash = BeaconUpgraded::SIGNATURE_HASH;
        assert_ne!(hash, B256::ZERO);
    }

    #[test]
    fn all_signature_hashes_are_unique() {
        let hashes = vec![
            ContractDeployed::SIGNATURE_HASH,
            TransactionSimulated::SIGNATURE_HASH,
            SafeTransactionQueued::SIGNATURE_HASH,
            SafeTransactionExecuted::SIGNATURE_HASH,
            DeploymentCollision::SIGNATURE_HASH,
            GovernorProposalCreated::SIGNATURE_HASH,
            ContractCreation_0::SIGNATURE_HASH,
            ContractCreation_1::SIGNATURE_HASH,
            Create3ProxyContractCreation::SIGNATURE_HASH,
            Upgraded::SIGNATURE_HASH,
            AdminChanged::SIGNATURE_HASH,
            BeaconUpgraded::SIGNATURE_HASH,
        ];

        // Verify all hashes are unique
        for (i, h1) in hashes.iter().enumerate() {
            for (j, h2) in hashes.iter().enumerate() {
                if i != j {
                    assert_ne!(h1, h2, "Signature hashes at index {i} and {j} should differ");
                }
            }
        }
    }
}
