//! Event parsing for treb deployment scripts.
//!
//! Converts raw EVM logs from forge script execution into structured
//! treb domain events. Provides ABI bindings via alloy's `sol!` macro,
//! event decoding, deployment extraction, proxy detection, and natspec parsing.

pub mod abi;
pub mod decoder;
pub mod deployments;
pub mod params;
pub mod proxy;

// Re-export ABI types for convenience.
pub use abi::{
    AdminChanged, BeaconUpgraded, ContractCreation_0, ContractCreation_1, ContractDeployed,
    Create3ProxyContractCreation, DeploymentCollision, DeploymentDetails, GovernorProposalCreated,
    SafeTransactionExecuted, SafeTransactionQueued, SimulatedTransaction, Transaction,
    TransactionSimulated, Upgraded,
};

// Re-export decoder types and functions.
pub use decoder::{CreateXEvent, ParsedEvent, ProxyEvent, TrebEvent, decode_events};

// Re-export deployment extraction types and functions.
pub use deployments::{
    ExtractedCollision, ExtractedDeployment, extract_collisions, extract_deployments,
};

// Re-export proxy relationship types and functions.
pub use proxy::{
    ProxyRelationship, ProxyType, detect_proxy_relationships, link_proxy_to_deployment,
};

// Re-export natspec parameter types and functions.
pub use params::{
    ParameterType, ScriptParameter, parse_custom_env_string, parse_script_parameters,
};
