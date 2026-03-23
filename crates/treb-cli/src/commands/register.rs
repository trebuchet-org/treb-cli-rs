//! `treb register` command implementation.
//!
//! Fetches a historical transaction, traces it for contract creations,
//! and records deployments in the registry. Falls back to receipt-only
//! mode when `debug_traceTransaction` is unsupported by the RPC.

use std::{collections::HashMap, env};

use anyhow::{Context, bail};
use chrono::Utc;
use serde::Serialize;
use treb_config::{ResolveOpts, load_local_config, resolve_config};
use treb_core::types::{
    ArtifactInfo, Deployment, DeploymentMethod, DeploymentStrategy, DeploymentType, Operation,
    ProxyInfo, Transaction, TransactionStatus, VerificationInfo, VerificationStatus,
    generate_deployment_id,
};
use treb_registry::Registry;

use crate::{
    commands::receipt::{
        DetectedProxy, TracedCreation, detect_proxy_patterns, extract_creations_from_trace,
        parse_hex_u64, receipt_contract_address, rpc_call, rpc_client,
    },
    output,
    ui::{color, emoji},
};
use owo_colors::{OwoColorize, Style};

const FOUNDRY_TOML: &str = "foundry.toml";
const TREB_DIR: &str = ".treb";

// ── RPC URL resolution ──────────────────────────────────────────────────

/// Resolve the RPC URL from --rpc-url or --network (via foundry.toml endpoints).
fn resolve_rpc_url(
    rpc_url: Option<String>,
    network: Option<String>,
    cwd: &std::path::Path,
) -> anyhow::Result<String> {
    if let Some(url) = rpc_url {
        return Ok(url);
    }

    let network_name = network.context("either --rpc-url or --network must be specified")?;

    // If it already looks like a URL, use it directly.
    if network_name.starts_with("http://") || network_name.starts_with("https://") {
        return Ok(network_name);
    }

    let endpoints = treb_config::resolve_rpc_endpoints(cwd).map_err(|e| anyhow::anyhow!("{e}"))?;

    let url = endpoints.get(&network_name).ok_or_else(|| {
        anyhow::anyhow!("network '{}' not found in foundry.toml [rpc_endpoints]", network_name)
    })?;

    if !url.missing_vars.is_empty() {
        bail!(
            "RPC URL for network '{}' is missing required environment variables after .env expansion: {}\n\n\
             Set the required environment variables or use --rpc-url directly.",
            network_name,
            url.missing_vars.join(", ")
        );
    }

    if url.unresolved {
        bail!(
            "RPC URL for network '{}' contains unresolved environment variables: {}\n\n\
             Set the required environment variables or use --rpc-url directly.",
            network_name,
            url.raw_url
        );
    }

    if url.expanded_url.trim().is_empty() {
        bail!(
            "RPC URL for network '{}' is empty after .env expansion\n\n\
             Set the required environment variables or use --rpc-url directly.",
            network_name
        );
    }

    Ok(url.expanded_url.clone())
}

/// Resolve the effective network for `register`.
///
/// Explicit `--network` wins. When `--rpc-url` is provided, network lookup is
/// not needed. Otherwise, fall back to the active config network.
fn resolve_effective_network(
    network: Option<String>,
    rpc_url: Option<&str>,
    cwd: &std::path::Path,
) -> anyhow::Result<Option<String>> {
    if network.is_some() || rpc_url.is_some() {
        return Ok(network);
    }

    let local = load_local_config(cwd).map_err(|err| anyhow::anyhow!("{err}"))?;

    if local.network.is_empty() {
        anyhow::bail!("no active network set in config, --network flag is required");
    }

    Ok(Some(local.network))
}

/// Resolve the effective namespace for `register`.
///
/// Explicit `--namespace` wins. Otherwise, reuse the shared config resolver so
/// register follows the same namespace precedence as other config-backed
/// commands.
fn resolve_effective_namespace(
    namespace: Option<String>,
    cwd: &std::path::Path,
) -> anyhow::Result<String> {
    if let Some(namespace) = namespace {
        return Ok(namespace);
    }

    let resolved = resolve_config(ResolveOpts {
        project_root: cwd.to_path_buf(),
        namespace: None,
        network: None,
        profile: None,
        sender_overrides: HashMap::new(),
    });

    Ok(match resolved {
        Ok(resolved) => resolved.namespace,
        Err(_) => "default".to_string(),
    })
}

// ── Contract name resolution ────────────────────────────────────────────

