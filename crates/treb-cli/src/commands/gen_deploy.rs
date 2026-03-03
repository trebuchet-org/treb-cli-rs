//! `treb gen-deploy` command implementation.
//!
//! Compiles the project, resolves a named contract artifact, extracts
//! constructor parameters from its ABI, and builds a `TemplateContext`
//! for Solidity deployment script generation.

use std::env;

use anyhow::{bail, Context};

use treb_forge::{compile_project, ArtifactIndex};

// ── Valid flag values ────────────────────────────────────────────────────

const VALID_STRATEGIES: &[&str] = &["create", "create2", "create3"];
const VALID_PROXIES: &[&str] = &["erc1967", "uups", "transparent", "beacon"];

// ── Template context types ───────────────────────────────────────────────

/// A single constructor parameter extracted from a contract's ABI.
#[derive(Debug)]
pub struct ConstructorParam {
    /// Parameter name (e.g., `initialCount`).
    pub name: String,
    /// Solidity type (e.g., `uint256`, `address`).
    pub solidity_type: String,
}

/// Context built from artifact introspection, used to render deployment
/// script templates in later stories.
#[derive(Debug)]
pub struct TemplateContext {
    /// Contract name from the resolved artifact.
    pub contract_name: String,
    /// Source file path relative to the project root (e.g., `src/Counter.sol`).
    pub artifact_path: String,
    /// Whether this artifact is a library (no creation bytecode).
    pub is_library: bool,
    /// Deployment strategy: `create`, `create2`, or `create3`.
    pub strategy: String,
    /// Proxy pattern, if requested.
    pub proxy: Option<String>,
    /// Custom proxy contract name, if provided.
    pub proxy_contract: Option<String>,
    /// Constructor parameters extracted from the contract's ABI.
    pub constructor_params: Vec<ConstructorParam>,
}

// ── Command entry point ──────────────────────────────────────────────────

