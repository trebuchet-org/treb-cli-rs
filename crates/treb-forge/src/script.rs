//! Script execution pipeline for forge scripts.
//!
//! Builds `ScriptArgs` programmatically from treb's resolved configuration
//! and drives the execution state machine through the full pipeline.

use std::{collections::HashMap, str::FromStr};

use alloy_primitives::{Address, Bytes, Log};
use forge_script::ScriptArgs;
use foundry_cheatcodes::BroadcastableTransactions;
use foundry_config::Chain;
use foundry_evm::traces::Traces;
use treb_config::ResolvedConfig;
use treb_core::error::TrebError;

use crate::sender::ResolvedSender;

/// Structured result from a forge script execution.
pub struct ExecutionResult {
    /// Whether the script execution succeeded.
    pub success: bool,
    /// Decoded console.log messages.
    pub logs: Vec<String>,
    /// Raw EVM logs emitted during execution.
    pub raw_logs: Vec<Log>,
    /// Total gas used by the script execution.
    pub gas_used: u64,
    /// Return data from the script entry point.
    pub returned: Bytes,
    /// Map of addresses to human-readable labels discovered during execution.
    pub labeled_addresses: HashMap<Address, String>,
    /// Broadcast-ready transactions collected during script execution.
    pub transactions: Option<BroadcastableTransactions>,
    /// Execution traces from the script run.
    pub traces: Traces,
}

