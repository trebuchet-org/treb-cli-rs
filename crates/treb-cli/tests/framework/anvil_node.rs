// AnvilNode - Managed Anvil instance wrapper for integration tests (P2-US-004)

use treb_forge::anvil::{AnvilConfig, AnvilInstance};

/// A managed Anvil node for integration tests.
///
/// Wraps [`AnvilInstance`] with test-friendly defaults (OS-assigned port, silent mode).
/// Dropping this struct aborts the underlying server tasks and frees the port.
pub struct AnvilNode {
    instance: AnvilInstance,
}

impl AnvilNode {
    /// Spawn an Anvil node on an OS-assigned port with default settings.
    pub async fn spawn() -> Result<Self, Box<dyn std::error::Error>> {
        let instance = AnvilConfig::new().port(0).spawn().await?;
        Ok(Self { instance })
    }

    /// Spawn an Anvil node with custom configuration.
    ///
    /// Note: if no port is set in the config, Anvil may bind to a fixed port.
    /// For test isolation, ensure `.port(0)` is set.
    pub async fn spawn_with_config(config: AnvilConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let instance = config.spawn().await?;
        Ok(Self { instance })
    }

    /// The port this Anvil node is listening on.
    pub fn port(&self) -> u16 {
        self.instance.port()
    }

    /// The EVM chain ID of this Anvil node.
    pub fn chain_id(&self) -> u64 {
        self.instance.chain_id()
    }

    /// The HTTP RPC URL of this Anvil node (e.g. `http://127.0.0.1:12345`).
    pub fn rpc_url(&self) -> &str {
        self.instance.rpc_url()
    }

    /// Access the underlying [`AnvilInstance`] for advanced operations.
    pub fn instance(&self) -> &AnvilInstance {
        &self.instance
    }
}
