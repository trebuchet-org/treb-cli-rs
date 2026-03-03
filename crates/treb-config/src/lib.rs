//! Configuration system for treb — layered config parsing, merging,
//! and validation.
//!
//! This crate is the single owner of all configuration logic. Other treb
//! crates depend on `treb-config` for resolved configuration and never
//! parse config files directly.

pub mod types;

// Re-export all config types at the crate root for convenience.
pub use types::{
    AccountConfig, ConfigWarning, ForkConfig, LocalConfig, NamespaceConfigV1, NamespaceRoles,
    ResolvedConfig, ResolvedSenders, SenderConfig, SenderType, TrebConfigFormat,
    TrebFileConfigV1, TrebFileConfigV2,
};
