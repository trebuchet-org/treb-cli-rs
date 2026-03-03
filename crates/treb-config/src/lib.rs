//! Configuration system for treb — layered config parsing, merging,
//! and validation.
//!
//! This crate is the single owner of all configuration logic. Other treb
//! crates depend on `treb-config` for resolved configuration and never
//! parse config files directly.

pub mod local;
pub mod trebfile;
pub mod trebfile_v1;
pub mod types;

// Re-export all config types at the crate root for convenience.
pub use types::{
    AccountConfig, ConfigWarning, ForkConfig, LocalConfig, NamespaceConfigV1, NamespaceRoles,
    ResolvedConfig, ResolvedSenders, SenderConfig, SenderType, TrebConfigFormat,
    TrebFileConfigV1, TrebFileConfigV2,
};

// Re-export local config functions.
pub use local::{load_local_config, save_local_config};

// Re-export treb.toml v2 parser functions.
pub use trebfile::{detect_treb_config_format, expand_env_vars, load_treb_config_v2};

// Re-export treb.toml v1 parser functions.
pub use trebfile_v1::{convert_v1_to_resolved, load_treb_config_v1};
