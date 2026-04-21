use std::{path::PathBuf, str::FromStr};

use alloy_chains::Chain;
use alloy_primitives::Address;
use anyhow::{Result, bail};
use forge_verify::{RetryArgs, VerifierArgs, VerifyArgs, provider::VerificationProviderType};
use foundry_cli::opts::{EtherscanOpts, RpcOpts};
use foundry_compilers::info::ContractInfo;
use treb_core::types::deployment::Deployment;

/// Options for building verification arguments from CLI flags.
pub struct VerifyOpts {
    pub verifier: String,
    pub verifier_url: Option<String>,
    pub contract_path: Option<String>,
    pub debug: bool,
    pub verifier_api_key: Option<String>,
    pub etherscan_api_key: Option<String>,
    pub rpc_url: Option<String>,
    pub force: bool,
    pub watch: bool,
    pub retries: u32,
    pub delay: u32,
    pub root: PathBuf,
}

/// Build a forge-verify `VerifyArgs` from a treb `Deployment` and CLI options.
pub fn build_verify_args(deployment: &Deployment, opts: &VerifyOpts) -> Result<VerifyArgs> {
    let address = Address::from_str(&deployment.address)
        .map_err(|e| anyhow::anyhow!("invalid address '{}': {e}", deployment.address))?;

    let contract_info = contract_info_from_opts(
        opts.contract_path.as_deref(),
        &deployment.artifact.path,
        &deployment.contract_name,
    );

    let constructor_args = if deployment.deployment_strategy.constructor_args.is_empty() {
        None
    } else {
        Some(deployment.deployment_strategy.constructor_args.clone())
    };

    let compiler_version = if deployment.artifact.compiler_version.is_empty() {
        None
    } else {
        Some(deployment.artifact.compiler_version.clone())
    };

    let verifier_type = match opts.verifier.to_lowercase().as_str() {
        "etherscan" => VerificationProviderType::Etherscan,
        "sourcify" => VerificationProviderType::Sourcify,
        "blockscout" => VerificationProviderType::Blockscout,
        other => bail!("unsupported verifier: {other}"),
    };

    let verifier = VerifierArgs {
        verifier: verifier_type,
        verifier_api_key: opts.verifier_api_key.clone(),
        verifier_url: opts.verifier_url.clone(),
    };

    let retry = RetryArgs { retries: opts.retries, delay: opts.delay };

    let etherscan = EtherscanOpts {
        key: opts.etherscan_api_key.clone(),
        chain: Some(Chain::from_id(deployment.chain_id)),
    };

    let rpc = RpcOpts { url: opts.rpc_url.clone(), ..Default::default() };

    Ok(VerifyArgs {
        address,
        contract: Some(contract_info),
        constructor_args,
        constructor_args_path: None,
        guess_constructor_args: false,
        creation_transaction_hash: None,
        compiler_version,
        compilation_profile: None,
        num_of_optimizations: None,
        flatten: false,
        force: opts.force,
        skip_is_verified_check: opts.force,
        watch: opts.watch,
        libraries: Vec::new(),
        root: Some(opts.root.clone()),
        show_standard_json_input: false,
        via_ir: false,
        evm_version: None,
        no_auto_detect: false,
        use_solc: None,
        etherscan,
        rpc,
        retry,
        verifier,
        language: None,
    })
}

/// Render the subset of `forge verify-contract` arguments that treb controls.
pub fn format_verify_command(args: &VerifyArgs) -> String {
    let mut command =
        vec!["forge".to_string(), "verify-contract".to_string(), args.address.to_string()];

    if let Some(contract) = &args.contract {
        command.push(contract.to_string());
    }
    if let Some(constructor_args) =
        args.constructor_args.as_deref().filter(|value| !value.is_empty())
    {
        push_flag_value(&mut command, "--constructor-args", constructor_args);
    }
    if let Some(compiler_version) = &args.compiler_version {
        push_flag_value(&mut command, "--compiler-version", compiler_version.as_str());
    }
    if args.force {
        command.push("--force".to_string());
    }
    if args.skip_is_verified_check {
        command.push("--skip-is-verified-check".to_string());
    }
    if args.watch {
        command.push("--watch".to_string());
    }
    if let Some(root) = &args.root {
        push_flag_value(&mut command, "--root", root.display().to_string());
    }
    if args.etherscan.key.is_some() {
        push_redacted_flag(&mut command, "--etherscan-api-key");
    }
    if let Some(chain) = args.etherscan.chain {
        push_flag_value(&mut command, "--chain", chain.to_string());
    }
    if let Some(rpc_url) = &args.rpc.url {
        push_flag_value(&mut command, "--rpc-url", rpc_url.as_str());
    }
    push_flag_value(&mut command, "--retries", args.retry.retries.to_string());
    push_flag_value(&mut command, "--delay", args.retry.delay.to_string());
    push_flag_value(&mut command, "--verifier", args.verifier.verifier.to_string());
    if args.verifier.verifier_api_key.is_some() {
        push_redacted_flag(&mut command, "--verifier-api-key");
    }
    if let Some(verifier_url) = &args.verifier.verifier_url {
        push_flag_value(&mut command, "--verifier-url", verifier_url.as_str());
    }

    shell_join(&command)
}

