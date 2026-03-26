//! Script execution pipeline for forge scripts.
//!
//! Builds `ScriptArgs` programmatically from treb's resolved configuration
//! and drives the execution state machine through the full pipeline.

use std::{
    collections::{BTreeSet, HashMap},
    str::FromStr,
};

use alloy_network::Ethereum;
use alloy_primitives::{Address, B256, Bytes, Log};
use forge_script::ScriptArgs;
use foundry_cheatcodes::BroadcastableTransactions;
use foundry_config::Chain;
use foundry_evm::traces::Traces;
use treb_config::ResolvedConfig;
use treb_core::error::TrebError;
use treb_registry::Registry;

use crate::sender::ResolvedSender;

/// Receipt from a successfully broadcast transaction.
#[derive(Debug, Clone)]
pub struct BroadcastReceipt {
    /// On-chain transaction hash.
    pub hash: B256,
    /// Block number the transaction was included in.
    pub block_number: u64,
    /// Actual gas consumed on-chain.
    pub gas_used: u64,
    /// Whether the transaction succeeded (EIP-658 status).
    pub status: bool,
    /// Contract name if this was a deployment.
    pub contract_name: Option<String>,
    /// Deployed contract address if this was a deployment.
    pub contract_address: Option<Address>,
    /// Raw JSON-RPC receipt object, preserved for broadcast file construction.
    pub raw_receipt: Option<serde_json::Value>,
}

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
    pub transactions: Option<BroadcastableTransactions<Ethereum>>,
    /// Execution traces from the script run.
    pub traces: Traces,
    /// Receipts from broadcast transactions.
    ///
    /// `Some` when the script was run with `--broadcast` and transactions were
    /// successfully sent on-chain. `None` for simulation-only runs.
    pub broadcast_receipts: Option<Vec<BroadcastReceipt>>,
}

/// Execute a forge script through the full pipeline.
///
/// Chains: preprocess → compile → link → prepare_execution → execute.
/// If `args.broadcast` is true, continues: prepare_simulation →
/// fill_metadata → bundle → wait_for_pending → broadcast.
///
/// The optional `confirm` callback is called after execute but before
/// broadcast. It receives the simulation result so callers can preview
/// transactions. If it returns `false`, broadcast is skipped.
#[allow(clippy::type_complexity)]
pub async fn execute_script(
    args: ScriptArgs,
    confirm: Option<Box<dyn FnOnce(&ExecutionResult) -> bool + Send>>,
) -> treb_core::Result<ExecutionResult> {
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

    // Clone result before the state machine potentially consumes `executed`.
    let result = executed.execution_result.clone();

    let should_broadcast =
        executed.args.broadcast && result.transactions.as_ref().is_some_and(|txs| !txs.is_empty());

    // Build a simulation result for the confirm callback.
    let decoded_logs = crate::console::decode_console_logs(&result.logs);
    let labeled: HashMap<Address, String> = result.labeled_addresses.clone().into_iter().collect();
    let sim_result = ExecutionResult {
        success: result.success,
        logs: decoded_logs.clone(),
        raw_logs: result.logs.clone(),
        gas_used: result.gas_used,
        returned: result.returned.clone(),
        labeled_addresses: labeled.clone(),
        transactions: result.transactions.clone(),
        traces: Vec::new(), // traces are expensive to clone, skip for preview
        broadcast_receipts: None,
    };

    let confirmed = if should_broadcast { confirm.is_none_or(|f| f(&sim_result)) } else { false };

    let broadcast_receipts =
        if should_broadcast && confirmed {
            let pre_sim = executed.prepare_simulation().await.map_err(|e| {
                TrebError::Forge(format!("forge simulation preparation failed: {e}"))
            })?;

            let filled = pre_sim
                .fill_metadata()
                .await
                .map_err(|e| TrebError::Forge(format!("forge metadata fill failed: {e}")))?;

            let bundled = filled
                .bundle()
                .await
                .map_err(|e| TrebError::Forge(format!("forge bundling failed: {e}")))?;

            let bundled = bundled.wait_for_pending().await.map_err(|e| {
                TrebError::Forge(format!("forge pending transaction wait failed: {e}"))
            })?;

            let broadcasted = bundled
                .broadcast()
                .await
                .map_err(|e| TrebError::Forge(format!("forge broadcast failed: {e}")))?;

            let mut receipts = Vec::new();
            for seq in broadcasted.sequence.sequences() {
                for (tx_meta, receipt) in seq.transactions.iter().zip(seq.receipts.iter()) {
                    use alloy_network::ReceiptResponse;
                    let raw = serde_json::to_value(receipt).ok();
                    receipts.push(BroadcastReceipt {
                        hash: receipt.transaction_hash(),
                        block_number: receipt.block_number().unwrap_or_default(),
                        gas_used: receipt.gas_used(),
                        status: receipt.status(),
                        contract_name: tx_meta.contract_name.clone().filter(|s| !s.is_empty()),
                        contract_address: receipt.contract_address(),
                        raw_receipt: raw,
                    });
                }
            }
            Some(receipts)
        } else {
            None
        };

    Ok(ExecutionResult {
        success: result.success,
        logs: decoded_logs,
        raw_logs: result.logs,
        gas_used: result.gas_used,
        returned: result.returned,
        labeled_addresses: labeled.clone(),
        transactions: result.transactions,
        traces: result.traces,
        broadcast_receipts,
    })
}