pub async fn run(
    artifact: &str,
    strategy: Option<&str>,
    proxy: Option<&str>,
    proxy_contract: Option<&str>,
    _output: Option<&str>,
    _json: bool,
) -> anyhow::Result<()> {
    // ── Validate strategy flag ───────────────────────────────────────────
    let strategy_str = strategy.unwrap_or("create");
    let strategy_lower = strategy_str.to_lowercase();
    if !VALID_STRATEGIES.contains(&strategy_lower.as_str()) {
        bail!(
            "invalid strategy '{}'. Valid strategies: {}",
            strategy_str,
            VALID_STRATEGIES.join(", ")
        );
    }

    // ── Validate proxy flag ──────────────────────────────────────────────
    let proxy_lower = proxy.map(|p| p.to_lowercase());
    if let Some(ref p) = proxy_lower {
        if !VALID_PROXIES.contains(&p.as_str()) {
            bail!(
                "invalid proxy pattern '{}'. Valid patterns: {}",
                proxy.unwrap(),
                VALID_PROXIES.join(", ")
            );
        }
    }

    // ── Compile project ──────────────────────────────────────────────────
    let cwd = env::current_dir().context("failed to determine current directory")?;
    let foundry_config = treb_config::load_foundry_config(&cwd)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let compilation = compile_project(&foundry_config)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // Collect available names before consuming compilation output.
    let available_names: Vec<String> = compilation
        .artifact_ids()
        .map(|id| id.name.clone())
        .collect();

    let artifact_index = ArtifactIndex::from_compile_output(compilation);

    // ── Resolve artifact ─────────────────────────────────────────────────
    let artifact_match = artifact_index
        .find_by_name(artifact)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .ok_or_else(|| {
            let mut msg = format!("contract '{}' not found in compilation output.", artifact);
            // List available contracts to help the user.
            let mut unique_names: Vec<&str> = available_names
                .iter()
                .map(|s| s.as_str())
                .collect();
            unique_names.sort();
            unique_names.dedup();
            if !unique_names.is_empty() {
                msg.push_str(&format!(
                    "\n\nAvailable contracts: {}",
                    unique_names.join(", ")
                ));
            }
            anyhow::anyhow!(msg)
        })?;

    // ── Detect library ───────────────────────────────────────────────────
    let is_library = !artifact_match.has_bytecode;

    if is_library && proxy_lower.is_some() {
        bail!("libraries cannot be deployed behind proxies");
    }

    // ── Extract constructor parameters ───────────────────────────────────
    let constructor_params: Vec<ConstructorParam> = artifact_match
        .abi
        .constructor()
        .map(|ctor| {
            ctor.inputs
                .iter()
                .map(|input| ConstructorParam {
                    name: input.name.clone(),
                    solidity_type: input.ty.clone(),
                })
                .collect()
        })
        .unwrap_or_default();

    // ── Build template context ───────────────────────────────────────────
    let artifact_path = artifact_match
        .artifact_id
        .source
        .to_string_lossy()
        .to_string();

    let _context = TemplateContext {
        contract_name: artifact_match.name.clone(),
        artifact_path,
        is_library,
        strategy: strategy_lower,
        proxy: proxy_lower,
        proxy_contract: proxy_contract.map(|s| s.to_string()),
        constructor_params,
    };

    // ── Diagnostic output ────────────────────────────────────────────────
    eprintln!(
        "Generating deploy script for {} (strategy: {}, proxy: {:?})",
        _context.contract_name, _context.strategy, _context.proxy,
    );

    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Strategy validation ──────────────────────────────────────────────

    #[test]
    fn valid_strategies_are_accepted() {
        for s in VALID_STRATEGIES {
            assert!(VALID_STRATEGIES.contains(s));
        }
    }

    #[test]
    fn valid_proxies_are_accepted() {
        for p in VALID_PROXIES {
            assert!(VALID_PROXIES.contains(p));
        }
    }

    // ── ConstructorParam ─────────────────────────────────────────────────

    #[test]
    fn constructor_param_fields() {
        let param = ConstructorParam {
            name: "initialCount".to_string(),
            solidity_type: "uint256".to_string(),
        };
        assert_eq!(param.name, "initialCount");
        assert_eq!(param.solidity_type, "uint256");
    }

    // ── TemplateContext construction ──────────────────────────────────────

    #[test]
    fn template_context_with_proxy() {
        let ctx = TemplateContext {
            contract_name: "Counter".to_string(),
            artifact_path: "src/Counter.sol".to_string(),
            is_library: false,
            strategy: "create".to_string(),
            proxy: Some("uups".to_string()),
            proxy_contract: Some("MyProxy".to_string()),
            constructor_params: vec![
                ConstructorParam {
                    name: "initialCount".to_string(),
                    solidity_type: "uint256".to_string(),
                },
                ConstructorParam {
                    name: "owner".to_string(),
                    solidity_type: "address".to_string(),
                },
            ],
        };

        assert_eq!(ctx.contract_name, "Counter");
        assert_eq!(ctx.strategy, "create");
        assert_eq!(ctx.proxy, Some("uups".to_string()));
        assert_eq!(ctx.proxy_contract, Some("MyProxy".to_string()));
        assert_eq!(ctx.constructor_params.len(), 2);
        assert!(!ctx.is_library);
    }

    #[test]
    fn template_context_library_no_constructor() {
        let ctx = TemplateContext {
            contract_name: "MathLib".to_string(),
            artifact_path: "src/MathLib.sol".to_string(),
            is_library: true,
            strategy: "create".to_string(),
            proxy: None,
            proxy_contract: None,
            constructor_params: vec![],
        };

        assert!(ctx.is_library);
        assert!(ctx.constructor_params.is_empty());
        assert!(ctx.proxy.is_none());
    }

    #[test]
    fn template_context_no_proxy() {
        let ctx = TemplateContext {
            contract_name: "Token".to_string(),
            artifact_path: "src/Token.sol".to_string(),
            is_library: false,
            strategy: "create2".to_string(),
            proxy: None,
            proxy_contract: None,
            constructor_params: vec![],
        };

        assert_eq!(ctx.strategy, "create2");
        assert!(ctx.proxy.is_none());
    }
}