/// Resolve the effective contract name from --contract-name or --contract.
///
/// `--contract-name` takes precedence. If only `--contract` is provided,
/// the name is extracted from the artifact path (e.g., "src/Counter.sol:Counter"
/// → "Counter").
fn resolve_contract_name(
    contract_name: Option<String>,
    contract: Option<String>,
) -> Option<String> {
    if contract_name.is_some() {
        return contract_name;
    }

    if let Some(ref artifact) = contract {
        if let Some(pos) = artifact.rfind(':') {
            return Some(artifact[pos + 1..].to_string());
        }
        if let Some(pos) = artifact.rfind('/') {
            let name = &artifact[pos + 1..];
            return Some(name.strip_suffix(".sol").unwrap_or(name).to_string());
        }
        return Some(artifact.clone());
    }

    None
}

fn deployment_labels(
    label: Option<&str>,
    total_creations: usize,
    index: usize,
) -> (String, Option<String>) {
    let display_label = label.filter(|label| !label.is_empty()).map(str::to_owned);
    let effective_label = display_label.clone().unwrap_or_default();
    let registry_label =
        if total_creations == 1 { effective_label } else { format!("{effective_label}_{index}") };

    (registry_label, display_label)
}

// ── Output types ────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RegisterOutputJson {
    deployments: Vec<RegisteredDeploymentJson>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RegisteredDeploymentJson {
    address: String,
    contract_name: String,
    deployment_id: String,
    label: String,
}

/// Apply a color style when color is enabled, plain text otherwise.
fn styled(text: &str, style: Style) -> String {
    if color::is_color_enabled() { format!("{}", text.style(style)) } else { text.to_string() }
}

