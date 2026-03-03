//! Deployment recording pipeline.
//!
//! Orchestrates the end-to-end flow from forge script execution through event
//! decoding, deployment hydration, duplicate detection, and registry recording.
//! All pipeline types and the [`RunPipeline`] orchestrator live here.

mod duplicates;
mod hydration;
mod orchestrator;
mod types;

pub use duplicates::{
    check_duplicate, resolve_duplicates, ConflictType, DuplicateConflict, DuplicateStrategy,
    ResolvedDuplicates,
};
pub use hydration::{
    generate_deployment_id, hydrate_deployment, hydrate_safe_transactions, hydrate_transactions,
    populate_safe_context,
};
pub use orchestrator::RunPipeline;
pub use types::{
    PipelineConfig, PipelineContext, PipelineResult, RecordedDeployment, RecordedTransaction,
    SkippedDeployment,
};

use std::process::Command;

/// Resolve the current git short commit hash.
///
/// Returns the 7-character short hash from `git rev-parse --short HEAD`,
/// or an empty string if the command fails (e.g. not in a git repo).
pub fn resolve_git_commit() -> String {
    Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_config_default_construction() {
        let config = PipelineConfig::default();
        assert!(config.script_path.is_empty());
        assert!(!config.dry_run);
        assert!(config.namespace.is_empty());
        assert_eq!(config.chain_id, 0);
        assert_eq!(config.script_sig, "run()");
        assert!(config.script_args.is_empty());
        assert!(config.env_vars.is_empty());
    }

    #[test]
    fn resolve_git_commit_returns_non_empty_in_repo() {
        let commit = resolve_git_commit();
        assert!(
            !commit.is_empty(),
            "resolve_git_commit() should return a non-empty string in a git repo"
        );
        // Short hash is typically 7 characters
        assert!(
            commit.len() >= 7,
            "git short hash should be at least 7 characters, got: {commit}"
        );
    }
}
