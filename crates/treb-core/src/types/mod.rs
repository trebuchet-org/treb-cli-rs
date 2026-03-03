//! Domain types for treb — deployments, transactions, and related models.

pub mod deployment;
pub mod enums;
pub mod ids;
pub mod transaction;

pub use deployment::{
    ArtifactInfo, Deployment, DeploymentStrategy, ProxyInfo, ProxyUpgrade, VerificationInfo,
    VerifierStatus,
};
pub use enums::{
    DeploymentMethod, DeploymentType, ProposalStatus, TransactionStatus, VerificationStatus,
};
pub use ids::DeploymentId;
pub use transaction::{Operation, SafeContext, Transaction};
