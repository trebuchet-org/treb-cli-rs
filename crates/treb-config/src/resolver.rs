//! Namespace resolution and layered config merger.
//!
//! Provides `resolve_namespace_v2` for walking dot-separated namespace
//! hierarchies and `resolve_config` for merging all config sources
//! into a single `ResolvedConfig`.

use std::{collections::HashMap, path::PathBuf};

use treb_core::error::{Result, TrebError};

use crate::{
    ResolvedConfig, SenderConfig, TrebConfigFormat, TrebFileConfigV2, convert_v1_to_resolved,
    detect_treb_config_format, extract_treb_senders_from_foundry, load_dotenv, load_local_config,
    load_treb_config_v1, load_treb_config_v2,
};

/// Options for resolving the full layered configuration.
pub struct ResolveOpts {
    /// Root directory of the project.
    pub project_root: PathBuf,
    /// CLI override for namespace.
    pub namespace: Option<String>,
    /// CLI override for network.
    pub network: Option<String>,
    /// CLI override for foundry profile.
    pub profile: Option<String>,
    /// CLI sender overrides (highest precedence).
    pub sender_overrides: HashMap<String, SenderConfig>,
}

/// Resolve v2 namespace hierarchy by walking dot-separated segments.
///
/// For namespace `"production.ntt"`, walks:
/// 1. `"default"` — base senders and profile
/// 2. `"production"` — overrides from parent
/// 3. `"production.ntt"` — most specific overrides
///
/// At each level, the namespace's profile overrides the previous, and
/// sender role→account mappings are merged (later levels win). After
/// accumulation, role names are resolved to `SenderConfig` by looking
/// up account names in `config.accounts`.
///
/// Returns error if the namespace (and all its non-default ancestors)
/// are not found in the config.
pub fn resolve_namespace_v2(
    config: &TrebFileConfigV2,
    namespace: &str,
) -> Result<(String, HashMap<String, SenderConfig>)> {
    // Build hierarchy: always "default", then each dot-prefix.
    let mut levels = vec!["default".to_string()];
    if namespace != "default" {
        let parts: Vec<&str> = namespace.split('.').collect();
        for i in 1..=parts.len() {
            levels.push(parts[..i].join("."));
        }
    }

    // Walk the hierarchy, accumulating profile and role→account mappings.
    let mut found_non_default = namespace == "default";
    let mut profile = String::new();
    let mut role_to_account: HashMap<String, String> = HashMap::new();

    for level in &levels {
        if let Some(ns_roles) = config.namespace.get(level) {
            if level != "default" {
                found_non_default = true;
            }
            if let Some(ref p) = ns_roles.profile {
                profile = p.clone();
            }
            for (role, account_name) in &ns_roles.senders {
                role_to_account.insert(role.clone(), account_name.clone());
            }
        }
    }

    if !found_non_default {
        let mut available: Vec<&String> = config.namespace.keys().collect();
        available.sort();
        return Err(TrebError::Config(format!(
            "namespace '{}' not found; available namespaces: {}",
            namespace,
            available.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
        )));
    }

    // Resolve role → account name → SenderConfig.
    let mut senders = HashMap::new();
    for (role, account_name) in &role_to_account {
        if let Some(account) = config.accounts.get(account_name) {
            senders.insert(role.clone(), account.clone());
        } else {
            // Account referenced but not defined — include empty config;
            // validation (US-007) will catch this later.
            senders.insert(role.clone(), SenderConfig::default());
        }
    }

    Ok((profile, senders))
}