/// Parse artifact path to extract source path for ContractInfo.
///
/// Artifact path format: `out/<source_relative>/<ContractName>.json`
/// We extract `<source_relative>` as the contract source path.
fn parse_contract_info(artifact_path: &str, contract_name: &str) -> ContractInfo {
    let path = artifact_path
        .strip_prefix("out/")
        .and_then(|rest| rest.rfind('/').map(|idx| rest[..idx].to_string()));

    ContractInfo { path, name: contract_name.to_string() }
}

fn contract_info_from_opts(
    contract_path: Option<&str>,
    artifact_path: &str,
    contract_name: &str,
) -> ContractInfo {
    match contract_path {
        Some(path_override) => parse_contract_path_override(path_override, contract_name),
        None => parse_contract_info(artifact_path, contract_name),
    }
}

fn parse_contract_path_override(contract_path: &str, default_contract_name: &str) -> ContractInfo {
    let (path, name) = match contract_path.split_once(':') {
        Some((path, name)) => (path.to_string(), name.to_string()),
        None => (contract_path.to_string(), default_contract_name.to_string()),
    };

    ContractInfo { path: Some(path), name }
}

fn push_flag_value(command: &mut Vec<String>, flag: &str, value: impl Into<String>) {
    command.push(flag.to_string());
    command.push(value.into());
}

fn push_redacted_flag(command: &mut Vec<String>, flag: &str) {
    push_flag_value(command, flag, "REDACTED");
}

fn shell_join(args: &[String]) -> String {
    args.iter().map(|arg| shell_quote(arg)).collect::<Vec<_>>().join(" ")
}