/// Builder for constructing forge `ScriptArgs` from treb configuration.
pub struct ScriptConfig {
    script_path: String,
    sig: String,
    args: Vec<String>,
    libraries: Vec<String>,
    target_contract: Option<String>,
    sender: Option<Address>,
    rpc_url: Option<String>,
    chain_id: Option<u64>,
    fork_url: Option<String>,
    broadcast: bool,
    slow: bool,
    dry_run: bool,
    gas_estimate_multiplier: u64,
    legacy: bool,
    non_interactive: bool,
    etherscan_api_key: Option<String>,
    verify: bool,
    /// Private key hex string to inject into `ScriptArgs.wallets`.
    /// When set, the forge execution pipeline can sign transactions with this key.
    private_key_hex: Option<String>,
    /// Multiple private key hex strings for v2 multi-sender scripts.
    /// When set, these are injected into `ScriptArgs.wallets.private_keys`.
    private_keys_hex: Vec<String>,
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
            libraries: Vec::new(),
            target_contract: None,
            sender: None,
            rpc_url: None,
            chain_id: None,
            fork_url: None,
            broadcast: false,
            slow: false,
            dry_run: false,
            gas_estimate_multiplier: 130,
            legacy: false,
            non_interactive: false,
            etherscan_api_key: None,
            verify: false,
            private_key_hex: None,
            private_keys_hex: Vec::new(),
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

