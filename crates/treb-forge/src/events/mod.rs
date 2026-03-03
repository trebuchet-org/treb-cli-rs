//! Event parsing for treb deployment scripts.
//!
//! Converts raw EVM logs from forge script execution into structured
//! treb domain events. Provides ABI bindings via alloy's `sol!` macro,
//! event decoding, deployment extraction, proxy detection, and natspec parsing.

pub mod abi;

// Re-export ABI types for convenience.
pub use abi::{
    AdminChanged, BeaconUpgraded, ContractCreation_0, ContractCreation_1, ContractDeployed,
    Create3ProxyContractCreation, DeploymentCollision, DeploymentDetails,
    GovernorProposalCreated, SafeTransactionExecuted, SafeTransactionQueued, SimulatedTransaction,
    Transaction, TransactionSimulated, Upgraded,
};
