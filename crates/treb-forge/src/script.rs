//! Script execution pipeline for forge scripts.
//!
//! Builds `ScriptArgs` programmatically from treb's resolved configuration
//! and drives the execution state machine through the full pipeline.

use std::str::FromStr;

use alloy_primitives::Address;
use forge_script::ScriptArgs;
use foundry_config::Chain;
use treb_config::ResolvedConfig;
use treb_core::error::TrebError;

// TODO: Implement ExecutionResult struct (US-004)
// TODO: Implement execute_script(args) -> Result<ExecutionResult> (US-004)

/// Structured result from a forge script execution.
pub struct ExecutionResult {
    // TODO: Add fields (success, logs, gas_used, etc.)
}

/// Builder for constructing forge `ScriptArgs` from treb configuration.
pub struct ScriptConfig {
    script_path: String,
    sig: String,
    args: Vec<String>,
    target_contract: Option<String>,
    sender: Option<Address>,
    rpc_url: Option<String>,
    chain_id: Option<u64>,
    fork_url: Option<String>,
    broadcast: bool,
    slow: bool,
    debug: bool,
    dry_run: bool,
    gas_estimate_multiplier: u64,
    legacy: bool,
    non_interactive: bool,
    etherscan_api_key: Option<String>,
    verify: bool,
}

impl ScriptConfig {
    /// Create a new `ScriptConfig` with sensible defaults.
    ///
    /// Default sig is `"run()"` and gas estimate multiplier is `130`.
    pub fn new(script_path: impl Into<String>) -> Self {
        Self {
            script_path: script_path.into(),
            sig: "run()".to_string(),
            args: Vec::new(),
            target_contract: None,
            sender: None,
            rpc_url: None,
            chain_id: None,
            fork_url: None,
            broadcast: false,
            slow: false,
            debug: false,
            dry_run: false,
            gas_estimate_multiplier: 130,
            legacy: false,
            non_interactive: false,
            etherscan_api_key: None,
            verify: false,
        }
    }

    pub fn sig(&mut self, sig: impl Into<String>) -> &mut Self {
        self.sig = sig.into();
        self
    }

    pub fn args(&mut self, args: Vec<String>) -> &mut Self {
        self.args = args;
        self
    }

    pub fn target_contract(&mut self, target: impl Into<String>) -> &mut Self {
        self.target_contract = Some(target.into());
        self
    }

    pub fn sender(&mut self, sender: Address) -> &mut Self {
        self.sender = Some(sender);
        self
    }

    pub fn rpc_url(&mut self, url: impl Into<String>) -> &mut Self {
        self.rpc_url = Some(url.into());
        self
    }

    pub fn chain_id(&mut self, chain_id: u64) -> &mut Self {
        self.chain_id = Some(chain_id);
        self
    }

    pub fn fork_url(&mut self, url: impl Into<String>) -> &mut Self {
        self.fork_url = Some(url.into());
        self
    }

    pub fn broadcast(&mut self, broadcast: bool) -> &mut Self {
        self.broadcast = broadcast;
        self
    }

    pub fn slow(&mut self, slow: bool) -> &mut Self {
        self.slow = slow;
        self
    }

    pub fn debug(&mut self, debug: bool) -> &mut Self {
        self.debug = debug;
        self
    }

    pub fn dry_run(&mut self, dry_run: bool) -> &mut Self {
        self.dry_run = dry_run;
        self
    }

    pub fn gas_estimate_multiplier(&mut self, multiplier: u64) -> &mut Self {
        self.gas_estimate_multiplier = multiplier;
        self
    }

    pub fn legacy(&mut self, legacy: bool) -> &mut Self {
        self.legacy = legacy;
        self
    }

    pub fn non_interactive(&mut self, non_interactive: bool) -> &mut Self {
        self.non_interactive = non_interactive;
        self
    }

    pub fn etherscan_api_key(&mut self, key: impl Into<String>) -> &mut Self {
        self.etherscan_api_key = Some(key.into());
        self
    }

    pub fn verify(&mut self, verify: bool) -> &mut Self {
        self.verify = verify;
        self
    }

    /// Convert this `ScriptConfig` into forge `ScriptArgs`.
    ///
    /// Constructs `ScriptArgs` from `Default::default()` and sets all fields
    /// including EVM args (fork URL, sender, chain ID).
    pub fn into_script_args(self) -> treb_core::Result<ScriptArgs> {
        // fork_url takes precedence over rpc_url; they both map to EvmArgs.fork_url
        let fork_url = self.fork_url.or(self.rpc_url);

        let mut args = ScriptArgs::default();
        args.path = self.script_path;
        args.sig = self.sig;
        args.args = self.args;
        args.target_contract = self.target_contract;
        args.broadcast = if self.dry_run { false } else { self.broadcast };
        args.slow = self.slow;
        args.debug = self.debug;
        args.gas_estimate_multiplier = self.gas_estimate_multiplier;
        args.legacy = self.legacy;
        args.non_interactive = self.non_interactive;
        args.etherscan_api_key = self.etherscan_api_key;
        args.verify = self.verify;
        // Set batch_size to the sensible default (ScriptArgs::default() gives 0)
        args.batch_size = 100;

        // EVM args
        args.evm.fork_url = fork_url;
        args.evm.sender = self.sender;
        if let Some(id) = self.chain_id {
            args.evm.env.chain = Some(Chain::from(id));
        }

        Ok(args)
    }
}

/// Build a `ScriptConfig` from a resolved treb configuration.
///
/// Extracts the RPC URL (from the network name, usable as a foundry RPC alias),
/// sender address (from the "deployer" role or first sender with an address),
/// and slow flag from the resolved config.
pub fn build_script_config(
    resolved: &ResolvedConfig,
    script_path: &str,
) -> treb_core::Result<ScriptConfig> {
    let mut config = ScriptConfig::new(script_path);

    // Extract slow flag
    config.slow(resolved.slow);

    // Extract sender address: prefer "deployer" role, then fall back to any
    // sender that has an address set.
    let sender_address = resolved
        .senders
        .get("deployer")
        .and_then(|s| s.address.as_deref())
        .or_else(|| {
            resolved
                .senders
                .values()
                .find_map(|s| s.address.as_deref())
        });

    if let Some(addr_str) = sender_address {
        let addr = Address::from_str(addr_str)
            .map_err(|e| TrebError::Forge(format!("invalid sender address '{addr_str}': {e}")))?;
        config.sender(addr);
    }

    // Extract RPC URL from network name (foundry resolves aliases via [rpc_endpoints])
    if let Some(network) = &resolved.network {
        config.rpc_url(network.clone());
    }

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_config_new_produces_valid_script_args() {
        let args = ScriptConfig::new("script/Deploy.s.sol")
            .into_script_args()
            .unwrap();

        assert_eq!(args.path, "script/Deploy.s.sol");
        assert_eq!(args.sig, "run()");
        assert_eq!(args.gas_estimate_multiplier, 130);
        assert_eq!(args.batch_size, 100);
        assert!(!args.broadcast);
        assert!(!args.slow);
        assert!(!args.debug);
        assert!(!args.legacy);
        assert!(args.evm.fork_url.is_none());
        assert!(args.evm.sender.is_none());
    }
}
