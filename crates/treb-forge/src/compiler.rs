//! In-process Solidity compilation via foundry's compiler infrastructure.
//!
//! Wraps foundry's project compilation to prevent `std::process::exit(0)` calls
//! and returns errors as `TrebError::Forge`.

use std::path::PathBuf;

use foundry_compilers::ArtifactId;
use foundry_config::Config;
use treb_core::error::TrebError;

/// Output from an in-process Solidity compilation.
pub struct CompilationOutput {
    /// The raw compiler output from foundry.
    output: foundry_compilers::ProjectCompileOutput,
    /// Root directory of the compiled project.
    project_root: PathBuf,
}

impl CompilationOutput {
    /// Returns `true` if the compilation produced any errors.
    pub fn has_compiler_errors(&self) -> bool {
        self.output.has_compiler_errors()
    }

    /// Returns an iterator over all artifact IDs produced by the compilation.
    pub fn artifact_ids(
        &self,
    ) -> impl Iterator<Item = ArtifactId> + '_ {
        self.output.artifact_ids().map(|(id, _)| id)
    }

    /// Returns the project root directory.
    pub fn project_root(&self) -> &PathBuf {
        &self.project_root
    }

    /// Returns a reference to the underlying `ProjectCompileOutput`.
    pub fn output(&self) -> &foundry_compilers::ProjectCompileOutput {
        &self.output
    }
}

/// Compile an entire Solidity project using the given foundry `Config`.
///
/// Validates that the project has input files before compiling.
/// Returns `TrebError::Forge` if no input files are found or compilation fails.
pub fn compile_project(config: &Config) -> treb_core::Result<CompilationOutput> {
    let project = config
        .project()
        .map_err(|e| TrebError::Forge(format!("failed to create project: {e}")))?;

    if !project.paths.has_input_files() {
        return Err(TrebError::Forge(
            "no input files found in project".to_string(),
        ));
    }

    let project_root = project.root().to_path_buf();

    let output = project
        .compile()
        .map_err(|e| TrebError::Forge(format!("compilation failed: {e}")))?;

    Ok(CompilationOutput {
        output,
        project_root,
    })
}

/// Compile specific Solidity files using the given foundry `Config`.
///
/// Returns `TrebError::Forge` if no files are provided or compilation fails.
pub fn compile_files(
    config: &Config,
    files: Vec<PathBuf>,
) -> treb_core::Result<CompilationOutput> {
    if files.is_empty() {
        return Err(TrebError::Forge(
            "no input files provided".to_string(),
        ));
    }

    let project = config
        .project()
        .map_err(|e| TrebError::Forge(format!("failed to create project: {e}")))?;

    let project_root = project.root().to_path_buf();

    let output = project
        .compile_files(files)
        .map_err(|e| TrebError::Forge(format!("compilation failed: {e}")))?;

    Ok(CompilationOutput {
        output,
        project_root,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_project_empty_returns_forge_error() {
        // Create a temp directory with a foundry.toml but no source files.
        let dir = tempfile::tempdir().unwrap();
        let foundry_toml = dir.path().join("foundry.toml");
        std::fs::write(&foundry_toml, "[profile.default]\nsrc = \"src\"\n").unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();

        let config = Config::load_with_root(dir.path()).expect("config should load");
        let result = compile_project(&config);

        match result {
            Err(TrebError::Forge(msg)) => {
                assert!(
                    msg.contains("no input files"),
                    "expected 'no input files' in message, got: {msg}"
                );
            }
            Err(other) => panic!("expected TrebError::Forge, got: {other}"),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }
}