    pub fn libraries(&mut self, libraries: Vec<String>) -> &mut Self {
        self.libraries = libraries;
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

    /// Returns a reference to the configured RPC URL, if any.
    pub fn rpc_url_ref(&self) -> Option<&str> {
        self.rpc_url.as_deref()
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

    pub fn private_keys_hex(&mut self, keys: Vec<String>) -> &mut Self {
        self.private_keys_hex = keys;
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
        for library in &self.libraries {
            cmd.push("--libraries".to_string());
            cmd.push(library.clone());
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
        args.build.libraries = self.libraries;
        args.target_contract = self.target_contract;
        args.broadcast = if self.dry_run { false } else { self.broadcast };
        args.slow = self.slow;
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

        if !self.private_keys_hex.is_empty() {
            args.wallets.private_keys = Some(self.private_keys_hex);
        }

        Ok(args)
    }
}

fn format_library_reference(path: &str, contract_name: &str, address: &str) -> String {
    match path.rsplit_once(':') {
        Some((_, existing_name)) if existing_name == contract_name => {
            format!("{path}:{address}")
        }
        _ => format!("{path}:{contract_name}:{address}"),
    }
}

/// Build Foundry `--libraries` linker flags from canonical deployment records.
pub fn deployed_library_flags(registry: &Registry, namespace: &str, chain_id: u64) -> Vec<String> {
    let mut libraries = BTreeSet::new();
    for deployment in registry.list_deployments() {
        if deployment.namespace != namespace
            || deployment.chain_id != chain_id
            || deployment.deployment_type != treb_core::types::DeploymentType::Library
        {
            continue;
        }

        let path = deployment.artifact.path.trim();
        if path.is_empty() {
            continue;
        }

        libraries.insert(format_library_reference(
            path,
            &deployment.contract_name,
            &deployment.address,
        ));
    }

    libraries.into_iter().collect()
}

/// Refresh a script config's linker flags from the current registry state.
pub fn apply_deployed_libraries(
    config: &mut ScriptConfig,
    registry: &Registry,
    namespace: &str,
    chain_id: u64,
) {
    config.libraries(deployed_library_flags(registry, namespace, chain_id));
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
/// Build a `ScriptConfig` with all wallet keys injected.
///
/// Injects ALL wallet/in-memory private keys (including sub-signers for
/// Safe/Governor senders) into `ScriptArgs.wallets.private_keys`. This
/// enables `vm.broadcast(address)` for any configured sender address.
///
/// The script is always configured with `broadcast = true` because scripts
/// use `vm.broadcast()` internally, but the actual broadcast/routing is
/// handled by the pipeline orchestrator after execution.
pub fn build_script_config_with_senders(
    resolved: &ResolvedConfig,
    script_path: &str,
    resolved_senders: &HashMap<String, ResolvedSender>,
) -> treb_core::Result<ScriptConfig> {
    let mut config = build_script_config(resolved, script_path)?;

    // Collect all private keys from all senders (including sub-signers).
    let mut all_keys: Vec<String> = Vec::new();
    for (role, sender) in resolved_senders {
        let key = crate::sender::extract_signing_key(role, sender, &resolved.senders);
        if let Some(k) = key {
            if !all_keys.contains(&k.to_string()) {
                all_keys.push(k.to_string());
            }
        }
    }

    if !all_keys.is_empty() {
        config.private_keys_hex(all_keys);
    }

    // Set the sender to the deployer's broadcast address if available.
    // For Governor+timelock, this is the timelock (the on-chain executor).
    if let Some(deployer) = resolved_senders.get("deployer") {
        config.sender(deployer.broadcast_address());
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
        // wallet opts should have the private key in the keys list
        assert!(
            args.wallets
                .private_keys
                .as_ref()
                .is_some_and(|keys| keys.contains(&ANVIL_KEY_0.to_string()))
        );
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
        // wallet opts should have the sub-signer's private key for vm.broadcast()
        assert!(
            args.wallets
                .private_keys
                .as_ref()
                .is_some_and(|keys| keys.contains(&ANVIL_KEY_0.to_string()))
        );
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
            .libraries(vec!["src/Lib.sol:Lib:0x1234".to_string()])
            .rpc_url("http://localhost:8545")
            .sender(ANVIL_ADDR_0)
            .broadcast(true)
            .slow(true)
            .legacy(true)
            .verify(true);
        let cmd = config.to_forge_command();
        assert!(cmd.contains(&"--sig".to_string()));
        assert!(cmd.contains(&"deploy(uint256)".to_string()));
        assert!(cmd.contains(&"--libraries".to_string()));
        assert!(cmd.contains(&"src/Lib.sol:Lib:0x1234".to_string()));
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

    #[test]
    fn v2_script_config_injects_multiple_private_keys() {
        let deployer_config = SenderConfig {
            type_: Some(SenderType::PrivateKey),
            private_key: Some(ANVIL_KEY_0.to_string()),
            ..Default::default()
        };
        let other_key = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
        let other_config = SenderConfig {
            type_: Some(SenderType::PrivateKey),
            private_key: Some(other_key.to_string()),
            ..Default::default()
        };
        let senders = HashMap::from([
            ("deployer".to_string(), deployer_config.clone()),
            ("other".to_string(), other_config.clone()),
        ]);
        let resolved_cfg = test_resolved_config(senders);

        let mut resolved_senders = HashMap::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let all = rt.block_on(sender::resolve_all_senders(&resolved_cfg.senders)).unwrap();
        for (k, v) in all {
            resolved_senders.insert(k, v);
        }

        let config = build_script_config_with_senders(
            &resolved_cfg,
            "script/Deploy.s.sol",
            &resolved_senders,
        )
        .unwrap();
        let args = config.into_script_args().unwrap();

        // Should have multiple private keys
        let keys = args.wallets.private_keys.expect("should have private_keys");
        assert_eq!(keys.len(), 2, "should inject both wallet keys");
        assert!(keys.contains(&ANVIL_KEY_0.to_string()));
        assert!(keys.contains(&other_key.to_string()));
    }

    #[test]
    fn into_script_args_carries_library_linker_flags() {
        let mut config = ScriptConfig::new("script/Deploy.s.sol");
        config.libraries(vec!["src/Lib.sol:Lib:0x1234".to_string()]);
        let args = config.into_script_args().unwrap();

        assert_eq!(args.build.libraries, vec!["src/Lib.sol:Lib:0x1234".to_string()]);
    }
}
