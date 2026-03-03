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
pub mod version;

// Re-export key public types for convenience.
pub use artifacts::ArtifactIndex;
pub use broadcast::BroadcastReader;
pub use compiler::{CompilationOutput, ProjectCompiler};
pub use console::decode_console_logs;
pub use script::{ExecutionResult, ScriptConfig};
pub use version::ForgeVersion;
