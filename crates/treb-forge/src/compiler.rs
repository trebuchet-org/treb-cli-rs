//! In-process Solidity compilation via foundry's compiler infrastructure.
//!
//! Wraps foundry's project compilation to prevent `std::process::exit(0)` calls
//! and returns errors as `TrebError::Forge`.

// TODO: Implement CompilationOutput struct with output and project_root fields
// TODO: Implement compile_project(config) -> Result<CompilationOutput>
// TODO: Implement compile_files(config, files) -> Result<CompilationOutput>
// TODO: Implement has_compiler_errors() and artifact_ids() accessors

/// Output from an in-process Solidity compilation.
pub struct CompilationOutput {
    // TODO: Add fields (ProjectCompileOutput, PathBuf)
}

/// Wrapper for in-process project compilation.
pub struct ProjectCompiler;
