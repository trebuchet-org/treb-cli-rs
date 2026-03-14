//! Type-safe Rust bindings for treb Solidity interfaces.
//!
//! Generated via alloy `sol!` macro from the canonical Solidity definitions
//! in `lib/treb-sol`. Uses inline definitions as a fallback because the
//! Solidity files contain `import` statements that the `sol!` macro cannot
//! resolve.
//!
//! Covers three interface groups:
//! - **ITrebEvents** — treb deployment and transaction events
//! - **ICreateX** — CreateX factory contract events
//! - **ProxyEvents** — ERC-1967 proxy standard events

use alloy_sol_types::sol;

// ---------------------------------------------------------------------------
// ITrebEvents — based on lib/treb-sol/src/internal/ITrebEvents.sol
// ---------------------------------------------------------------------------
sol! {
    #[derive(Debug)]
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

    #[derive(Debug)]
    struct SimulatedTransaction {
        bytes32 transactionId;
        bytes32 senderId;
        address sender;
        bytes returnData;
        Transaction transaction;
        uint256 gasUsed;
    }

    #[derive(Debug)]
    struct Transaction {
        address to;
        bytes data;
        uint256 value;
    }

    #[derive(Debug)]
    event TransactionSimulated(
        SimulatedTransaction simulatedTx
    );

    #[derive(Debug)]
    event ContractDeployed(
        address indexed deployer,
        address indexed location,
        bytes32 indexed transactionId,
        DeploymentDetails deployment
    );

    #[derive(Debug)]
    event SafeTransactionQueued(
        bytes32 indexed safeTxHash,
        address indexed safe,
        address indexed proposer,
        bytes32[] transactionIds
    );

    #[derive(Debug)]
    event SafeTransactionExecuted(
        bytes32 indexed safeTxHash,
        address indexed safe,
        address indexed executor,
        bytes32[] transactionIds
    );

    #[derive(Debug)]
    event DeploymentCollision(
        address indexed existingContract,
        DeploymentDetails deployment
    );

    #[derive(Debug)]
    event GovernorProposalCreated(
        uint256 indexed proposalId,
        address indexed governor,
        address indexed proposer,
        bytes32[] transactionIds
    );
}

// ---------------------------------------------------------------------------
// ICreateX — based on lib/treb-sol/lib/createx-forge/script/ICreateX.sol
// ---------------------------------------------------------------------------
sol! {
    #[derive(Debug)]
    #[sol(rpc)]
    event ContractCreation(address indexed newContract, bytes32 indexed salt);

    #[derive(Debug)]
    #[sol(rpc)]
    event ContractCreation(address indexed newContract);

    #[derive(Debug)]
    #[sol(rpc)]
    event Create3ProxyContractCreation(address indexed newContract, bytes32 indexed salt);
}

// ---------------------------------------------------------------------------
// ERC-1967 Proxy Events
// ---------------------------------------------------------------------------
sol! {
    #[derive(Debug)]
    event Upgraded(address indexed implementation);

    #[derive(Debug)]
    event AdminChanged(address previousAdmin, address newAdmin);

    #[derive(Debug)]
    event BeaconUpgraded(address indexed beacon);
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::B256;
    use alloy_sol_types::SolEvent;

    #[test]
    fn all_event_signature_hashes_are_nonzero() {
        let hashes: Vec<(&str, B256)> = vec![
            ("ContractDeployed", ContractDeployed::SIGNATURE_HASH),
            ("TransactionSimulated", TransactionSimulated::SIGNATURE_HASH),
            ("SafeTransactionQueued", SafeTransactionQueued::SIGNATURE_HASH),
            ("SafeTransactionExecuted", SafeTransactionExecuted::SIGNATURE_HASH),
            ("DeploymentCollision", DeploymentCollision::SIGNATURE_HASH),
            ("GovernorProposalCreated", GovernorProposalCreated::SIGNATURE_HASH),
            ("ContractCreation_0", ContractCreation_0::SIGNATURE_HASH),
            ("ContractCreation_1", ContractCreation_1::SIGNATURE_HASH),
            ("Create3ProxyContractCreation", Create3ProxyContractCreation::SIGNATURE_HASH),
            ("Upgraded", Upgraded::SIGNATURE_HASH),
            ("AdminChanged", AdminChanged::SIGNATURE_HASH),
            ("BeaconUpgraded", BeaconUpgraded::SIGNATURE_HASH),
        ];

        for (name, hash) in &hashes {
            assert_ne!(*hash, B256::ZERO, "SIGNATURE_HASH for {name} should not be zero");
        }
    }

    #[test]
    fn all_event_signature_hashes_are_unique() {
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

        for (i, h1) in hashes.iter().enumerate() {
            for (j, h2) in hashes.iter().enumerate() {
                if i != j {
                    assert_ne!(h1, h2, "Signature hashes at index {i} and {j} should differ");
                }
            }
        }
    }
}
