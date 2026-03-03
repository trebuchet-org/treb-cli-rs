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

    /// I/O errors (file system, network).
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// A `Result` alias using [`TrebError`] as the default error type.
pub type Result<T> = std::result::Result<T, TrebError>;
