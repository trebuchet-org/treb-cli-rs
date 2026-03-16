//! Configuration system for treb — layered config parsing, merging,
//! and validation.
//!
//! This crate is the single owner of all configuration logic. Other treb
//! crates depend on `treb-config` for resolved configuration and never
//! parse config files directly.

pub mod env;
pub mod foundry;
pub mod local;
pub mod resolver;
pub mod trebfile;
pub mod trebfile_v1;
pub mod types;
pub mod validation;

// Re-export all config types at the crate root for convenience.
pub use types::{
    AccountConfig, ConfigWarning, ForkConfig, LocalConfig, NamespaceConfigV1, NamespaceRoles,
    ResolvedConfig, ResolvedSenders, SenderConfig, SenderType, TrebConfigFormat, TrebFileConfigV1,
    TrebFileConfigV2,
};

// Re-export local config functions.
pub use local::{load_local_config, save_local_config};

// Re-export treb.toml v2 parser functions.
pub use trebfile::{
    detect_treb_config_format, expand_env_vars, load_treb_config_v2, serialize_treb_config_v2,
};

// Re-export treb.toml v1 parser functions.
pub use trebfile_v1::{convert_v1_to_resolved, load_treb_config_v1};

// Re-export .env loading.
pub use env::load_dotenv;

// Re-export foundry config integration.
pub use foundry::{
    ResolvedRpcEndpoint, RpcOverrideGuard, extract_treb_senders_from_foundry, load_foundry_config,
    override_rpc_endpoint, resolve_rpc_endpoints, rpc_endpoints,
};

// Re-export resolver.
pub use resolver::{ResolveOpts, resolve_config, resolve_namespace_v2};

// Re-export validation.
pub use validation::{validate_config, validate_sender};
