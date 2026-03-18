//! In-process Foundry integration for treb.
//!
//! This crate bridges treb's configuration/registry system with foundry's
//! compilation and script execution pipeline. All forge functionality is
//! accessed through Rust crate APIs with no subprocess calls to `forge`.

pub mod anvil;
pub mod artifacts;
pub mod broadcast;
pub mod compiler;
pub mod console;
pub mod createx;
pub mod events;
pub mod fund;
pub mod governor;
pub mod pipeline;
pub mod script;
pub mod sender;
pub mod sender_config;
pub mod version;

// Re-export key public types for convenience.
pub use anvil::{
    AnvilConfig, AnvilInstance, BackgroundAnvil, BackgroundAnvilConfig, find_available_port,
    poll_anvil_health, spawn_background_anvil, stop_background_anvil,
};
pub use artifacts::{ArtifactIndex, ArtifactMatch};
pub use broadcast::{
    BroadcastData, BroadcastTransaction, read_all_broadcasts, read_latest_broadcast,
};
pub use compiler::{CompilationOutput, compile_files, compile_project};
pub use console::decode_console_logs;
pub use createx::{CREATEX_ADDRESS, createx_deployed_bytecode, deploy_createx, verify_createx};
pub use pipeline::{
    ConflictType, DuplicateConflict, DuplicateStrategy, PipelineConfig, PipelineContext,
    PipelineResult, RecordedDeployment, RecordedTransaction, ResolvedDuplicates, RunPipeline,
    SkippedDeployment, check_duplicate, generate_deployment_id, hydrate_deployment,
    hydrate_safe_transactions, hydrate_transactions, resolve_duplicates, resolve_git_commit,
};
pub use script::{
    ExecutionResult, ScriptConfig, build_script_config, build_script_config_with_senders,
    execute_script,
};
pub use sender::{
    ResolvedSender, SenderCategory, default_test_signers, extract_signing_key, in_memory_signer,
    resolve_all_senders, resolve_sender,
};
pub use fund::fund_senders_on_fork;
pub use governor::{is_terminal, query_proposal_state};
pub use sender_config::encode_sender_configs;
pub use version::{ForgeVersion, detect_forge_version};

// Re-export foundry-linking for downstream use (Phase 8 deployment recording pipeline).
pub use foundry_linking;