fn shell_quote(arg: &str) -> String {
    if !arg.is_empty()
        && arg
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'/' | b':' | b'.' | b'_' | b'-' | b'='))
    {
        return arg.to_string();
    }

    format!("'{}'", arg.replace('\'', r#"'"'"'"#))
}

/// Build an explorer URL for a verified contract based on the chain and verifier.
///
/// Returns `None` if the chain is not recognized (e.g. a custom chain ID with
/// no known block explorer).
pub fn explorer_url(chain_id: u64, address: &str, verifier: &str) -> Option<String> {
    match verifier {
        "etherscan" | "blockscout" => {
            let chain = Chain::from_id(chain_id);
            chain
                .etherscan_urls()
                .map(|(_, browser_url)| format!("{browser_url}/address/{address}#code"))
        }
        "sourcify" => {
            Some(format!("https://repo.sourcify.dev/contracts/full_match/{chain_id}/{address}/"))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::{TimeZone, Utc};
    use treb_core::types::{
        deployment::{ArtifactInfo, Deployment, DeploymentStrategy, VerificationInfo},
        enums::{DeploymentMethod, DeploymentType, VerificationStatus},
    };

    use super::*;

    fn make_deployment() -> Deployment {
        Deployment {
            id: "production/1/Counter:v1".into(),
            namespace: "production".into(),
            chain_id: 1,
            contract_name: "Counter".into(),
            label: "v1".into(),
            address: "0x1234567890abcdef1234567890abcdef12345678".into(),
            deployment_type: DeploymentType::Singleton,
            execution: None,
            transaction_id: "tx-001".into(),
            deployment_strategy: DeploymentStrategy {
                method: DeploymentMethod::Create,
                salt: String::new(),
                init_code_hash: String::new(),
                factory: String::new(),
                constructor_args: String::new(),
                entropy: String::new(),
            },
            proxy_info: None,
            artifact: ArtifactInfo {
                path: "out/Counter.sol/Counter.json".into(),
                compiler_version: "0.8.24".into(),
                bytecode_hash: "0xabcdef".into(),
                script_path: "script/Deploy.s.sol".into(),
                git_commit: "abc1234".into(),
            },
            verification: VerificationInfo {
                status: VerificationStatus::Unverified,
                etherscan_url: String::new(),
                verified_at: None,
                reason: String::new(),
                verifiers: HashMap::new(),
            },
            tags: None,
            created_at: Utc.with_ymd_and_hms(2025, 1, 15, 10, 30, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2025, 1, 15, 10, 30, 0).unwrap(),
        }
    }

    fn default_opts() -> VerifyOpts {
        VerifyOpts {
            verifier: "etherscan".into(),
            verifier_url: None,
            contract_path: None,
            debug: false,
            verifier_api_key: None,
            etherscan_api_key: None,
            rpc_url: None,
            force: false,
            watch: false,
            retries: 5,
            delay: 5,
            root: PathBuf::from("/tmp/test-project"),
        }
    }

    #[test]
    fn maps_deployment_address() {
        let d = make_deployment();
        let args = build_verify_args(&d, &default_opts()).unwrap();
        let expected = Address::from_str("0x1234567890abcdef1234567890abcdef12345678").unwrap();
        assert_eq!(args.address, expected);
    }

    #[test]
    fn maps_contract_info_from_artifact_path() {
        let d = make_deployment();
        let args = build_verify_args(&d, &default_opts()).unwrap();
        let contract = args.contract.unwrap();
        assert_eq!(contract.name, "Counter");
        assert_eq!(contract.path, Some("Counter.sol".into()));
    }

    #[test]
    fn maps_contract_info_nested_path() {
        let mut d = make_deployment();
        d.artifact.path = "out/src/tokens/MyToken.sol/MyToken.json".into();
        d.contract_name = "MyToken".into();
        let args = build_verify_args(&d, &default_opts()).unwrap();
        let contract = args.contract.unwrap();
        assert_eq!(contract.name, "MyToken");
        assert_eq!(contract.path, Some("src/tokens/MyToken.sol".into()));
    }

    #[test]
    fn contract_path_override_replaces_artifact_contract_info() {
        let mut d = make_deployment();
        d.contract_name = "CounterProxy".into();
        let opts = VerifyOpts {
            contract_path: Some("./src/Counter.sol:Counter".into()),
            ..default_opts()
        };

        let args = build_verify_args(&d, &opts).unwrap();
        let contract = args.contract.unwrap();
        assert_eq!(contract.name, "Counter");
        assert_eq!(contract.path, Some("./src/Counter.sol".into()));
    }

    #[test]
    fn contract_path_without_colon_uses_deployment_contract_name() {
        let mut d = make_deployment();
        d.contract_name = "CounterProxy".into();
        let opts = VerifyOpts { contract_path: Some("./src/Counter.sol".into()), ..default_opts() };

        let args = build_verify_args(&d, &opts).unwrap();
        let contract = args.contract.unwrap();
        assert_eq!(contract.name, "CounterProxy");
        assert_eq!(contract.path, Some("./src/Counter.sol".into()));
    }

    #[test]
    fn falls_back_to_artifact_contract_info_when_contract_path_missing() {
        let d = make_deployment();

        let args = build_verify_args(&d, &default_opts()).unwrap();
        let contract = args.contract.unwrap();
        assert_eq!(contract.name, "Counter");
        assert_eq!(contract.path, Some("Counter.sol".into()));
    }

    #[test]
    fn maps_compiler_version() {
        let d = make_deployment();
        let args = build_verify_args(&d, &default_opts()).unwrap();
        assert_eq!(args.compiler_version, Some("0.8.24".into()));
    }

    #[test]
    fn maps_empty_compiler_version_to_none() {
        let mut d = make_deployment();
        d.artifact.compiler_version = String::new();
        let args = build_verify_args(&d, &default_opts()).unwrap();
        assert_eq!(args.compiler_version, None);
    }

    #[test]
    fn maps_constructor_args_when_present() {
        let mut d = make_deployment();
        d.deployment_strategy.constructor_args =
            "000000000000000000000000000000000000000000000000000000000000002a".into();
        let args = build_verify_args(&d, &default_opts()).unwrap();
        assert_eq!(
            args.constructor_args,
            Some("000000000000000000000000000000000000000000000000000000000000002a".into())
        );
    }

    #[test]
    fn maps_constructor_args_absent() {
        let d = make_deployment();
        let args = build_verify_args(&d, &default_opts()).unwrap();
        assert_eq!(args.constructor_args, None);
    }

    #[test]
    fn maps_verifier_etherscan() {
        let d = make_deployment();
        let opts = VerifyOpts { verifier: "etherscan".into(), ..default_opts() };
        let args = build_verify_args(&d, &opts).unwrap();
        assert_eq!(args.verifier.verifier, VerificationProviderType::Etherscan);
    }

    #[test]
    fn maps_verifier_sourcify() {
        let d = make_deployment();
        let opts = VerifyOpts { verifier: "sourcify".into(), ..default_opts() };
        let args = build_verify_args(&d, &opts).unwrap();
        assert_eq!(args.verifier.verifier, VerificationProviderType::Sourcify);
    }

    #[test]
    fn maps_verifier_blockscout() {
        let d = make_deployment();
        let opts = VerifyOpts { verifier: "blockscout".into(), ..default_opts() };
        let args = build_verify_args(&d, &opts).unwrap();
        assert_eq!(args.verifier.verifier, VerificationProviderType::Blockscout);
    }

    #[test]
    fn rejects_unsupported_verifier() {
        let d = make_deployment();
        let opts = VerifyOpts { verifier: "unknown".into(), ..default_opts() };
        let err = build_verify_args(&d, &opts).unwrap_err();
        assert!(err.to_string().contains("unsupported verifier"));
    }

    #[test]
    fn maps_retry_args() {
        let d = make_deployment();
        let opts = VerifyOpts { retries: 10, delay: 15, ..default_opts() };
        let args = build_verify_args(&d, &opts).unwrap();
        assert_eq!(args.retry.retries, 10);
        assert_eq!(args.retry.delay, 15);
    }

    #[test]
    fn maps_force_and_skip_is_verified_check() {
        let d = make_deployment();
        let opts = VerifyOpts { force: true, ..default_opts() };
        let args = build_verify_args(&d, &opts).unwrap();
        assert!(args.force);
        assert!(args.skip_is_verified_check);
    }

    #[test]
    fn force_false_does_not_skip_verified_check() {
        let d = make_deployment();
        let args = build_verify_args(&d, &default_opts()).unwrap();
        assert!(!args.force);
        assert!(!args.skip_is_verified_check);
    }

    #[test]
    fn maps_watch_flag() {
        let d = make_deployment();
        let opts = VerifyOpts { watch: true, ..default_opts() };
        let args = build_verify_args(&d, &opts).unwrap();
        assert!(args.watch);
    }

    #[test]
    fn maps_root_path() {
        let d = make_deployment();
        let opts = VerifyOpts { root: PathBuf::from("/my/project"), ..default_opts() };
        let args = build_verify_args(&d, &opts).unwrap();
        assert_eq!(args.root, Some(PathBuf::from("/my/project")));
    }

    #[test]
    fn maps_verifier_api_key_and_url() {
        let d = make_deployment();
        let opts = VerifyOpts {
            verifier_api_key: Some("my-api-key".into()),
            verifier_url: Some("https://custom-api.example.com".into()),
            ..default_opts()
        };
        let args = build_verify_args(&d, &opts).unwrap();
        assert_eq!(args.verifier.verifier_api_key, Some("my-api-key".into()));
        assert_eq!(args.verifier.verifier_url, Some("https://custom-api.example.com".into()));
    }

    #[test]
    fn formats_verify_command_with_treb_managed_flags() {
        let d = make_deployment();
        let opts = VerifyOpts {
            contract_path: Some("./src/My Counter.sol:Counter".into()),
            verifier: "blockscout".into(),
            verifier_api_key: Some("my-api-key".into()),
            verifier_url: Some("https://example.com/api".into()),
            etherscan_api_key: Some("etherscan-key".into()),
            rpc_url: Some("https://rpc.example.com".into()),
            force: true,
            watch: true,
            retries: 10,
            delay: 15,
            ..default_opts()
        };

        let args = build_verify_args(&d, &opts).unwrap();
        let command = format_verify_command(&args);

        assert!(command.starts_with(&format!(
            "forge verify-contract {} './src/My Counter.sol:Counter'",
            args.address
        )));
        assert!(!command.contains("--contract"));
        assert!(command.contains("--compiler-version 0.8.24"));
        assert!(command.contains("--force"));
        assert!(command.contains("--skip-is-verified-check"));
        assert!(command.contains("--watch"));
        assert!(command.contains("--root /tmp/test-project"));
        assert!(command.contains("--etherscan-api-key REDACTED"));
        assert!(!command.contains("etherscan-key"));
        assert!(command.contains("--chain"));
        assert!(command.contains("--rpc-url https://rpc.example.com"));
        assert!(command.contains("--retries 10"));
        assert!(command.contains("--delay 15"));
        assert!(command.contains("--verifier blockscout"));
        assert!(command.contains("--verifier-api-key REDACTED"));
        assert!(!command.contains("my-api-key"));
        assert!(command.contains("--verifier-url https://example.com/api"));
    }

    #[test]
    fn formats_verify_command_omits_unset_optional_flags() {
        let d = make_deployment();

        let args = build_verify_args(&d, &default_opts()).unwrap();
        let command = format_verify_command(&args);

        assert!(!command.contains("--rpc-url"));
        assert!(!command.contains("--watch"));
        assert!(!command.contains("--verifier-url"));
        assert!(!command.contains("--etherscan-api-key"));
    }

    #[test]
    fn maps_etherscan_api_key() {
        let d = make_deployment();
        let opts = VerifyOpts { etherscan_api_key: Some("etherscan-key".into()), ..default_opts() };
        let args = build_verify_args(&d, &opts).unwrap();
        assert_eq!(args.etherscan.key, Some("etherscan-key".into()));
    }

    #[test]
    fn maps_chain_id_to_etherscan_chain() {
        let d = make_deployment();
        let args = build_verify_args(&d, &default_opts()).unwrap();
        let chain = args.etherscan.chain.unwrap();
        assert_eq!(chain.id(), 1);
    }

    #[test]
    fn maps_rpc_url() {
        let d = make_deployment();
        let opts = VerifyOpts { rpc_url: Some("https://rpc.example.com".into()), ..default_opts() };
        let args = build_verify_args(&d, &opts).unwrap();
        assert_eq!(args.rpc.url, Some("https://rpc.example.com".into()));
    }

    #[test]
    fn invalid_address_returns_error() {
        let mut d = make_deployment();
        d.address = "not-an-address".into();
        let err = build_verify_args(&d, &default_opts()).unwrap_err();
        assert!(err.to_string().contains("invalid address"));
    }

    #[test]
    fn parse_contract_info_standard_path() {
        let info = parse_contract_info("out/Counter.sol/Counter.json", "Counter");
        assert_eq!(info.name, "Counter");
        assert_eq!(info.path, Some("Counter.sol".into()));
    }

    #[test]
    fn parse_contract_info_no_out_prefix() {
        let info = parse_contract_info("Counter.sol/Counter.json", "Counter");
        assert_eq!(info.name, "Counter");
        assert_eq!(info.path, None);
    }

    #[test]
    fn explorer_url_etherscan_mainnet() {
        let url = explorer_url(1, "0x1234", "etherscan");
        assert!(url.is_some());
        let url = url.unwrap();
        assert!(url.contains("etherscan.io"));
        assert!(url.contains("0x1234"));
        assert!(url.ends_with("#code"));
    }

    #[test]
    fn explorer_url_sourcify() {
        let url = explorer_url(1, "0x1234", "sourcify").unwrap();
        assert!(url.contains("sourcify.dev"));
        assert!(url.contains("0x1234"));
        assert!(url.contains("/1/"));
    }

    #[test]
    fn explorer_url_unknown_chain() {
        let url = explorer_url(999999999, "0x1234", "etherscan");
        // Unknown chain may not have etherscan URLs
        // This is acceptable — returns None
        assert!(url.is_none() || url.unwrap().contains("0x1234"));
    }

    #[test]
    fn explorer_url_unknown_verifier() {
        let url = explorer_url(1, "0x1234", "unknown");
        assert!(url.is_none());
    }
}