// ── Main entry point ────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub async fn run(
    tx_hash: &str,
    network: Option<String>,
    rpc_url: Option<String>,
    address: Option<String>,
    contract: Option<String>,
    contract_name: Option<String>,
    label: Option<String>,
    namespace: Option<String>,
    deployment_type: Option<DeploymentType>,
    _skip_verify: bool,
    json: bool,
) -> anyhow::Result<()> {
    let cwd = env::current_dir().context("failed to determine current directory")?;

    // ── Validate ────────────────────────────────────────────────────────
    if !cwd.join(FOUNDRY_TOML).exists() {
        bail!(
            "no foundry.toml found in {}\n\n\
             Run `forge init` to create a Foundry project, then `treb init`.",
            cwd.display()
        );
    }
    if !cwd.join(TREB_DIR).exists() {
        bail!(
            "project not initialized — .treb/ directory not found in {}\n\n\
             Run `treb init` first.",
            cwd.display()
        );
    }
    if !tx_hash.starts_with("0x") && !tx_hash.starts_with("0X") {
        bail!("--tx-hash must be a hex string starting with 0x");
    }

    // ── Resolve RPC URL ─────────────────────────────────────────────────
    let network = resolve_effective_network(network, rpc_url.as_deref(), &cwd)?;
    let effective_rpc_url = resolve_rpc_url(rpc_url, network, &cwd)?;
    let client = rpc_client()?;

    // ── Fetch chain ID ──────────────────────────────────────────────────
    let chain_id_result =
        rpc_call(&client, &effective_rpc_url, "eth_chainId", serde_json::json!([])).await?;
    let chain_id = parse_hex_u64(chain_id_result.as_str().unwrap_or("0x0"));

    // ── Fetch transaction ───────────────────────────────────────────────
    let tx_result = rpc_call(
        &client,
        &effective_rpc_url,
        "eth_getTransactionByHash",
        serde_json::json!([tx_hash]),
    )
    .await?;

    if tx_result.is_null() {
        bail!("transaction not found: {tx_hash}");
    }

    let sender = tx_result.get("from").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let nonce = parse_hex_u64(tx_result.get("nonce").and_then(|v| v.as_str()).unwrap_or("0x0"));

    // ── Fetch receipt ───────────────────────────────────────────────────
    let receipt_result = rpc_call(
        &client,
        &effective_rpc_url,
        "eth_getTransactionReceipt",
        serde_json::json!([tx_hash]),
    )
    .await?;

    if receipt_result.is_null() {
        bail!("transaction receipt not found (transaction may be pending): {tx_hash}");
    }

    let block_number =
        parse_hex_u64(receipt_result.get("blockNumber").and_then(|v| v.as_str()).unwrap_or("0x0"));

    let receipt_status = receipt_result.get("status").and_then(|v| v.as_str()).unwrap_or("0x1");
    if receipt_status == "0x0" {
        bail!("transaction reverted: {tx_hash}");
    }

    // ── Try trace, fall back to receipt-only ─────────────────────────────
    if !json {
        output::print_stage("\u{1f50d}", "Tracing transaction...");
    }

    let mut creations;
    let trace_result = rpc_call(
        &client,
        &effective_rpc_url,
        "debug_traceTransaction",
        serde_json::json!([tx_hash, {"tracer": "callTracer"}]),
    )
    .await;

    let mut proxy_patterns: Vec<DetectedProxy> = Vec::new();

    match trace_result {
        Ok(trace) => {
            creations = extract_creations_from_trace(&trace);
            proxy_patterns = detect_proxy_patterns(&trace);

            // Also check receipt contractAddress (direct CREATE may appear
            // at the top level of the trace, but add it as a safety net).
            if let Some(addr) = receipt_contract_address(&receipt_result) {
                if !creations.iter().any(|c| c.address.eq_ignore_ascii_case(&addr)) {
                    creations.push(TracedCreation {
                        address: addr,
                        from: sender.clone(),
                        create_type: "CREATE".to_string(),
                    });
                }
            }
        }
        Err(_) => {
            if !json {
                eprintln!(
                    "{}",
                    output::format_warning_banner(
                        "\u{26a0}\u{fe0f}",
                        "debug_traceTransaction not available, using receipt-only mode"
                    )
                );
            }
            creations = Vec::new();

            if let Some(addr) = receipt_contract_address(&receipt_result) {
                creations.push(TracedCreation {
                    address: addr,
                    from: sender.clone(),
                    create_type: "CREATE".to_string(),
                });
            }
        }
    }

    if creations.is_empty() {
        bail!("no contract creations found in transaction {tx_hash}");
    }

    // ── Filter by --address ─────────────────────────────────────────────
    if let Some(ref filter_addr) = address {
        let filter_lower = filter_addr.to_lowercase();
        creations.retain(|c| c.address.to_lowercase() == filter_lower);
        if creations.is_empty() {
            bail!("address {} not found in transaction {tx_hash}", filter_addr);
        }
    }

    // ── Build defaults ──────────────────────────────────────────────────
    let effective_namespace = resolve_effective_namespace(namespace, &cwd)?;
    let effective_name = resolve_contract_name(contract_name, contract);
    let tx_id = format!("tx-{tx_hash}");

    // ── Open registry ───────────────────────────────────────────────────
    let mut registry = Registry::open(&cwd).context("failed to open registry")?;

    // ── Register each creation ──────────────────────────────────────────
    if !json {
        output::print_stage("\u{1f4dd}", "Registering deployments...");
    }

    let mut registered: Vec<(RegisteredDeploymentJson, String)> = Vec::new();
    let mut dep_labels: Vec<Option<String>> = Vec::new();

    for (i, creation) in creations.iter().enumerate() {
        let name = match &effective_name {
            Some(n) if creations.len() == 1 => n.clone(),
            Some(n) => format!("{n}_{i}"),
            None if creations.len() == 1 => "Unknown".to_string(),
            None => format!("Unknown_{i}"),
        };

        let (dep_label, display_label) = deployment_labels(label.as_deref(), creations.len(), i);

        let deployment_id =
            generate_deployment_id(&effective_namespace, chain_id, &name, &dep_label);

        // Duplicate detection
        if registry.get_deployment(&deployment_id).is_some() {
            bail!(
                "deployment already exists: {deployment_id}\n\n\
                 Use a different --label or --namespace to avoid conflicts."
            );
        }

        let method = match creation.create_type.as_str() {
            "CREATE2" => DeploymentMethod::Create2,
            "CREATE3" => DeploymentMethod::Create3,
            _ => DeploymentMethod::Create,
        };

        // Determine deployment type and proxy info.
        // --deployment-type flag overrides auto-detection.
        let detected_proxy =
            proxy_patterns.iter().find(|p| p.proxy_address.eq_ignore_ascii_case(&creation.address));

        let (effective_dep_type, effective_proxy_info) = match &deployment_type {
            Some(dt) => (dt.clone(), None),
            None => match detected_proxy {
                Some(dp) => (
                    DeploymentType::Proxy,
                    Some(ProxyInfo {
                        proxy_type: String::new(),
                        implementation: dp.implementation_address.clone(),
                        admin: String::new(),
                        history: vec![],
                    }),
                ),
                None => (DeploymentType::Unknown, None),
            },
        };

        dep_labels.push(display_label.clone());

        let deployment = Deployment {
            id: deployment_id.clone(),
            namespace: effective_namespace.clone(),
            chain_id,
            contract_name: name.clone(),
            label: dep_label.clone(),
            address: creation.address.clone(),
            deployment_type: effective_dep_type,
            transaction_id: tx_id.clone(),
            deployment_strategy: DeploymentStrategy {
                method,
                salt: String::new(),
                init_code_hash: String::new(),
                factory: if creation.from != sender {
                    creation.from.clone()
                } else {
                    String::new()
                },
                constructor_args: String::new(),
                entropy: String::new(),
            },
            proxy_info: effective_proxy_info,
            artifact: ArtifactInfo {
                path: String::new(),
                compiler_version: String::new(),
                bytecode_hash: String::new(),
                script_path: String::new(),
                git_commit: String::new(),
            },
            verification: VerificationInfo {
                status: VerificationStatus::Unverified,
                etherscan_url: String::new(),
                verified_at: None,
                reason: String::new(),
                verifiers: HashMap::new(),
            },
            tags: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        registry
            .insert_deployment(deployment)
            .with_context(|| format!("failed to register deployment {deployment_id}"))?;

        registered.push((
            RegisteredDeploymentJson {
                address: creation.address.clone(),
                contract_name: name,
                deployment_id,
                label: display_label.unwrap_or_default(),
            },
            creation.create_type.clone(),
        ));
    }

    // ── Create transaction record ───────────────────────────────────────
    let deployment_ids: Vec<String> =
        registered.iter().map(|(d, _)| d.deployment_id.clone()).collect();

    let operations: Vec<Operation> = registered
        .iter()
        .map(|(d, create_type)| Operation {
            operation_type: "DEPLOY".to_string(),
            target: d.address.clone(),
            method: create_type.clone(),
            result: {
                let mut m = HashMap::new();
                m.insert("address".into(), serde_json::Value::String(d.address.clone()));
                m
            },
        })
        .collect();

    let transaction = Transaction {
        id: tx_id.clone(),
        chain_id,
        hash: tx_hash.to_string(),
        status: TransactionStatus::Executed,
        block_number,
        sender,
        nonce,
        deployments: deployment_ids,
        operations,
        safe_context: None,
        broadcast_file: None,
        environment: effective_namespace.clone(),
        created_at: Utc::now(),
    };

    registry.insert_transaction(transaction).context("failed to record transaction")?;

    // ── Display ─────────────────────────────────────────────────────────
    let dep_jsons: Vec<RegisteredDeploymentJson> = registered.into_iter().map(|(d, _)| d).collect();

    if json {
        output::print_json(&RegisterOutputJson { deployments: dep_jsons })?;
    } else {
        let n = dep_jsons.len();
        let header = format!("{} Successfully registered {} deployment(s)", emoji::CHECK_MARK, n);
        println!("{}\n", styled(&header, color::SUCCESS));

        for (i, (dep, label)) in dep_jsons.iter().zip(dep_labels.iter()).enumerate() {
            println!("  Deployment {}:", i + 1);
            println!("    Deployment ID: {}", dep.deployment_id);
            println!("    Address: {}", dep.address);
            println!("    Contract: {}", dep.contract_name);
            if let Some(label) = label {
                println!("    Label: {}", label);
            }
            if i < n - 1 {
                println!();
            }
        }
    }

    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use super::*;

    use tempfile::TempDir;
    use treb_config::{LocalConfig, save_local_config};

    // ── Helper ──────────────────────────────────────────────────────────

    fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().expect("env test lock poisoned")
    }

    struct EnvVarGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvVarGuard {
        fn unset(key: &'static str) -> Self {
            let original = std::env::var(key).ok();
            // SAFETY: Serialized by env_lock() in tests that mutate env vars.
            unsafe { std::env::remove_var(key) };
            Self { key, original }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => {
                    // SAFETY: Serialized by env_lock() in tests that mutate env vars.
                    unsafe { std::env::set_var(self.key, value) };
                }
                None => {
                    // SAFETY: Serialized by env_lock() in tests that mutate env vars.
                    unsafe { std::env::remove_var(self.key) };
                }
            }
        }
    }

    fn setup_project(dir: &std::path::Path) {
        std::fs::write(
            dir.join("foundry.toml"),
            r#"
[profile.default]
src = "src"

[rpc_endpoints]
mainnet = "https://eth-mainnet.example.com"
sepolia = "https://sepolia.example.com"
needs_env = "https://rpc.example.com/${API_KEY}"
"#,
        )
        .unwrap();
    }

    fn setup_project_with_config(dir: &std::path::Path, network: &str) {
        setup_project(dir);
        save_local_config(
            dir,
            &LocalConfig { namespace: "default".to_string(), network: network.to_string() },
        )
        .unwrap();
    }

    // ── resolve_effective_network ───────────────────────────────────────

    #[test]
    fn effective_network_falls_back_to_config() {
        let tmp = TempDir::new().unwrap();
        setup_project_with_config(tmp.path(), "mainnet");

        let network = resolve_effective_network(None, None, tmp.path()).unwrap();

        assert_eq!(network.as_deref(), Some("mainnet"));
    }

    #[test]
    fn effective_network_ignores_invalid_local_namespace() {
        let tmp = TempDir::new().unwrap();
        setup_project(tmp.path());
        std::fs::write(
            tmp.path().join("treb.toml"),
            r#"
[namespace.default]
profile = "default"
"#,
        )
        .unwrap();
        save_local_config(
            tmp.path(),
            &LocalConfig { namespace: "staging".to_string(), network: "mainnet".to_string() },
        )
        .unwrap();

        let network = resolve_effective_network(None, None, tmp.path()).unwrap();

        assert_eq!(network.as_deref(), Some("mainnet"));
    }

    #[test]
    fn effective_network_missing_config_errors() {
        let tmp = TempDir::new().unwrap();
        setup_project(tmp.path());

        let err = resolve_effective_network(None, None, tmp.path()).unwrap_err();

        assert_eq!(err.to_string(), "no active network set in config, --network flag is required");
    }

    // ── resolve_effective_namespace ───────────────────────────────────

    fn setup_project_with_namespace_config(dir: &std::path::Path, namespace: &str) {
        setup_project(dir);
        std::fs::write(
            dir.join("treb.toml"),
            r#"
[accounts.deployer]
type = "private_key"
address = "0x0000000000000000000000000000000000000001"
private_key = "0x01"

[namespace.default]
profile = "default"

[namespace.default.senders]
deployer = "deployer"

[namespace.staging]
profile = "staging"

[namespace.staging.senders]
deployer = "deployer"
"#,
        )
        .unwrap();
        save_local_config(
            dir,
            &LocalConfig { namespace: namespace.to_string(), network: "mainnet".to_string() },
        )
        .unwrap();
    }

    #[test]
    fn effective_namespace_prefers_explicit_flag() {
        let tmp = TempDir::new().unwrap();
        setup_project_with_namespace_config(tmp.path(), "staging");

        let namespace =
            resolve_effective_namespace(Some("production".to_string()), tmp.path()).unwrap();

        assert_eq!(namespace, "production");
    }

    #[test]
    fn effective_namespace_falls_back_to_config() {
        let tmp = TempDir::new().unwrap();
        setup_project_with_namespace_config(tmp.path(), "staging");

        let namespace = resolve_effective_namespace(None, tmp.path()).unwrap();
        let deployment_id = generate_deployment_id(&namespace, 31_337, "Counter", "");

        assert_eq!(namespace, "staging");
        assert_eq!(deployment_id, "staging/31337/Counter");
    }

    #[test]
    fn effective_namespace_defaults_when_config_file_missing() {
        let tmp = TempDir::new().unwrap();
        setup_project(tmp.path());
        std::fs::create_dir_all(tmp.path().join(".treb")).unwrap();

        let namespace = resolve_effective_namespace(None, tmp.path()).unwrap();

        assert_eq!(namespace, "default");
    }

    #[test]
    fn effective_namespace_defaults_when_config_resolution_fails() {
        let tmp = TempDir::new().unwrap();
        setup_project(tmp.path());
        std::fs::write(
            tmp.path().join("treb.toml"),
            r#"
[namespace.default]
profile = "default"
"#,
        )
        .unwrap();
        save_local_config(
            tmp.path(),
            &LocalConfig { namespace: "staging".to_string(), network: "mainnet".to_string() },
        )
        .unwrap();

        let namespace = resolve_effective_namespace(None, tmp.path()).unwrap();

        assert_eq!(namespace, "default");
    }

    // ── resolve_rpc_url ─────────────────────────────────────────────────

    #[test]
    fn rpc_url_direct_overrides_network() {
        let tmp = TempDir::new().unwrap();
        let url = resolve_rpc_url(
            Some("https://my-rpc.example.com".to_string()),
            Some("mainnet".to_string()),
            tmp.path(),
        )
        .unwrap();
        assert_eq!(url, "https://my-rpc.example.com");
    }

    #[test]
    fn rpc_url_resolves_from_foundry_toml() {
        let tmp = TempDir::new().unwrap();
        setup_project(tmp.path());
        let url = resolve_rpc_url(None, Some("mainnet".to_string()), tmp.path()).unwrap();
        assert_eq!(url, "https://eth-mainnet.example.com");
    }

    #[test]
    fn rpc_url_network_as_url_passthrough() {
        let tmp = TempDir::new().unwrap();
        let url =
            resolve_rpc_url(None, Some("https://direct-url.example.com".to_string()), tmp.path())
                .unwrap();
        assert_eq!(url, "https://direct-url.example.com");
    }

    #[test]
    fn rpc_url_missing_both_errors() {
        let tmp = TempDir::new().unwrap();
        let err = resolve_rpc_url(None, None, tmp.path()).unwrap_err();
        assert!(err.to_string().contains("--rpc-url or --network"));
    }

    #[test]
    fn rpc_url_unknown_network_errors() {
        let tmp = TempDir::new().unwrap();
        setup_project(tmp.path());
        let err = resolve_rpc_url(None, Some("goerli".to_string()), tmp.path()).unwrap_err();
        assert!(err.to_string().contains("goerli"));
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn rpc_url_resolves_from_dotenv() {
        let _lock = env_lock();
        let _api_key = EnvVarGuard::unset("API_KEY");
        let tmp = TempDir::new().unwrap();
        setup_project(tmp.path());
        std::fs::write(tmp.path().join(".env"), "API_KEY=dotenv-key\n").unwrap();

        let url = resolve_rpc_url(None, Some("needs_env".to_string()), tmp.path()).unwrap();

        assert_eq!(url, "https://rpc.example.com/dotenv-key");
    }

    #[test]
    fn rpc_url_missing_env_var_errors() {
        let _lock = env_lock();
        let _api_key = EnvVarGuard::unset("API_KEY");
        let tmp = TempDir::new().unwrap();
        setup_project(tmp.path());
        let err = resolve_rpc_url(None, Some("needs_env".to_string()), tmp.path()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("needs_env"), "got: {msg}");
        assert!(msg.contains("API_KEY"), "got: {msg}");
    }

    // ── extract_creations_from_trace ────────────────────────────────────

    #[test]
    fn trace_single_create() {
        let trace = serde_json::json!({
            "type": "CREATE",
            "from": "0xSender",
            "to": "0xCreated",
            "value": "0x0",
            "input": "0x6080...",
            "output": "0x6080..."
        });
        let creations = extract_creations_from_trace(&trace);
        assert_eq!(creations.len(), 1);
        assert_eq!(creations[0].address, "0xCreated");
        assert_eq!(creations[0].from, "0xSender");
        assert_eq!(creations[0].create_type, "CREATE");
    }

    #[test]
    fn trace_nested_creates() {
        let trace = serde_json::json!({
            "type": "CALL",
            "from": "0xSender",
            "to": "0xFactory",
            "calls": [
                {
                    "type": "CREATE",
                    "from": "0xFactory",
                    "to": "0xChild1"
                },
                {
                    "type": "CREATE2",
                    "from": "0xFactory",
                    "to": "0xChild2"
                }
            ]
        });
        let creations = extract_creations_from_trace(&trace);
        assert_eq!(creations.len(), 2);
        assert_eq!(creations[0].address, "0xChild1");
        assert_eq!(creations[0].create_type, "CREATE");
        assert_eq!(creations[1].address, "0xChild2");
        assert_eq!(creations[1].create_type, "CREATE2");
    }

    #[test]
    fn trace_deeply_nested_create() {
        let trace = serde_json::json!({
            "type": "CALL",
            "from": "0xSender",
            "to": "0xRouter",
            "calls": [{
                "type": "DELEGATECALL",
                "from": "0xRouter",
                "to": "0xImpl",
                "calls": [{
                    "type": "CREATE",
                    "from": "0xImpl",
                    "to": "0xDeepChild"
                }]
            }]
        });
        let creations = extract_creations_from_trace(&trace);
        assert_eq!(creations.len(), 1);
        assert_eq!(creations[0].address, "0xDeepChild");
        assert_eq!(creations[0].from, "0xImpl");
    }

    #[test]
    fn trace_no_creates() {
        let trace = serde_json::json!({
            "type": "CALL",
            "from": "0xSender",
            "to": "0xContract",
            "calls": [{
                "type": "CALL",
                "from": "0xContract",
                "to": "0xOther"
            }]
        });
        let creations = extract_creations_from_trace(&trace);
        assert!(creations.is_empty());
    }

    #[test]
    fn trace_create_without_to_is_skipped() {
        let trace = serde_json::json!({
            "type": "CREATE",
            "from": "0xSender"
            // no "to" field — creation failed before address assigned
        });
        let creations = extract_creations_from_trace(&trace);
        assert!(creations.is_empty());
    }

    // ── resolve_contract_name ───────────────────────────────────────────

    #[test]
    fn contract_name_takes_precedence() {
        let result = resolve_contract_name(
            Some("Counter".to_string()),
            Some("src/Counter.sol:Counter".to_string()),
        );
        assert_eq!(result, Some("Counter".to_string()));
    }

    #[test]
    fn contract_name_extracted_from_artifact_colon() {
        let result = resolve_contract_name(None, Some("src/Counter.sol:Counter".to_string()));
        assert_eq!(result, Some("Counter".to_string()));
    }

    #[test]
    fn contract_name_extracted_from_artifact_path() {
        let result = resolve_contract_name(None, Some("src/Counter.sol".to_string()));
        assert_eq!(result, Some("Counter".to_string()));
    }

    #[test]
    fn contract_name_bare_name() {
        let result = resolve_contract_name(None, Some("Counter".to_string()));
        assert_eq!(result, Some("Counter".to_string()));
    }

    #[test]
    fn contract_name_none_when_both_empty() {
        let result = resolve_contract_name(None, None);
        assert_eq!(result, None);
    }

    #[test]
    fn deployment_labels_hide_generated_suffix_for_unlabeled_multi_create() {
        let (registry_label, display_label) = deployment_labels(None, 2, 0);

        assert_eq!(registry_label, "_0");
        assert_eq!(display_label, None);
    }

    #[test]
    fn deployment_labels_hide_generated_suffix_for_labeled_multi_create() {
        let (registry_label, display_label) = deployment_labels(Some("factory"), 2, 1);

        assert_eq!(registry_label, "factory_1");
        assert_eq!(display_label.as_deref(), Some("factory"));
    }

    #[test]
    fn register_json_label_uses_display_label_for_unlabeled_multi_create() {
        let (registry_label, display_label) = deployment_labels(None, 2, 0);
        let payload = serde_json::to_value(RegisteredDeploymentJson {
            address: "0x1234".to_string(),
            contract_name: "Unknown_0".to_string(),
            deployment_id: "default/31337/Unknown_0:_0".to_string(),
            label: display_label.unwrap_or_default(),
        })
        .expect("register deployment json should serialize");

        assert_eq!(registry_label, "_0");
        assert_eq!(payload["label"], "");
    }

    #[test]
    fn register_json_label_uses_display_label_for_labeled_multi_create() {
        let (registry_label, display_label) = deployment_labels(Some("factory"), 2, 1);
        let payload = serde_json::to_value(RegisteredDeploymentJson {
            address: "0x1234".to_string(),
            contract_name: "Factory_1".to_string(),
            deployment_id: "default/31337/Factory_1:factory_1".to_string(),
            label: display_label.unwrap_or_default(),
        })
        .expect("register deployment json should serialize");

        assert_eq!(registry_label, "factory_1");
        assert_eq!(payload["label"], "factory");
    }

    // ── parse_hex_u64 ───────────────────────────────────────────────────

    #[test]
    fn parse_hex_with_prefix() {
        assert_eq!(parse_hex_u64("0x1"), 1);
        assert_eq!(parse_hex_u64("0xff"), 255);
        assert_eq!(parse_hex_u64("0x12d687"), 1234567);
    }

    #[test]
    fn parse_hex_without_prefix() {
        assert_eq!(parse_hex_u64("ff"), 255);
    }

    #[test]
    fn parse_hex_invalid_returns_zero() {
        assert_eq!(parse_hex_u64("not_hex"), 0);
    }

    // ── receipt_contract_address ─────────────────────────────────────────

    #[test]
    fn receipt_has_contract_address() {
        let receipt = serde_json::json!({
            "contractAddress": "0xNewContract"
        });
        assert_eq!(receipt_contract_address(&receipt), Some("0xNewContract".to_string()));
    }

    #[test]
    fn receipt_null_contract_address() {
        let receipt = serde_json::json!({
            "contractAddress": null
        });
        assert_eq!(receipt_contract_address(&receipt), None);
    }

    #[test]
    fn receipt_zero_contract_address() {
        let receipt = serde_json::json!({
            "contractAddress": "0x0000000000000000000000000000000000000000"
        });
        assert_eq!(receipt_contract_address(&receipt), None);
    }

    #[test]
    fn receipt_empty_contract_address() {
        let receipt = serde_json::json!({
            "contractAddress": ""
        });
        assert_eq!(receipt_contract_address(&receipt), None);
    }

    #[test]
    fn receipt_missing_contract_address() {
        let receipt = serde_json::json!({
            "status": "0x1"
        });
        assert_eq!(receipt_contract_address(&receipt), None);
    }

    // ── detect_proxy_patterns ──────────────────────────────────────────

    #[test]
    fn proxy_detected_create_with_delegatecall() {
        let trace = serde_json::json!({
            "type": "CALL",
            "from": "0xSender",
            "to": "0xFactory",
            "calls": [{
                "type": "CREATE",
                "from": "0xFactory",
                "to": "0xProxy",
                "calls": [{
                    "type": "DELEGATECALL",
                    "from": "0xProxy",
                    "to": "0xImplementation"
                }]
            }]
        });
        let patterns = detect_proxy_patterns(&trace);
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].proxy_address, "0xProxy");
        assert_eq!(patterns[0].implementation_address, "0xImplementation");
    }

    #[test]
    fn proxy_detected_create2_with_delegatecall() {
        let trace = serde_json::json!({
            "type": "CALL",
            "from": "0xSender",
            "to": "0xFactory",
            "calls": [{
                "type": "CREATE2",
                "from": "0xFactory",
                "to": "0xProxy",
                "calls": [{
                    "type": "DELEGATECALL",
                    "from": "0xProxy",
                    "to": "0xImpl"
                }]
            }]
        });
        let patterns = detect_proxy_patterns(&trace);
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].proxy_address, "0xProxy");
        assert_eq!(patterns[0].implementation_address, "0xImpl");
    }

    #[test]
    fn proxy_not_detected_without_delegatecall() {
        let trace = serde_json::json!({
            "type": "CALL",
            "from": "0xSender",
            "to": "0xFactory",
            "calls": [{
                "type": "CREATE",
                "from": "0xFactory",
                "to": "0xContract",
                "calls": [{
                    "type": "CALL",
                    "from": "0xContract",
                    "to": "0xOther"
                }]
            }]
        });
        let patterns = detect_proxy_patterns(&trace);
        assert!(patterns.is_empty());
    }

    #[test]
    fn proxy_not_detected_delegatecall_from_different_address() {
        let trace = serde_json::json!({
            "type": "CALL",
            "from": "0xSender",
            "to": "0xFactory",
            "calls": [{
                "type": "CREATE",
                "from": "0xFactory",
                "to": "0xContract",
                "calls": [{
                    "type": "DELEGATECALL",
                    "from": "0xOtherAddress",
                    "to": "0xImpl"
                }]
            }]
        });
        let patterns = detect_proxy_patterns(&trace);
        assert!(patterns.is_empty());
    }

    #[test]
    fn proxy_detected_deeply_nested_delegatecall() {
        let trace = serde_json::json!({
            "type": "CALL",
            "from": "0xSender",
            "to": "0xFactory",
            "calls": [{
                "type": "CREATE",
                "from": "0xFactory",
                "to": "0xProxy",
                "calls": [{
                    "type": "CALL",
                    "from": "0xProxy",
                    "to": "0xHelper",
                    "calls": [{
                        "type": "DELEGATECALL",
                        "from": "0xProxy",
                        "to": "0xImpl"
                    }]
                }]
            }]
        });
        let patterns = detect_proxy_patterns(&trace);
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].implementation_address, "0xImpl");
    }

    #[test]
    fn proxy_no_creates_no_patterns() {
        let trace = serde_json::json!({
            "type": "CALL",
            "from": "0xSender",
            "to": "0xContract",
            "calls": [{
                "type": "DELEGATECALL",
                "from": "0xContract",
                "to": "0xImpl"
            }]
        });
        let patterns = detect_proxy_patterns(&trace);
        assert!(patterns.is_empty());
    }

    #[test]
    fn proxy_case_insensitive_address_matching() {
        let trace = serde_json::json!({
            "type": "CREATE",
            "from": "0xFactory",
            "to": "0xAbCdEf",
            "calls": [{
                "type": "DELEGATECALL",
                "from": "0xabcdef",
                "to": "0xImpl"
            }]
        });
        let patterns = detect_proxy_patterns(&trace);
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].proxy_address, "0xAbCdEf");
    }
}