/// Resolve the full layered configuration from all sources.
///
/// Precedence (highest first):
/// 1. CLI overrides (namespace, network, profile, sender_overrides)
/// 2. treb.toml (v2 or v1) — namespace-resolved senders
/// 3. foundry.toml `[profile.*.treb.senders.*]` — fallback when no treb.toml
/// 4. `.treb/config.local.json` — default namespace/network
/// 5. `.env` / `.env.local` — loaded first for env var expansion
pub fn resolve_config(opts: ResolveOpts) -> Result<ResolvedConfig> {
    // 1. Load environment variables (needed for env var expansion in config).
    load_dotenv(&opts.project_root);

    // 2. Load local config for default namespace/network.
    let local = load_local_config(&opts.project_root)?;

    // 3. Determine namespace and network (CLI > local > defaults).
    let namespace = opts.namespace.unwrap_or(local.namespace);
    let network =
        opts.network.or(if local.network.is_empty() { None } else { Some(local.network) });

    // 4. Detect treb.toml format and resolve.
    let format = detect_treb_config_format(&opts.project_root);
    let treb_path = opts.project_root.join("treb.toml");

    let (mut profile, mut senders, slow, fork_setup, mut config_source) = match format {
        TrebConfigFormat::V2 => {
            let config = load_treb_config_v2(&treb_path)?;
            let (p, s) = resolve_namespace_v2(&config, &namespace)?;
            let fork = config.fork.setup.clone();
            (p, s, false, fork, "treb.toml (v2)".to_string())
        }
        TrebConfigFormat::V1 => {
            let config = load_treb_config_v1(&treb_path)?;
            let resolved = convert_v1_to_resolved(&config, &namespace);
            (resolved.profile, resolved.senders, resolved.slow, None, "treb.toml (v1)".to_string())
        }
        TrebConfigFormat::None => {
            (String::new(), HashMap::new(), false, None, "defaults".to_string())
        }
    };

    // 5. Fallback to foundry.toml senders if no treb.toml found.
    if senders.is_empty() && format == TrebConfigFormat::None {
        let foundry_profile = if profile.is_empty() { "default" } else { &profile };
        let foundry_senders =
            extract_treb_senders_from_foundry(&opts.project_root, foundry_profile);
        if !foundry_senders.is_empty() {
            senders = foundry_senders;
            config_source = "foundry.toml".to_string();
        }
    }

    // 6. Apply CLI overrides (highest precedence).
    if let Some(p) = opts.profile {
        profile = p;
    }
    for (role, sender) in opts.sender_overrides {
        senders.insert(role, sender);
    }

    Ok(ResolvedConfig {
        namespace,
        network,
        profile,
        senders,
        slow,
        fork_setup,
        config_source,
        project_root: opts.project_root,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NamespaceRoles, SenderType};
    use tempfile::TempDir;

    fn make_v2_config() -> TrebFileConfigV2 {
        let mut config = TrebFileConfigV2::default();

        // Accounts.
        config.accounts.insert(
            "deployer".to_string(),
            SenderConfig {
                type_: Some(SenderType::PrivateKey),
                address: Some("0xDeployerAddr".to_string()),
                private_key: Some("0xDeployerKey".to_string()),
                ..Default::default()
            },
        );
        config.accounts.insert(
            "ledger_signer".to_string(),
            SenderConfig {
                type_: Some(SenderType::Ledger),
                address: Some("0xLedgerAddr".to_string()),
                derivation_path: Some("m/44'/60'/0'/0/0".to_string()),
                ..Default::default()
            },
        );
        config.accounts.insert(
            "multisig".to_string(),
            SenderConfig {
                type_: Some(SenderType::Safe),
                safe: Some("0xSafeContract".to_string()),
                signer: Some("deployer".to_string()),
                ..Default::default()
            },
        );

        // default namespace: deployer -> deployer, profile="default".
        let mut default_ns =
            NamespaceRoles { profile: Some("default".to_string()), senders: HashMap::new() };
        default_ns.senders.insert("deployer".to_string(), "deployer".to_string());
        config.namespace.insert("default".to_string(), default_ns);

        // production namespace: deployer -> ledger_signer, admin -> multisig.
        let mut production_ns =
            NamespaceRoles { profile: Some("optimized".to_string()), senders: HashMap::new() };
        production_ns.senders.insert("deployer".to_string(), "ledger_signer".to_string());
        production_ns.senders.insert("admin".to_string(), "multisig".to_string());
        config.namespace.insert("production".to_string(), production_ns);

        // production.ntt: deployer -> deployer (override back), governance -> multisig.
        let mut ntt_ns = NamespaceRoles {
            profile: None, // inherits from production
            senders: HashMap::new(),
        };
        ntt_ns.senders.insert("deployer".to_string(), "deployer".to_string());
        ntt_ns.senders.insert("governance".to_string(), "multisig".to_string());
        config.namespace.insert("production.ntt".to_string(), ntt_ns);

        config
    }

    // ---- resolve_namespace_v2 ----

    #[test]
    fn resolve_v2_default_namespace() {
        let config = make_v2_config();
        let (profile, senders) = resolve_namespace_v2(&config, "default").unwrap();

        assert_eq!(profile, "default");
        assert_eq!(senders.len(), 1);
        assert_eq!(senders["deployer"].type_, Some(SenderType::PrivateKey));
    }

    #[test]
    fn resolve_v2_two_level_namespace() {
        let config = make_v2_config();
        let (profile, senders) = resolve_namespace_v2(&config, "production").unwrap();

        assert_eq!(profile, "optimized");
        // deployer (from default, overridden by production) + admin (from production).
        assert_eq!(senders.len(), 2);
        assert_eq!(senders["deployer"].type_, Some(SenderType::Ledger));
        assert_eq!(senders["admin"].type_, Some(SenderType::Safe));
    }

    #[test]
    fn resolve_v2_three_level_deep() {
        let config = make_v2_config();
        let (profile, senders) = resolve_namespace_v2(&config, "production.ntt").unwrap();

        // Profile inherited from "production" (production.ntt has None).
        assert_eq!(profile, "optimized");

        // Senders accumulated and overridden:
        //   deployer: default=deployer -> production=ledger_signer -> ntt=deployer
        //   admin: production=multisig (inherited)
        //   governance: ntt=multisig (new)
        assert_eq!(senders.len(), 3);
        assert_eq!(senders["deployer"].type_, Some(SenderType::PrivateKey));
        assert_eq!(senders["admin"].type_, Some(SenderType::Safe));
        assert_eq!(senders["governance"].type_, Some(SenderType::Safe));
    }

    #[test]
    fn resolve_v2_role_to_account_mapping() {
        let config = make_v2_config();
        let (_, senders) = resolve_namespace_v2(&config, "production").unwrap();

        // "deployer" role -> "ledger_signer" account -> Ledger config.
        assert_eq!(senders["deployer"].type_, Some(SenderType::Ledger));
        assert_eq!(senders["deployer"].address, Some("0xLedgerAddr".to_string()));
        assert_eq!(senders["deployer"].derivation_path, Some("m/44'/60'/0'/0/0".to_string()));

        // "admin" role -> "multisig" account -> Safe config.
        assert_eq!(senders["admin"].type_, Some(SenderType::Safe));
        assert_eq!(senders["admin"].safe, Some("0xSafeContract".to_string()));
    }

    #[test]
    fn resolve_v2_missing_namespace_errors_with_available() {
        let config = make_v2_config();
        let err = resolve_namespace_v2(&config, "staging").unwrap_err();
        let msg = err.to_string();

        assert!(msg.contains("staging"), "error should mention requested namespace: {msg}");
        assert!(msg.contains("not found"), "error should say 'not found': {msg}");
        // Should list available namespaces.
        assert!(msg.contains("default"), "error should list available namespaces: {msg}");
        assert!(msg.contains("production"), "error should list available namespaces: {msg}");
    }

    #[test]
    fn resolve_v2_undefined_account_returns_default_sender() {
        let mut config = make_v2_config();
        // Add a namespace that references a non-existent account.
        let mut ns = NamespaceRoles { profile: None, senders: HashMap::new() };
        ns.senders.insert("deployer".to_string(), "nonexistent_account".to_string());
        config.namespace.insert("test".to_string(), ns);

        let (_, senders) = resolve_namespace_v2(&config, "test").unwrap();
        // The sender exists but has default (empty) config.
        assert_eq!(senders["deployer"], SenderConfig::default());
    }

    // ---- resolve_config (full integration) ----

    #[test]
    fn resolve_config_v2_with_local_config_and_env() {
        let tmp = TempDir::new().unwrap();

        // .env with a private key variable.
        std::fs::write(tmp.path().join(".env"), "TREB_TEST_RESOLVE_KEY_US006=0xFromEnv\n").unwrap();

        // treb.toml v2 referencing env var.
        std::fs::write(
            tmp.path().join("treb.toml"),
            r#"
[accounts.deployer]
type = "private_key"
address = "0xDeployerAddr"
private_key = "${TREB_TEST_RESOLVE_KEY_US006}"

[namespace.default]
profile = "default"

[namespace.default.senders]
deployer = "deployer"

[fork]
setup = "script/ForkSetup.s.sol"
"#,
        )
        .unwrap();

        // Local config with network selection.
        std::fs::create_dir_all(tmp.path().join(".treb")).unwrap();
        std::fs::write(
            tmp.path().join(".treb/config.local.json"),
            r#"{"namespace":"default","network":"sepolia"}"#,
        )
        .unwrap();

        let resolved = resolve_config(ResolveOpts {
            project_root: tmp.path().to_path_buf(),
            namespace: None,
            network: None,
            profile: None,
            sender_overrides: HashMap::new(),
        })
        .unwrap();

        assert_eq!(resolved.namespace, "default");
        assert_eq!(resolved.network, Some("sepolia".to_string()));
        assert_eq!(resolved.profile, "default");
        assert_eq!(resolved.senders.len(), 1);
        assert_eq!(resolved.senders["deployer"].type_, Some(SenderType::PrivateKey));
        // Env var expanded from .env file.
        assert_eq!(resolved.senders["deployer"].private_key, Some("0xFromEnv".to_string()));
        assert_eq!(resolved.fork_setup, Some("script/ForkSetup.s.sol".to_string()));
        assert_eq!(resolved.config_source, "treb.toml (v2)");
        assert!(!resolved.slow);

        // SAFETY: test uses a unique env var name, no other threads access it.
        unsafe { std::env::remove_var("TREB_TEST_RESOLVE_KEY_US006") };
    }

    #[test]
    fn resolve_config_fallback_to_foundry() {
        let tmp = TempDir::new().unwrap();

        // No treb.toml — only foundry.toml with treb senders.
        std::fs::write(
            tmp.path().join("foundry.toml"),
            r#"
[profile.default]
src = "src"
out = "out"

[profile.default.treb.senders.deployer]
type = "private_key"
address = "0xFoundryDeployer"
"#,
        )
        .unwrap();

        let resolved = resolve_config(ResolveOpts {
            project_root: tmp.path().to_path_buf(),
            namespace: None,
            network: None,
            profile: None,
            sender_overrides: HashMap::new(),
        })
        .unwrap();

        assert_eq!(resolved.senders.len(), 1);
        assert_eq!(resolved.senders["deployer"].address.as_deref(), Some("0xFoundryDeployer"));
        assert_eq!(resolved.config_source, "foundry.toml");
    }

    #[test]
    fn resolve_config_cli_overrides_take_precedence() {
        let tmp = TempDir::new().unwrap();

        // treb.toml v2 with default and production namespaces.
        std::fs::write(
            tmp.path().join("treb.toml"),
            r#"
[accounts.deployer]
type = "private_key"
address = "0xDeployerAddr"

[namespace.default]
profile = "default"

[namespace.default.senders]
deployer = "deployer"

[namespace.production]
profile = "optimized"

[namespace.production.senders]
deployer = "deployer"
"#,
        )
        .unwrap();

        // Local config with namespace=default, network=mainnet.
        std::fs::create_dir_all(tmp.path().join(".treb")).unwrap();
        std::fs::write(
            tmp.path().join(".treb/config.local.json"),
            r#"{"namespace":"default","network":"mainnet"}"#,
        )
        .unwrap();

        let mut cli_senders = HashMap::new();
        cli_senders.insert(
            "extra".to_string(),
            SenderConfig {
                type_: Some(SenderType::Ledger),
                address: Some("0xCLIAddr".to_string()),
                ..Default::default()
            },
        );

        let resolved = resolve_config(ResolveOpts {
            project_root: tmp.path().to_path_buf(),
            namespace: Some("production".to_string()),
            network: Some("sepolia".to_string()),
            profile: Some("ci".to_string()),
            sender_overrides: cli_senders,
        })
        .unwrap();

        // CLI namespace overrides local config.
        assert_eq!(resolved.namespace, "production");
        // CLI network overrides local config.
        assert_eq!(resolved.network, Some("sepolia".to_string()));
        // CLI profile overrides treb.toml.
        assert_eq!(resolved.profile, "ci");
        // deployer from treb.toml + extra from CLI.
        assert_eq!(resolved.senders.len(), 2);
        assert!(resolved.senders.contains_key("deployer"));
        assert_eq!(resolved.senders["extra"].type_, Some(SenderType::Ledger));
        assert_eq!(resolved.senders["extra"].address.as_deref(), Some("0xCLIAddr"));
    }

    #[test]
    fn resolve_config_missing_namespace_propagates_error() {
        let tmp = TempDir::new().unwrap();

        std::fs::write(
            tmp.path().join("treb.toml"),
            r#"
[accounts.deployer]
type = "private_key"

[namespace.default.senders]
deployer = "deployer"
"#,
        )
        .unwrap();

        let err = resolve_config(ResolveOpts {
            project_root: tmp.path().to_path_buf(),
            namespace: Some("staging".to_string()),
            network: None,
            profile: None,
            sender_overrides: HashMap::new(),
        })
        .unwrap_err();

        let msg = err.to_string();
        assert!(
            msg.contains("staging") && msg.contains("not found"),
            "expected namespace error: {msg}"
        );
    }
}