/// Execute a forge script through the full pipeline.
///
/// Chains the state machine stages: preprocess → compile → link →
/// prepare_execution → execute, then extracts the `ScriptResult` and
/// decodes console.log messages.
pub async fn execute_script(args: ScriptArgs) -> treb_core::Result<ExecutionResult> {
    let preprocessed = args
        .preprocess()
        .await
        .map_err(|e| TrebError::Forge(format!("forge preprocessing failed: {e}")))?;

    let compiled = preprocessed
        .compile()
        .map_err(|e| TrebError::Forge(format!("forge compilation failed: {e}")))?;

    let linked = compiled
        .link()
        .await
        .map_err(|e| TrebError::Forge(format!("forge linking failed: {e}")))?;

    let prepared = linked
        .prepare_execution()
        .await
        .map_err(|e| TrebError::Forge(format!("forge execution preparation failed: {e}")))?;

    let executed = prepared
        .execute()
        .await
        .map_err(|e| TrebError::Forge(format!("forge execution failed: {e}")))?;

    let result = executed.execution_result;
    let decoded_logs = crate::console::decode_console_logs(&result.logs);
    let labeled = result.labeled_addresses.into_iter().collect();

    Ok(ExecutionResult {
        success: result.success,
        logs: decoded_logs,
        raw_logs: result.logs,
        gas_used: result.gas_used,
        returned: result.returned,
        labeled_addresses: labeled,
        transactions: result.transactions,
        traces: result.traces,
    })
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
    /// Private key hex string to inject into `ScriptArgs.wallets`.
    /// When set, the forge execution pipeline can sign transactions with this key.
    private_key_hex: Option<String>,
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
            private_key_hex: None,
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

    pub fn private_key_hex(&mut self, key: impl Into<String>) -> &mut Self {
        self.private_key_hex = Some(key.into());
        self
    }

    /// Build the equivalent `forge script` CLI command string.
    ///
    /// Returns the command as a vector of arguments suitable for display.
    /// Includes all relevant flags: path, sig, rpc-url, sender, broadcast,
    /// slow, legacy, verify.
    pub fn to_forge_command(&self) -> Vec<String> {
        let mut cmd = vec!["forge".to_string(), "script".to_string()];
        cmd.push(self.script_path.clone());
        if self.sig != "run()" {
            cmd.push("--sig".to_string());
            cmd.push(self.sig.clone());
        }
        for arg in &self.args {
            cmd.push(arg.clone());
        }
        if let Some(ref tc) = self.target_contract {
            cmd.push("--target-contract".to_string());
            cmd.push(tc.clone());
        }
        let rpc = self.fork_url.as_ref().or(self.rpc_url.as_ref());
        if let Some(url) = rpc {
            cmd.push("--rpc-url".to_string());
            cmd.push(url.clone());
        }
        if let Some(ref sender) = self.sender {
            cmd.push("--sender".to_string());
            cmd.push(format!("{:#x}", sender));
        }
        if self.broadcast {
            cmd.push("--broadcast".to_string());
        }
        if self.slow {
            cmd.push("--slow".to_string());
        }
        if self.legacy {
            cmd.push("--legacy".to_string());
        }
        if self.verify {
            cmd.push("--verify".to_string());
        }
        cmd
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

        // Wallet args — inject private key for signing
        if let Some(key) = self.private_key_hex {
            args.wallets.private_key = Some(key);
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
        .or_else(|| resolved.senders.values().find_map(|s| s.address.as_deref()));

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

/// Build a `ScriptConfig` from a resolved treb configuration with wallet integration.
///
/// Extends [`build_script_config`] by wiring resolved senders into the wallet
/// configuration. For the "deployer" role:
/// - **Wallet / InMemory** senders: sets both `evm.sender` and injects the private key into
///   `ScriptArgs.wallets` so forge can sign transactions.
/// - **Safe / Governor** senders: sets only `evm.sender` to the contract address; actual signing is
///   deferred to Phase 17.
pub fn build_script_config_with_senders(
    resolved: &ResolvedConfig,
    script_path: &str,
    resolved_senders: &HashMap<String, ResolvedSender>,
) -> treb_core::Result<ScriptConfig> {
    let mut config = build_script_config(resolved, script_path)?;

    // Look up the deployer in resolved senders
    let deployer = resolved_senders.get("deployer");

    if let Some(sender) = deployer {
        // Override the sender address with the resolved sender's address
        config.sender(sender.sender_address());

        // For Wallet/InMemory senders, inject the private key into wallet opts
        // so forge's execution pipeline can sign transactions.
        match sender {
            ResolvedSender::Wallet(_) | ResolvedSender::InMemory(_) => {
                // Get the private key hex from the original sender config
                if let Some(pk) =
                    resolved.senders.get("deployer").and_then(|sc| sc.private_key.as_deref())
                {
                    config.private_key_hex(pk);
                }
            }
            // Safe/Governor: only evm.sender is set (signing deferred to Phase 17)
            ResolvedSender::Safe { .. } | ResolvedSender::Governor { .. } => {}
        }
    }

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use alloy_primitives::address;
    use treb_config::{SenderConfig, SenderType};

    use crate::sender;

    /// Anvil account 0 private key (well-known test key).
    const ANVIL_KEY_0: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    /// Anvil account 0 address.
    const ANVIL_ADDR_0: Address = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");

    fn test_resolved_config(senders: HashMap<String, SenderConfig>) -> ResolvedConfig {
        ResolvedConfig {
            namespace: "test".to_string(),
            network: None,
            profile: "default".to_string(),
            senders,
            slow: false,
            fork_setup: None,
            config_source: "test".to_string(),
            project_root: PathBuf::from("/tmp"),
        }
    }

    #[test]
    fn script_config_new_produces_valid_script_args() {
        let args = ScriptConfig::new("script/Deploy.s.sol").into_script_args().unwrap();

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

    #[test]
    fn private_key_hex_injected_into_wallet_opts() {
        let mut config = ScriptConfig::new("script/Deploy.s.sol");
        config.private_key_hex(ANVIL_KEY_0);
        let args = config.into_script_args().unwrap();

        assert_eq!(args.wallets.private_key, Some(ANVIL_KEY_0.to_string()));
    }

    #[test]
    fn private_key_sender_sets_sender_and_wallet_opts() {
        let deployer_config = SenderConfig {
            type_: Some(SenderType::PrivateKey),
            private_key: Some(ANVIL_KEY_0.to_string()),
            ..Default::default()
        };
        let senders = HashMap::from([("deployer".to_string(), deployer_config.clone())]);
        let resolved = test_resolved_config(senders);

        // Resolve the deployer sender
        let mut resolved_senders = HashMap::new();
        let mut visited = std::collections::HashSet::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let resolved_sender = rt
            .block_on(sender::resolve_sender(
                "deployer",
                &deployer_config,
                &resolved.senders,
                &mut visited,
            ))
            .unwrap();
        resolved_senders.insert("deployer".to_string(), resolved_sender);

        let config =
            build_script_config_with_senders(&resolved, "script/Deploy.s.sol", &resolved_senders)
                .unwrap();
        let args = config.into_script_args().unwrap();

        // evm.sender should be the derived address
        assert_eq!(args.evm.sender, Some(ANVIL_ADDR_0));
        // wallet opts should have the private key
        assert_eq!(args.wallets.private_key, Some(ANVIL_KEY_0.to_string()));
    }

    #[test]
    fn safe_sender_sets_sender_without_wallet_opts() {
        let safe_addr = "0x0000000000000000000000000000000000000042";
        let deployer_config = SenderConfig {
            type_: Some(SenderType::PrivateKey),
            private_key: Some(ANVIL_KEY_0.to_string()),
            ..Default::default()
        };
        let safe_config = SenderConfig {
            type_: Some(SenderType::Safe),
            safe: Some(safe_addr.to_string()),
            signer: Some("signer".to_string()),
            ..Default::default()
        };

        let senders = HashMap::from([
            ("deployer".to_string(), safe_config.clone()),
            ("signer".to_string(), deployer_config),
        ]);
        let resolved = test_resolved_config(senders);

        // Resolve the deployer (which is a Safe sender)
        let mut resolved_senders = HashMap::new();
        let mut visited = std::collections::HashSet::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let resolved_sender = rt
            .block_on(sender::resolve_sender(
                "deployer",
                &safe_config,
                &resolved.senders,
                &mut visited,
            ))
            .unwrap();
        resolved_senders.insert("deployer".to_string(), resolved_sender);

        let config =
            build_script_config_with_senders(&resolved, "script/Deploy.s.sol", &resolved_senders)
                .unwrap();
        let args = config.into_script_args().unwrap();

        // evm.sender should be the safe address
        assert_eq!(args.evm.sender, Some(safe_addr.parse::<Address>().unwrap()));
        // wallet opts should NOT have a private key (signing deferred)
        assert!(args.wallets.private_key.is_none());
    }

    #[test]
    fn to_forge_command_basic() {
        let config = ScriptConfig::new("script/Deploy.s.sol");
        let cmd = config.to_forge_command();
        assert_eq!(cmd, vec!["forge", "script", "script/Deploy.s.sol"]);
    }

    #[test]
    fn to_forge_command_with_all_flags() {
        let mut config = ScriptConfig::new("script/Deploy.s.sol");
        config
            .sig("deploy(uint256)")
            .rpc_url("http://localhost:8545")
            .sender(ANVIL_ADDR_0)
            .broadcast(true)
            .slow(true)
            .legacy(true)
            .verify(true);
        let cmd = config.to_forge_command();
        assert!(cmd.contains(&"--sig".to_string()));
        assert!(cmd.contains(&"deploy(uint256)".to_string()));
        assert!(cmd.contains(&"--rpc-url".to_string()));
        assert!(cmd.contains(&"http://localhost:8545".to_string()));
        assert!(cmd.contains(&"--sender".to_string()));
        assert!(cmd.contains(&"--broadcast".to_string()));
        assert!(cmd.contains(&"--slow".to_string()));
        assert!(cmd.contains(&"--legacy".to_string()));
        assert!(cmd.contains(&"--verify".to_string()));
    }

    #[test]
    fn to_forge_command_default_sig_omitted() {
        let config = ScriptConfig::new("script/Deploy.s.sol");
        let cmd = config.to_forge_command();
        assert!(!cmd.contains(&"--sig".to_string()), "default sig run() should be omitted");
    }

    #[test]
    fn existing_build_script_config_preserved() {
        let deployer_config = SenderConfig {
            type_: Some(SenderType::PrivateKey),
            private_key: Some(ANVIL_KEY_0.to_string()),
            address: Some(ANVIL_ADDR_0.to_string()),
            ..Default::default()
        };
        let senders = HashMap::from([("deployer".to_string(), deployer_config)]);
        let resolved = test_resolved_config(senders);

        // Original build_script_config should still work (no wallet opts)
        let config = build_script_config(&resolved, "script/Deploy.s.sol").unwrap();
        let args = config.into_script_args().unwrap();

        assert_eq!(args.evm.sender, Some(ANVIL_ADDR_0));
        assert!(args.wallets.private_key.is_none());
    }
}
