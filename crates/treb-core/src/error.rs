//! Project-wide error types for treb.

use thiserror::Error;

/// Unified error type for all treb-core operations.
#[derive(Debug, Error)]
pub enum TrebError {
    /// Configuration-related errors (loading, parsing, validation).
    #[error("config error: {0}")]
    Config(String),

    /// Registry interaction errors (artifact storage, lookups).
    #[error("registry error: {0}")]
    Registry(String),

    /// Forge/compilation errors.
    #[error("forge error: {0}")]
    Forge(String),

    /// Safe Transaction Service errors.
    #[error("safe error: {0}")]
    Safe(String),

    /// Governor/governance proposal errors.
    #[error("governor error: {0}")]
    Governor(String),

    /// Fork-mode errors.
    #[error("fork error: {0}")]
    Fork(String),

    /// CLI interaction errors (non-TTY, missing input, selector failure).
    #[error("cli error: {0}")]
    Cli(String),

    /// I/O errors (file system, network).
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// A `Result` alias using [`TrebError`] as the default error type.
pub type Result<T> = std::result::Result<T, TrebError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fork_error_displays_correctly() {
        let err = TrebError::Fork("network already forked".into());
        assert_eq!(err.to_string(), "fork error: network already forked");
    }

    #[test]
    fn governor_error_displays_correctly() {
        let err = TrebError::Governor("proposal not found".into());
        assert_eq!(err.to_string(), "governor error: proposal not found");
    }
}
