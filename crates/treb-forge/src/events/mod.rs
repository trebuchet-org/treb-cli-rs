//! Event parsing for treb deployment scripts.
//!
//! Converts raw EVM logs from forge script execution into structured
//! treb domain events. Provides ABI bindings via alloy's `sol!` macro,
//! event decoding, deployment extraction, proxy detection, and natspec parsing.

pub mod abi;
pub mod decoder;
pub mod deployments;

// Re-export ABI types for convenience.
pub use abi::{
    AdminChanged, BeaconUpgraded, ContractCreation_0, ContractCreation_1, ContractDeployed,
    Create3ProxyContractCreation, DeploymentCollision, DeploymentDetails,
    GovernorProposalCreated, SafeTransactionExecuted, SafeTransactionQueued, SimulatedTransaction,
    Transaction, TransactionSimulated, Upgraded,
};

// Re-export decoder types and functions.
pub use decoder::{decode_events, CreateXEvent, ParsedEvent, ProxyEvent, TrebEvent};

// Re-export deployment extraction types and functions.
pub use deployments::{
    extract_collisions, extract_deployments, ExtractedCollision, ExtractedDeployment,
};
