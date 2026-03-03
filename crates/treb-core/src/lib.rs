//! Core library for treb — deployment management for Foundry projects.

pub mod error;
pub mod primitives {
    //! Re-exports of alloy primitive types used throughout treb.
    pub use alloy_primitives::{Address, B256, U256};
}

// Convenience re-exports.
pub use error::{Result, TrebError};

// Ensure foundry crates are linked and usable.
pub use foundry_common;
pub use foundry_config;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::{TrebError, primitives::Address};

    #[test]
    fn address_zero_is_zero() {
        let addr = Address::ZERO;
        assert!(addr.is_zero());
    }

    #[test]
    fn treb_error_config_formats() {
        let err = TrebError::Config("missing foundry.toml".to_string());
        assert_eq!(err.to_string(), "config error: missing foundry.toml");
    }

    #[test]
    fn treb_error_io_from_std() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err: TrebError = io_err.into();
        assert!(matches!(err, TrebError::Io(_)));
        assert!(err.to_string().contains("file not found"));
    }
}
