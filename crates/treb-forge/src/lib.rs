//! In-process Foundry integration for treb.
//!
//! This crate bridges treb's configuration/registry system with foundry's
//! compilation and script execution pipeline. All forge functionality is
//! accessed through Rust crate APIs with no subprocess calls to `forge`.

pub mod artifacts;
pub mod broadcast;
pub mod compiler;
pub mod console;
pub mod script;
pub mod sender;
pub mod version;

// Re-export key public types for convenience.
pub use artifacts::{ArtifactIndex, ArtifactMatch};
pub use broadcast::{read_all_broadcasts, read_latest_broadcast, BroadcastData, BroadcastTransaction};
pub use compiler::{compile_files, compile_project, CompilationOutput};
pub use console::decode_console_logs;
pub use script::{build_script_config, execute_script, ExecutionResult, ScriptConfig};
pub use sender::{
    default_test_signers, in_memory_signer, resolve_all_senders, resolve_sender, ResolvedSender,
};
pub use version::{detect_forge_version, ForgeVersion};
