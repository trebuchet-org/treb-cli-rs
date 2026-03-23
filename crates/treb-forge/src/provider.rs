//! Shared alloy provider construction helpers.
//!
//! All RPC call sites in treb-forge should build providers through these
//! helpers instead of duplicating inline `ProviderBuilder` patterns.

use alloy_network::EthereumWallet;
use alloy_provider::Provider;
use treb_core::TrebError;

/// Build an HTTP provider (no wallet) for read-only RPC calls.
pub fn build_http_provider(rpc_url: &str) -> Result<impl Provider, TrebError> {
    let url: url::Url =
        rpc_url.parse().map_err(|e| TrebError::Forge(format!("invalid RPC URL: {e}")))?;

    Ok(alloy_provider::ProviderBuilder::new().connect_http(url))
}

/// Build an HTTP provider with wallet for signing and sending transactions.
pub fn build_wallet_provider(
    rpc_url: &str,
    wallet: EthereumWallet,
) -> Result<impl Provider, TrebError> {
    let url: url::Url =
        rpc_url.parse().map_err(|e| TrebError::Forge(format!("invalid RPC URL: {e}")))?;

    Ok(alloy_provider::ProviderBuilder::new().wallet(wallet).connect_http(url))
}
