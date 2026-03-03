//! Core library for treb — deployment management for Foundry projects.

pub mod primitives {
    //! Re-exports of alloy primitive types used throughout treb.
    pub use alloy_primitives::{Address, B256, U256};
}

// Ensure foundry crates are linked and usable.
pub use foundry_common;
pub use foundry_config;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::primitives::Address;

    #[test]
    fn address_zero_is_zero() {
        let addr = Address::ZERO;
        assert!(addr.is_zero());
    }
}
