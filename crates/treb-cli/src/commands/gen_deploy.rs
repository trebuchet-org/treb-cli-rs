//! `treb gen-deploy` command implementation.
//!
//! Compiles the project, resolves a named contract artifact, extracts
//! constructor parameters from its ABI, and generates a Solidity
//! deployment script using Handlebars templates.

use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context};
use handlebars::Handlebars;
use serde::Serialize;

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

/// Context built from artifact introspection.
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

// ── Render types (Handlebars + JSON) ─────────────────────────────────────

#[derive(Debug, Serialize)]
struct RenderParam {
    name: String,
    solidity_type: String,
    placeholder: String,
}

#[derive(Debug, Serialize, Default)]
struct RenderContext {
    contract_name: String,
    import_path: String,
    is_library: bool,
    has_constructor_params: bool,
    constructor_params: Vec<RenderParam>,
    constructor_args: String,
    // Proxy template fields
    is_create: bool,
    is_create2: bool,
    is_create3: bool,
    proxy_contract: String,
    has_custom_proxy: bool,
}

#[derive(Debug, Serialize)]
struct GenDeployOutput {
    contract_name: String,
    strategy: String,
    proxy: Option<String>,
    output_path: String,
    code: String,
}

// ── CREATE template ──────────────────────────────────────────────────────

const CREATE_TEMPLATE: &str = "\
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Script, console} from \"forge-std/Script.sol\";
import { {{contract_name}} } from \"{{import_path}}\";

contract Deploy{{contract_name}} is Script {
    function run() public {
        vm.startBroadcast();
{{#if is_library}}

        // Library deployment — deploy raw bytecode
        bytes memory bytecode = type({{contract_name}}).creationCode;
        address deployed;
        assembly {
            deployed := create(0, add(bytecode, 0x20), mload(bytecode))
        }
        require(deployed != address(0), \"{{contract_name}} deployment failed\");

        console.log(\"{{contract_name}} deployed at:\", deployed);
{{else}}
{{#if has_constructor_params}}

        // TODO: Set constructor arguments
{{#each constructor_params}}
        {{solidity_type}} {{name}} = {{placeholder}}; // TODO: replace placeholder
{{/each}}

        {{@root.contract_name}} deployed = new {{@root.contract_name}}({{@root.constructor_args}});
{{else}}

        {{contract_name}} deployed = new {{contract_name}}();
{{/if}}

        console.log(\"{{contract_name}} deployed at:\", address(deployed));
{{/if}}

        vm.stopBroadcast();
    }
}
";

// ── CREATE2 template ────────────────────────────────────────────────────

const CREATE2_TEMPLATE: &str = "\
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Script, console} from \"forge-std/Script.sol\";
import { {{contract_name}} } from \"{{import_path}}\";

contract Deploy{{contract_name}} is Script {
    function run() public {
        vm.startBroadcast();

        // TODO: Set your deployment salt
        bytes32 salt = bytes32(0);
{{#if is_library}}

        // Library deployment — deploy raw bytecode via CREATE2
        bytes memory bytecode = type({{contract_name}}).creationCode;
        address deployed;
        assembly {
            deployed := create2(0, add(bytecode, 0x20), mload(bytecode), salt)
        }
        require(deployed != address(0), \"{{contract_name}} deployment failed\");

        console.log(\"{{contract_name}} deployed at:\", deployed);
{{else}}
{{#if has_constructor_params}}

        // TODO: Set constructor arguments
{{#each constructor_params}}
        {{solidity_type}} {{name}} = {{placeholder}}; // TODO: replace placeholder
{{/each}}

        {{@root.contract_name}} deployed = new {{@root.contract_name}}{salt: salt}({{@root.constructor_args}});
{{else}}

        {{contract_name}} deployed = new {{contract_name}}{salt: salt}();
{{/if}}

        console.log(\"{{contract_name}} deployed at:\", address(deployed));
{{/if}}

        vm.stopBroadcast();
    }
}
";

// ── CREATE3 template ────────────────────────────────────────────────────

const CREATE3_TEMPLATE: &str = "\
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Script, console} from \"forge-std/Script.sol\";
import { {{contract_name}} } from \"{{import_path}}\";

interface ICreateX {
    function deployCreate3(bytes32 salt, bytes memory initCode) external payable returns (address);
}

contract Deploy{{contract_name}} is Script {
    // TODO: Set the CreateX factory address for your network
    // See https://github.com/pcaversaccio/createx for deployment addresses
    ICreateX constant CREATEX = ICreateX(address(0));

    function run() public {
        vm.startBroadcast();

        // TODO: Set your deployment salt
        bytes32 salt = bytes32(0);
{{#if has_constructor_params}}

        // TODO: Set constructor arguments
{{#each constructor_params}}
        {{solidity_type}} {{name}} = {{placeholder}}; // TODO: replace placeholder
{{/each}}

        bytes memory initCode = abi.encodePacked(
            type({{@root.contract_name}}).creationCode,
            abi.encode({{@root.constructor_args}})
        );
{{else}}

        bytes memory initCode = type({{contract_name}}).creationCode;
{{/if}}

        address deployed = CREATEX.deployCreate3(salt, initCode);

        console.log(\"{{contract_name}} deployed at:\", deployed);

        vm.stopBroadcast();
    }
}
";

// ── ERC1967 proxy template ──────────────────────────────────────────────

const ERC1967_PROXY_TEMPLATE: &str = "\
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Script, console} from \"forge-std/Script.sol\";
import { {{contract_name}} } from \"{{import_path}}\";
{{#if has_custom_proxy}}
// TODO: Import your custom proxy contract: {{proxy_contract}}
{{else}}
import { ERC1967Proxy } from \"@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol\";
{{/if}}
{{#if is_create3}}

interface ICreateX {
    function deployCreate3(bytes32 salt, bytes memory initCode) external payable returns (address);
}
{{/if}}

contract Deploy{{contract_name}} is Script {
{{#if is_create3}}
    // TODO: Set the CreateX factory address for your network
    // See https://github.com/pcaversaccio/createx for deployment addresses
    ICreateX constant CREATEX = ICreateX(address(0));

{{/if}}
    function run() public {
        vm.startBroadcast();
{{#if is_create2}}

        // TODO: Set your deployment salt
        bytes32 salt = bytes32(0);
{{/if}}
{{#if is_create3}}

        // TODO: Set your deployment salt
        bytes32 salt = bytes32(0);
{{/if}}
{{#if has_constructor_params}}

        // TODO: Set constructor arguments
{{#each constructor_params}}
        {{solidity_type}} {{name}} = {{placeholder}}; // TODO: replace placeholder
{{/each}}
{{/if}}

        // Deploy implementation
{{#if is_create}}
{{#if has_constructor_params}}
        {{contract_name}} implementation = new {{contract_name}}({{constructor_args}});
{{else}}
        {{contract_name}} implementation = new {{contract_name}}();
{{/if}}
{{/if}}
{{#if is_create2}}
{{#if has_constructor_params}}
        {{contract_name}} implementation = new {{contract_name}}{salt: salt}({{constructor_args}});
{{else}}
        {{contract_name}} implementation = new {{contract_name}}{salt: salt}();
{{/if}}
{{/if}}
{{#if is_create3}}
{{#if has_constructor_params}}
        bytes memory implInitCode = abi.encodePacked(
            type({{contract_name}}).creationCode,
            abi.encode({{constructor_args}})
        );
{{else}}
        bytes memory implInitCode = type({{contract_name}}).creationCode;
{{/if}}
        address implAddr = CREATEX.deployCreate3(salt, implInitCode);
        {{contract_name}} implementation = {{contract_name}}(implAddr);
{{/if}}

        // TODO: Set initialization data for the proxy.
        // If your contract uses an initializer, encode the call here.
        // Use initData = \"\" if no initialization is needed.
        bytes memory initData = abi.encodeWithSelector(
            {{contract_name}}.initialize.selector
            // TODO: Add initialize function arguments
        );

        // Deploy ERC1967 proxy
        {{proxy_contract}} proxy = new {{proxy_contract}}(
            address(implementation),
            initData
        );

        console.log(\"{{contract_name}} implementation deployed at:\", address(implementation));
        console.log(\"{{proxy_contract}} deployed at:\", address(proxy));

        vm.stopBroadcast();
    }
}
";

// ── UUPS proxy template ────────────────────────────────────────────────

const UUPS_PROXY_TEMPLATE: &str = "\
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Script, console} from \"forge-std/Script.sol\";
import { {{contract_name}} } from \"{{import_path}}\";
{{#if has_custom_proxy}}
// TODO: Import your custom proxy contract: {{proxy_contract}}
{{else}}
import { ERC1967Proxy } from \"@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol\";
{{/if}}
{{#if is_create3}}

interface ICreateX {
    function deployCreate3(bytes32 salt, bytes memory initCode) external payable returns (address);
}
{{/if}}

// NOTE: {{contract_name}} must inherit UUPSUpgradeable and override _authorizeUpgrade.
contract Deploy{{contract_name}} is Script {
{{#if is_create3}}
    // TODO: Set the CreateX factory address for your network
    // See https://github.com/pcaversaccio/createx for deployment addresses
    ICreateX constant CREATEX = ICreateX(address(0));

{{/if}}
    function run() public {
        vm.startBroadcast();
{{#if is_create2}}

        // TODO: Set your deployment salt
        bytes32 salt = bytes32(0);
{{/if}}
{{#if is_create3}}

        // TODO: Set your deployment salt
        bytes32 salt = bytes32(0);
{{/if}}
{{#if has_constructor_params}}

        // TODO: Set constructor arguments
{{#each constructor_params}}
        {{solidity_type}} {{name}} = {{placeholder}}; // TODO: replace placeholder
{{/each}}
{{/if}}

        // Deploy implementation (must inherit UUPSUpgradeable)
{{#if is_create}}
{{#if has_constructor_params}}
        {{contract_name}} implementation = new {{contract_name}}({{constructor_args}});
{{else}}
        {{contract_name}} implementation = new {{contract_name}}();
{{/if}}
{{/if}}
{{#if is_create2}}
{{#if has_constructor_params}}
        {{contract_name}} implementation = new {{contract_name}}{salt: salt}({{constructor_args}});
{{else}}
        {{contract_name}} implementation = new {{contract_name}}{salt: salt}();
{{/if}}
{{/if}}
{{#if is_create3}}
{{#if has_constructor_params}}
        bytes memory implInitCode = abi.encodePacked(
            type({{contract_name}}).creationCode,
            abi.encode({{constructor_args}})
        );
{{else}}
        bytes memory implInitCode = type({{contract_name}}).creationCode;
{{/if}}
        address implAddr = CREATEX.deployCreate3(salt, implInitCode);
        {{contract_name}} implementation = {{contract_name}}(implAddr);
{{/if}}

        // TODO: Set initialization data for the proxy
        bytes memory initData = abi.encodeWithSelector(
            {{contract_name}}.initialize.selector
            // TODO: Add initialize function arguments
        );

        // Deploy UUPS proxy (uses ERC1967Proxy)
        {{proxy_contract}} proxy = new {{proxy_contract}}(
            address(implementation),
            initData
        );

        console.log(\"{{contract_name}} implementation deployed at:\", address(implementation));
        console.log(\"{{proxy_contract}} deployed at:\", address(proxy));

        vm.stopBroadcast();
    }
}
";

// ── Transparent proxy template ─────────────────────────────────────────

const TRANSPARENT_PROXY_TEMPLATE: &str = "\
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Script, console} from \"forge-std/Script.sol\";
import { {{contract_name}} } from \"{{import_path}}\";
{{#if has_custom_proxy}}
// TODO: Import your custom proxy contract: {{proxy_contract}}
{{else}}
import { TransparentUpgradeableProxy } from \"@openzeppelin/contracts/proxy/transparent/TransparentUpgradeableProxy.sol\";
{{/if}}
{{#if is_create3}}

interface ICreateX {
    function deployCreate3(bytes32 salt, bytes memory initCode) external payable returns (address);
}
{{/if}}

contract Deploy{{contract_name}} is Script {
{{#if is_create3}}
    // TODO: Set the CreateX factory address for your network
    // See https://github.com/pcaversaccio/createx for deployment addresses
    ICreateX constant CREATEX = ICreateX(address(0));

{{/if}}
    function run() public {
        vm.startBroadcast();
{{#if is_create2}}

        // TODO: Set your deployment salt
        bytes32 salt = bytes32(0);
{{/if}}
{{#if is_create3}}

        // TODO: Set your deployment salt
        bytes32 salt = bytes32(0);
{{/if}}
{{#if has_constructor_params}}

        // TODO: Set constructor arguments
{{#each constructor_params}}
        {{solidity_type}} {{name}} = {{placeholder}}; // TODO: replace placeholder
{{/each}}
{{/if}}

        // Deploy implementation
{{#if is_create}}
{{#if has_constructor_params}}
        {{contract_name}} implementation = new {{contract_name}}({{constructor_args}});
{{else}}
        {{contract_name}} implementation = new {{contract_name}}();
{{/if}}
{{/if}}
{{#if is_create2}}
{{#if has_constructor_params}}
        {{contract_name}} implementation = new {{contract_name}}{salt: salt}({{constructor_args}});
{{else}}
        {{contract_name}} implementation = new {{contract_name}}{salt: salt}();
{{/if}}
{{/if}}
{{#if is_create3}}
{{#if has_constructor_params}}
        bytes memory implInitCode = abi.encodePacked(
            type({{contract_name}}).creationCode,
            abi.encode({{constructor_args}})
        );
{{else}}
        bytes memory implInitCode = type({{contract_name}}).creationCode;
{{/if}}
        address implAddr = CREATEX.deployCreate3(salt, implInitCode);
        {{contract_name}} implementation = {{contract_name}}(implAddr);
{{/if}}

        // TODO: Set the proxy admin address
        address proxyAdmin = msg.sender; // TODO: replace with actual admin

        // TODO: Set initialization data for the proxy
        bytes memory initData = abi.encodeWithSelector(
            {{contract_name}}.initialize.selector
            // TODO: Add initialize function arguments
        );

        // Deploy transparent proxy
        {{proxy_contract}} proxy = new {{proxy_contract}}(
            address(implementation),
            proxyAdmin,
            initData
        );

        console.log(\"{{contract_name}} implementation deployed at:\", address(implementation));
        console.log(\"{{proxy_contract}} deployed at:\", address(proxy));

        vm.stopBroadcast();
    }
}
";

// ── Beacon proxy template ──────────────────────────────────────────────

const BEACON_PROXY_TEMPLATE: &str = "\
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Script, console} from \"forge-std/Script.sol\";
import { {{contract_name}} } from \"{{import_path}}\";
import { UpgradeableBeacon } from \"@openzeppelin/contracts/proxy/beacon/UpgradeableBeacon.sol\";
{{#if has_custom_proxy}}
// TODO: Import your custom proxy contract: {{proxy_contract}}
{{else}}
import { BeaconProxy } from \"@openzeppelin/contracts/proxy/beacon/BeaconProxy.sol\";
{{/if}}
{{#if is_create3}}

interface ICreateX {
    function deployCreate3(bytes32 salt, bytes memory initCode) external payable returns (address);
}
{{/if}}

contract Deploy{{contract_name}} is Script {
{{#if is_create3}}
    // TODO: Set the CreateX factory address for your network
    // See https://github.com/pcaversaccio/createx for deployment addresses
    ICreateX constant CREATEX = ICreateX(address(0));

{{/if}}
    function run() public {
        vm.startBroadcast();
{{#if is_create2}}

        // TODO: Set your deployment salt
        bytes32 salt = bytes32(0);
{{/if}}
{{#if is_create3}}

        // TODO: Set your deployment salt
        bytes32 salt = bytes32(0);
{{/if}}
{{#if has_constructor_params}}

        // TODO: Set constructor arguments
{{#each constructor_params}}
        {{solidity_type}} {{name}} = {{placeholder}}; // TODO: replace placeholder
{{/each}}
{{/if}}

        // Deploy implementation
{{#if is_create}}
{{#if has_constructor_params}}
        {{contract_name}} implementation = new {{contract_name}}({{constructor_args}});
{{else}}
        {{contract_name}} implementation = new {{contract_name}}();
{{/if}}
{{/if}}
{{#if is_create2}}
{{#if has_constructor_params}}
        {{contract_name}} implementation = new {{contract_name}}{salt: salt}({{constructor_args}});
{{else}}
        {{contract_name}} implementation = new {{contract_name}}{salt: salt}();
{{/if}}
{{/if}}
{{#if is_create3}}
{{#if has_constructor_params}}
        bytes memory implInitCode = abi.encodePacked(
            type({{contract_name}}).creationCode,
            abi.encode({{constructor_args}})
        );
{{else}}
        bytes memory implInitCode = type({{contract_name}}).creationCode;
{{/if}}
        address implAddr = CREATEX.deployCreate3(salt, implInitCode);
        {{contract_name}} implementation = {{contract_name}}(implAddr);
{{/if}}

        // TODO: Set the beacon owner address
        address beaconOwner = msg.sender; // TODO: replace with actual owner

        // Deploy upgradeable beacon
        UpgradeableBeacon beacon = new UpgradeableBeacon(
            address(implementation),
            beaconOwner
        );

        // TODO: Set initialization data for the proxy
        bytes memory initData = abi.encodeWithSelector(
            {{contract_name}}.initialize.selector
            // TODO: Add initialize function arguments
        );

        // Deploy beacon proxy
        {{proxy_contract}} proxy = new {{proxy_contract}}(
            address(beacon),
            initData
        );

        console.log(\"{{contract_name}} implementation deployed at:\", address(implementation));
        console.log(\"UpgradeableBeacon deployed at:\", address(beacon));
        console.log(\"{{proxy_contract}} deployed at:\", address(proxy));

        vm.stopBroadcast();
    }
}
";

// ── Helpers ──────────────────────────────────────────────────────────────

/// Return the default proxy contract name for a given proxy pattern.
fn default_proxy_contract(proxy: &str) -> &str {
    match proxy {
        "erc1967" | "uups" => "ERC1967Proxy",
        "transparent" => "TransparentUpgradeableProxy",
        "beacon" => "BeaconProxy",
        _ => "",
    }
}

/// Return a Solidity placeholder literal for the given type.
fn placeholder_for_type(solidity_type: &str) -> String {
    match solidity_type {
        t if t.starts_with("uint") || t.starts_with("int") => "0".to_string(),
        "address" => "address(0)".to_string(),
        "bool" => "false".to_string(),
        "string" => "\"\"".to_string(),
        "bytes" => "\"\"".to_string(),
        t if t.starts_with("bytes") => format!("{t}(0)"),
        _ => "/* TODO */".to_string(),
    }
}

/// Compute a Solidity-compatible relative import path from one file to another.
///
/// Both paths must be relative to the project root.
fn relative_import_path(from_file: &Path, to_file: &Path) -> String {
    let from_dir = from_file.parent().unwrap_or(Path::new(""));
    let from_components: Vec<_> = from_dir.components().collect();
    let to_components: Vec<_> = to_file.components().collect();

    let common_len = from_components
        .iter()
        .zip(to_components.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let mut parts: Vec<&str> = Vec::new();

    // Go up from the source directory.
    for _ in 0..(from_components.len() - common_len) {
        parts.push("..");
    }

    // Then descend to the target.
    for comp in &to_components[common_len..] {
        if let Component::Normal(s) = comp {
            parts.push(s.to_str().unwrap());
        }
    }

    let joined = parts.join("/");
    if joined.starts_with("..") {
        joined
    } else {
        format!("./{joined}")
    }
}

// ── Command entry point ──────────────────────────────────────────────────

pub async fn run(
    artifact: &str,
    strategy: Option<&str>,
    proxy: Option<&str>,
    proxy_contract: Option<&str>,
    output: Option<&str>,
    json: bool,
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

    let context = TemplateContext {
        contract_name: artifact_match.name.clone(),
        artifact_path,
        is_library,
        strategy: strategy_lower,
        proxy: proxy_lower,
        proxy_contract: proxy_contract.map(|s| s.to_string()),
        constructor_params,
    };

    // ── Determine output path ────────────────────────────────────────────
    let output_path = output
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(format!("script/Deploy{}.s.sol", context.contract_name))
        });

    // ── Build render context ─────────────────────────────────────────────
    let import_path =
        relative_import_path(&output_path, Path::new(&context.artifact_path));

    let has_constructor_params = !context.constructor_params.is_empty();
    let constructor_args = context
        .constructor_params
        .iter()
        .map(|p| p.name.clone())
        .collect::<Vec<_>>()
        .join(", ");

    let render_params: Vec<RenderParam> = context
        .constructor_params
        .iter()
        .map(|p| RenderParam {
            name: p.name.clone(),
            solidity_type: p.solidity_type.clone(),
            placeholder: placeholder_for_type(&p.solidity_type),
        })
        .collect();

    let proxy_contract_name = match &context.proxy_contract {
        Some(name) => name.clone(),
        None => context
            .proxy
            .as_deref()
            .map(default_proxy_contract)
            .unwrap_or("")
            .to_string(),
    };

    let render_ctx = RenderContext {
        contract_name: context.contract_name.clone(),
        import_path,
        is_library: context.is_library,
        has_constructor_params,
        constructor_params: render_params,
        constructor_args,
        is_create: context.strategy == "create",
        is_create2: context.strategy == "create2",
        is_create3: context.strategy == "create3",
        proxy_contract: proxy_contract_name,
        has_custom_proxy: context.proxy_contract.is_some(),
    };

    // ── Render template ──────────────────────────────────────────────────
    let mut hbs = Handlebars::new();
    hbs.register_escape_fn(handlebars::no_escape);
    hbs.set_strict_mode(true);
    hbs.register_template_string("create", CREATE_TEMPLATE)
        .context("failed to register CREATE template")?;
    hbs.register_template_string("create2", CREATE2_TEMPLATE)
        .context("failed to register CREATE2 template")?;
    hbs.register_template_string("create3", CREATE3_TEMPLATE)
        .context("failed to register CREATE3 template")?;
    hbs.register_template_string("proxy_erc1967", ERC1967_PROXY_TEMPLATE)
        .context("failed to register ERC1967 proxy template")?;
    hbs.register_template_string("proxy_uups", UUPS_PROXY_TEMPLATE)
        .context("failed to register UUPS proxy template")?;
    hbs.register_template_string("proxy_transparent", TRANSPARENT_PROXY_TEMPLATE)
        .context("failed to register transparent proxy template")?;
    hbs.register_template_string("proxy_beacon", BEACON_PROXY_TEMPLATE)
        .context("failed to register beacon proxy template")?;

    let template_name = match &context.proxy {
        Some(proxy) => format!("proxy_{proxy}"),
        None => context.strategy.clone(),
    };

    let code = hbs
        .render(&template_name, &render_ctx)
        .context("failed to render deployment script")?;

    // ── Output ───────────────────────────────────────────────────────────
    if json {
        let json_output = GenDeployOutput {
            contract_name: context.contract_name,
            strategy: context.strategy,
            proxy: context.proxy,
            output_path: output_path.to_string_lossy().to_string(),
            code,
        };
        crate::output::print_json(&json_output)?;
    } else {
        if let Some(parent) = output_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("failed to create directory: {}", parent.display())
                })?;
            }
        }
        fs::write(&output_path, &code).with_context(|| {
            format!("failed to write deploy script: {}", output_path.display())
        })?;
        eprintln!(
            "Generated deploy script: {}",
            output_path.display()
        );
    }

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

    // ── placeholder_for_type ─────────────────────────────────────────────

    #[test]
    fn placeholder_uint256() {
        assert_eq!(placeholder_for_type("uint256"), "0");
    }

    #[test]
    fn placeholder_int128() {
        assert_eq!(placeholder_for_type("int128"), "0");
    }

    #[test]
    fn placeholder_address() {
        assert_eq!(placeholder_for_type("address"), "address(0)");
    }

    #[test]
    fn placeholder_bool() {
        assert_eq!(placeholder_for_type("bool"), "false");
    }

    #[test]
    fn placeholder_string() {
        assert_eq!(placeholder_for_type("string"), "\"\"");
    }

    #[test]
    fn placeholder_bytes() {
        assert_eq!(placeholder_for_type("bytes"), "\"\"");
    }

    #[test]
    fn placeholder_bytes32() {
        assert_eq!(placeholder_for_type("bytes32"), "bytes32(0)");
    }

    #[test]
    fn placeholder_unknown_type() {
        assert_eq!(placeholder_for_type("tuple"), "/* TODO */");
    }

    // ── relative_import_path ─────────────────────────────────────────────

    #[test]
    fn import_path_script_to_src() {
        let from = Path::new("script/DeployCounter.s.sol");
        let to = Path::new("src/Counter.sol");
        assert_eq!(relative_import_path(from, to), "../src/Counter.sol");
    }

    #[test]
    fn import_path_root_to_src() {
        let from = Path::new("Deploy.s.sol");
        let to = Path::new("src/Counter.sol");
        assert_eq!(relative_import_path(from, to), "./src/Counter.sol");
    }

    #[test]
    fn import_path_nested_to_src() {
        let from = Path::new("script/nested/Deploy.s.sol");
        let to = Path::new("src/Counter.sol");
        assert_eq!(relative_import_path(from, to), "../../src/Counter.sol");
    }

    #[test]
    fn import_path_same_dir() {
        let from = Path::new("src/Deploy.s.sol");
        let to = Path::new("src/Counter.sol");
        assert_eq!(relative_import_path(from, to), "./Counter.sol");
    }

    // ── Template rendering ───────────────────────────────────────────────

    fn render_create(ctx: &RenderContext) -> String {
        let mut hbs = Handlebars::new();
        hbs.register_escape_fn(handlebars::no_escape);
        hbs.set_strict_mode(true);
        hbs.register_template_string("create", CREATE_TEMPLATE).unwrap();
        hbs.render("create", ctx).unwrap()
    }

    #[test]
    fn render_contract_with_constructor() {
        let ctx = RenderContext {
            contract_name: "Counter".to_string(),
            import_path: "../src/Counter.sol".to_string(),
            is_library: false,
            has_constructor_params: true,
            constructor_params: vec![
                RenderParam {
                    name: "initialCount".to_string(),
                    solidity_type: "uint256".to_string(),
                    placeholder: "0".to_string(),
                },
                RenderParam {
                    name: "owner".to_string(),
                    solidity_type: "address".to_string(),
                    placeholder: "address(0)".to_string(),
                },
            ],
            constructor_args: "initialCount, owner".to_string(),
            ..Default::default()
        };

        let code = render_create(&ctx);
        assert!(code.contains("// SPDX-License-Identifier: UNLICENSED"));
        assert!(code.contains("pragma solidity ^0.8.13;"));
        assert!(code.contains("import {Script, console} from \"forge-std/Script.sol\";"));
        assert!(code.contains("import { Counter } from \"../src/Counter.sol\";"));
        assert!(code.contains("contract DeployCounter is Script"));
        assert!(code.contains("uint256 initialCount = 0;"));
        assert!(code.contains("address owner = address(0);"));
        assert!(code.contains("Counter deployed = new Counter(initialCount, owner);"));
        assert!(code.contains("console.log(\"Counter deployed at:\", address(deployed));"));
    }

    #[test]
    fn render_contract_no_constructor() {
        let ctx = RenderContext {
            contract_name: "SimpleContract".to_string(),
            import_path: "../src/SimpleContract.sol".to_string(),
            is_library: false,
            has_constructor_params: false,
            constructor_params: vec![],
            constructor_args: String::new(),
            ..Default::default()
        };

        let code = render_create(&ctx);
        assert!(code.contains("SimpleContract deployed = new SimpleContract();"));
        assert!(!code.contains("TODO: Set constructor"));
    }

    #[test]
    fn render_library() {
        let ctx = RenderContext {
            contract_name: "MathLib".to_string(),
            import_path: "../src/MathLib.sol".to_string(),
            is_library: true,
            has_constructor_params: false,
            constructor_params: vec![],
            constructor_args: String::new(),
            ..Default::default()
        };

        let code = render_create(&ctx);
        assert!(code.contains("type(MathLib).creationCode"));
        assert!(code.contains("deployed := create(0,"));
        assert!(code.contains("console.log(\"MathLib deployed at:\", deployed);"));
        assert!(!code.contains("new MathLib"));
    }

    #[test]
    fn render_has_spdx_pragma_imports() {
        let ctx = RenderContext {
            contract_name: "Token".to_string(),
            import_path: "../src/Token.sol".to_string(),
            is_library: false,
            has_constructor_params: false,
            constructor_params: vec![],
            constructor_args: String::new(),
            ..Default::default()
        };

        let code = render_create(&ctx);
        // First line should be SPDX
        assert!(code.starts_with("// SPDX-License-Identifier: UNLICENSED\n"));
        // Should have pragma
        assert!(code.contains("pragma solidity ^0.8.13;"));
        // Should have forge-std import
        assert!(code.contains("import {Script, console} from \"forge-std/Script.sol\";"));
        // Should inherit from Script
        assert!(code.contains("is Script"));
        // Should have vm.startBroadcast/stopBroadcast
        assert!(code.contains("vm.startBroadcast();"));
        assert!(code.contains("vm.stopBroadcast();"));
    }

    // ── CREATE2 template rendering ──────────────────────────────────────

    fn render_create2(ctx: &RenderContext) -> String {
        let mut hbs = Handlebars::new();
        hbs.register_escape_fn(handlebars::no_escape);
        hbs.set_strict_mode(true);
        hbs.register_template_string("create2", CREATE2_TEMPLATE).unwrap();
        hbs.render("create2", ctx).unwrap()
    }

    #[test]
    fn render_create2_with_constructor() {
        let ctx = RenderContext {
            contract_name: "Counter".to_string(),
            import_path: "../src/Counter.sol".to_string(),
            is_library: false,
            has_constructor_params: true,
            constructor_params: vec![
                RenderParam {
                    name: "initialCount".to_string(),
                    solidity_type: "uint256".to_string(),
                    placeholder: "0".to_string(),
                },
                RenderParam {
                    name: "owner".to_string(),
                    solidity_type: "address".to_string(),
                    placeholder: "address(0)".to_string(),
                },
            ],
            constructor_args: "initialCount, owner".to_string(),
            ..Default::default()
        };

        let code = render_create2(&ctx);
        assert!(code.contains("// SPDX-License-Identifier: UNLICENSED"));
        assert!(code.contains("pragma solidity ^0.8.13;"));
        assert!(code.contains("import { Counter } from \"../src/Counter.sol\";"));
        assert!(code.contains("contract DeployCounter is Script"));
        assert!(code.contains("bytes32 salt = bytes32(0);"));
        assert!(code.contains("// TODO: Set your deployment salt"));
        assert!(code.contains("uint256 initialCount = 0;"));
        assert!(code.contains("address owner = address(0);"));
        assert!(code.contains("Counter deployed = new Counter{salt: salt}(initialCount, owner);"));
        assert!(code.contains("console.log(\"Counter deployed at:\", address(deployed));"));
    }

    #[test]
    fn render_create2_no_constructor() {
        let ctx = RenderContext {
            contract_name: "SimpleContract".to_string(),
            import_path: "../src/SimpleContract.sol".to_string(),
            is_library: false,
            has_constructor_params: false,
            constructor_params: vec![],
            constructor_args: String::new(),
            ..Default::default()
        };

        let code = render_create2(&ctx);
        assert!(code.contains("bytes32 salt = bytes32(0);"));
        assert!(code.contains("SimpleContract deployed = new SimpleContract{salt: salt}();"));
        assert!(!code.contains("TODO: Set constructor"));
    }

    #[test]
    fn render_create2_library() {
        let ctx = RenderContext {
            contract_name: "MathLib".to_string(),
            import_path: "../src/MathLib.sol".to_string(),
            is_library: true,
            has_constructor_params: false,
            constructor_params: vec![],
            constructor_args: String::new(),
            ..Default::default()
        };

        let code = render_create2(&ctx);
        assert!(code.contains("bytes32 salt = bytes32(0);"));
        assert!(code.contains("type(MathLib).creationCode"));
        assert!(code.contains("deployed := create2(0,"));
        assert!(code.contains("console.log(\"MathLib deployed at:\", deployed);"));
        assert!(!code.contains("new MathLib"));
    }

    // ── CREATE3 template rendering ──────────────────────────────────────

    fn render_create3(ctx: &RenderContext) -> String {
        let mut hbs = Handlebars::new();
        hbs.register_escape_fn(handlebars::no_escape);
        hbs.set_strict_mode(true);
        hbs.register_template_string("create3", CREATE3_TEMPLATE).unwrap();
        hbs.render("create3", ctx).unwrap()
    }

    #[test]
    fn render_create3_with_constructor() {
        let ctx = RenderContext {
            contract_name: "Counter".to_string(),
            import_path: "../src/Counter.sol".to_string(),
            is_library: false,
            has_constructor_params: true,
            constructor_params: vec![
                RenderParam {
                    name: "initialCount".to_string(),
                    solidity_type: "uint256".to_string(),
                    placeholder: "0".to_string(),
                },
            ],
            constructor_args: "initialCount".to_string(),
            ..Default::default()
        };

        let code = render_create3(&ctx);
        assert!(code.contains("// SPDX-License-Identifier: UNLICENSED"));
        assert!(code.contains("pragma solidity ^0.8.13;"));
        assert!(code.contains("import { Counter } from \"../src/Counter.sol\";"));
        assert!(code.contains("interface ICreateX"));
        assert!(code.contains("deployCreate3(bytes32 salt, bytes memory initCode)"));
        assert!(code.contains("ICreateX constant CREATEX = ICreateX(address(0));"));
        assert!(code.contains("// TODO: Set the CreateX factory address"));
        assert!(code.contains("bytes32 salt = bytes32(0);"));
        assert!(code.contains("uint256 initialCount = 0;"));
        assert!(code.contains("type(Counter).creationCode"));
        assert!(code.contains("abi.encode(initialCount)"));
        assert!(code.contains("CREATEX.deployCreate3(salt, initCode)"));
        assert!(code.contains("console.log(\"Counter deployed at:\", deployed);"));
    }

    #[test]
    fn render_create3_no_constructor() {
        let ctx = RenderContext {
            contract_name: "Token".to_string(),
            import_path: "../src/Token.sol".to_string(),
            is_library: false,
            has_constructor_params: false,
            constructor_params: vec![],
            constructor_args: String::new(),
            ..Default::default()
        };

        let code = render_create3(&ctx);
        assert!(code.contains("bytes memory initCode = type(Token).creationCode;"));
        assert!(code.contains("CREATEX.deployCreate3(salt, initCode)"));
        assert!(!code.contains("abi.encode"));
        assert!(!code.contains("abi.encodePacked"));
        assert!(!code.contains("TODO: Set constructor"));
    }

    #[test]
    fn render_create3_library() {
        let ctx = RenderContext {
            contract_name: "MathLib".to_string(),
            import_path: "../src/MathLib.sol".to_string(),
            is_library: true,
            has_constructor_params: false,
            constructor_params: vec![],
            constructor_args: String::new(),
            ..Default::default()
        };

        let code = render_create3(&ctx);
        assert!(code.contains("bytes memory initCode = type(MathLib).creationCode;"));
        assert!(code.contains("CREATEX.deployCreate3(salt, initCode)"));
        assert!(!code.contains("abi.encode"));
    }

    #[test]
    fn render_create3_has_todo_comments() {
        let ctx = RenderContext {
            contract_name: "Counter".to_string(),
            import_path: "../src/Counter.sol".to_string(),
            is_library: false,
            has_constructor_params: false,
            constructor_params: vec![],
            constructor_args: String::new(),
            ..Default::default()
        };

        let code = render_create3(&ctx);
        assert!(code.contains("// TODO: Set the CreateX factory address"));
        assert!(code.contains("// TODO: Set your deployment salt"));
    }

    #[test]
    fn render_create2_has_todo_comments() {
        let ctx = RenderContext {
            contract_name: "Counter".to_string(),
            import_path: "../src/Counter.sol".to_string(),
            is_library: false,
            has_constructor_params: false,
            constructor_params: vec![],
            constructor_args: String::new(),
            ..Default::default()
        };

        let code = render_create2(&ctx);
        assert!(code.contains("// TODO: Set your deployment salt"));
    }

    // ── default_proxy_contract ────────────────────────────────────────────

    #[test]
    fn default_proxy_contract_erc1967() {
        assert_eq!(default_proxy_contract("erc1967"), "ERC1967Proxy");
    }

    #[test]
    fn default_proxy_contract_uups() {
        assert_eq!(default_proxy_contract("uups"), "ERC1967Proxy");
    }

    #[test]
    fn default_proxy_contract_transparent() {
        assert_eq!(default_proxy_contract("transparent"), "TransparentUpgradeableProxy");
    }

    #[test]
    fn default_proxy_contract_beacon() {
        assert_eq!(default_proxy_contract("beacon"), "BeaconProxy");
    }

    // ── Proxy template rendering helpers ────────────────────────────────

    fn render_proxy(template_name: &str, template: &str, ctx: &RenderContext) -> String {
        let mut hbs = Handlebars::new();
        hbs.register_escape_fn(handlebars::no_escape);
        hbs.set_strict_mode(true);
        hbs.register_template_string(template_name, template).unwrap();
        hbs.render(template_name, ctx).unwrap()
    }

    // ── ERC1967 proxy template rendering ────────────────────────────────

    #[test]
    fn render_erc1967_create_with_constructor() {
        let ctx = RenderContext {
            contract_name: "Counter".to_string(),
            import_path: "../src/Counter.sol".to_string(),
            has_constructor_params: true,
            constructor_params: vec![
                RenderParam {
                    name: "initialCount".to_string(),
                    solidity_type: "uint256".to_string(),
                    placeholder: "0".to_string(),
                },
            ],
            constructor_args: "initialCount".to_string(),
            is_create: true,
            proxy_contract: "ERC1967Proxy".to_string(),
            ..Default::default()
        };

        let code = render_proxy("proxy_erc1967", ERC1967_PROXY_TEMPLATE, &ctx);
        assert!(code.contains("import { Counter } from \"../src/Counter.sol\";"));
        assert!(code.contains("import { ERC1967Proxy } from \"@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol\";"));
        assert!(code.contains("contract DeployCounter is Script"));
        assert!(code.contains("uint256 initialCount = 0;"));
        assert!(code.contains("Counter implementation = new Counter(initialCount);"));
        assert!(code.contains("Counter.initialize.selector"));
        assert!(code.contains("ERC1967Proxy proxy = new ERC1967Proxy("));
        assert!(code.contains("address(implementation),"));
        assert!(code.contains("console.log(\"Counter implementation deployed at:\", address(implementation));"));
        assert!(code.contains("console.log(\"ERC1967Proxy deployed at:\", address(proxy));"));
    }

    #[test]
    fn render_erc1967_create_no_constructor() {
        let ctx = RenderContext {
            contract_name: "Token".to_string(),
            import_path: "../src/Token.sol".to_string(),
            is_create: true,
            proxy_contract: "ERC1967Proxy".to_string(),
            ..Default::default()
        };

        let code = render_proxy("proxy_erc1967", ERC1967_PROXY_TEMPLATE, &ctx);
        assert!(code.contains("Token implementation = new Token();"));
        assert!(!code.contains("TODO: Set constructor arguments"));
        assert!(code.contains("ERC1967Proxy proxy = new ERC1967Proxy("));
    }

    #[test]
    fn render_erc1967_create2() {
        let ctx = RenderContext {
            contract_name: "Counter".to_string(),
            import_path: "../src/Counter.sol".to_string(),
            is_create2: true,
            proxy_contract: "ERC1967Proxy".to_string(),
            ..Default::default()
        };

        let code = render_proxy("proxy_erc1967", ERC1967_PROXY_TEMPLATE, &ctx);
        assert!(code.contains("bytes32 salt = bytes32(0);"));
        assert!(code.contains("// TODO: Set your deployment salt"));
        assert!(code.contains("Counter implementation = new Counter{salt: salt}();"));
        assert!(code.contains("ERC1967Proxy proxy = new ERC1967Proxy("));
        assert!(!code.contains("ICreateX"));
    }

    #[test]
    fn render_erc1967_create3() {
        let ctx = RenderContext {
            contract_name: "Counter".to_string(),
            import_path: "../src/Counter.sol".to_string(),
            is_create3: true,
            proxy_contract: "ERC1967Proxy".to_string(),
            ..Default::default()
        };

        let code = render_proxy("proxy_erc1967", ERC1967_PROXY_TEMPLATE, &ctx);
        assert!(code.contains("interface ICreateX"));
        assert!(code.contains("ICreateX constant CREATEX = ICreateX(address(0));"));
        assert!(code.contains("bytes32 salt = bytes32(0);"));
        assert!(code.contains("type(Counter).creationCode"));
        assert!(code.contains("CREATEX.deployCreate3(salt, implInitCode)"));
        assert!(code.contains("Counter implementation = Counter(implAddr);"));
        assert!(code.contains("ERC1967Proxy proxy = new ERC1967Proxy("));
    }

    #[test]
    fn render_erc1967_custom_proxy_contract() {
        let ctx = RenderContext {
            contract_name: "Counter".to_string(),
            import_path: "../src/Counter.sol".to_string(),
            is_create: true,
            proxy_contract: "MyProxy".to_string(),
            has_custom_proxy: true,
            ..Default::default()
        };

        let code = render_proxy("proxy_erc1967", ERC1967_PROXY_TEMPLATE, &ctx);
        assert!(code.contains("// TODO: Import your custom proxy contract: MyProxy"));
        assert!(!code.contains("@openzeppelin"));
        assert!(code.contains("MyProxy proxy = new MyProxy("));
        assert!(code.contains("console.log(\"MyProxy deployed at:\", address(proxy));"));
    }

    // ── UUPS proxy template rendering ───────────────────────────────────

    #[test]
    fn render_uups_create_no_constructor() {
        let ctx = RenderContext {
            contract_name: "Token".to_string(),
            import_path: "../src/Token.sol".to_string(),
            is_create: true,
            proxy_contract: "ERC1967Proxy".to_string(),
            ..Default::default()
        };

        let code = render_proxy("proxy_uups", UUPS_PROXY_TEMPLATE, &ctx);
        assert!(code.contains("// NOTE: Token must inherit UUPSUpgradeable and override _authorizeUpgrade."));
        assert!(code.contains("Deploy implementation (must inherit UUPSUpgradeable)"));
        assert!(code.contains("Token implementation = new Token();"));
        assert!(code.contains("Deploy UUPS proxy (uses ERC1967Proxy)"));
        assert!(code.contains("ERC1967Proxy proxy = new ERC1967Proxy("));
        assert!(code.contains("import { ERC1967Proxy }"));
    }

    #[test]
    fn render_uups_create2_with_constructor() {
        let ctx = RenderContext {
            contract_name: "Counter".to_string(),
            import_path: "../src/Counter.sol".to_string(),
            has_constructor_params: true,
            constructor_params: vec![
                RenderParam {
                    name: "owner".to_string(),
                    solidity_type: "address".to_string(),
                    placeholder: "address(0)".to_string(),
                },
            ],
            constructor_args: "owner".to_string(),
            is_create2: true,
            proxy_contract: "ERC1967Proxy".to_string(),
            ..Default::default()
        };

        let code = render_proxy("proxy_uups", UUPS_PROXY_TEMPLATE, &ctx);
        assert!(code.contains("bytes32 salt = bytes32(0);"));
        assert!(code.contains("address owner = address(0);"));
        assert!(code.contains("Counter implementation = new Counter{salt: salt}(owner);"));
        assert!(code.contains("ERC1967Proxy proxy = new ERC1967Proxy("));
    }

    // ── Transparent proxy template rendering ────────────────────────────

    #[test]
    fn render_transparent_create_no_constructor() {
        let ctx = RenderContext {
            contract_name: "Token".to_string(),
            import_path: "../src/Token.sol".to_string(),
            is_create: true,
            proxy_contract: "TransparentUpgradeableProxy".to_string(),
            ..Default::default()
        };

        let code = render_proxy("proxy_transparent", TRANSPARENT_PROXY_TEMPLATE, &ctx);
        assert!(code.contains("import { TransparentUpgradeableProxy } from \"@openzeppelin/contracts/proxy/transparent/TransparentUpgradeableProxy.sol\";"));
        assert!(code.contains("Token implementation = new Token();"));
        assert!(code.contains("address proxyAdmin = msg.sender;"));
        assert!(code.contains("// TODO: Set the proxy admin address"));
        assert!(code.contains("TransparentUpgradeableProxy proxy = new TransparentUpgradeableProxy("));
        assert!(code.contains("proxyAdmin,"));
    }

    #[test]
    fn render_transparent_create2_with_constructor() {
        let ctx = RenderContext {
            contract_name: "Counter".to_string(),
            import_path: "../src/Counter.sol".to_string(),
            has_constructor_params: true,
            constructor_params: vec![
                RenderParam {
                    name: "initialCount".to_string(),
                    solidity_type: "uint256".to_string(),
                    placeholder: "0".to_string(),
                },
            ],
            constructor_args: "initialCount".to_string(),
            is_create2: true,
            proxy_contract: "TransparentUpgradeableProxy".to_string(),
            ..Default::default()
        };

        let code = render_proxy("proxy_transparent", TRANSPARENT_PROXY_TEMPLATE, &ctx);
        assert!(code.contains("bytes32 salt = bytes32(0);"));
        assert!(code.contains("uint256 initialCount = 0;"));
        assert!(code.contains("Counter implementation = new Counter{salt: salt}(initialCount);"));
        assert!(code.contains("TransparentUpgradeableProxy proxy = new TransparentUpgradeableProxy("));
        assert!(code.contains("proxyAdmin,"));
    }

    #[test]
    fn render_transparent_custom_proxy_contract() {
        let ctx = RenderContext {
            contract_name: "Counter".to_string(),
            import_path: "../src/Counter.sol".to_string(),
            is_create: true,
            proxy_contract: "MyProxy".to_string(),
            has_custom_proxy: true,
            ..Default::default()
        };

        let code = render_proxy("proxy_transparent", TRANSPARENT_PROXY_TEMPLATE, &ctx);
        assert!(code.contains("// TODO: Import your custom proxy contract: MyProxy"));
        assert!(!code.contains("TransparentUpgradeableProxy"));
        assert!(code.contains("MyProxy proxy = new MyProxy("));
    }

    // ── Beacon proxy template rendering ─────────────────────────────────

    #[test]
    fn render_beacon_create_no_constructor() {
        let ctx = RenderContext {
            contract_name: "Token".to_string(),
            import_path: "../src/Token.sol".to_string(),
            is_create: true,
            proxy_contract: "BeaconProxy".to_string(),
            ..Default::default()
        };

        let code = render_proxy("proxy_beacon", BEACON_PROXY_TEMPLATE, &ctx);
        assert!(code.contains("import { UpgradeableBeacon } from \"@openzeppelin/contracts/proxy/beacon/UpgradeableBeacon.sol\";"));
        assert!(code.contains("import { BeaconProxy } from \"@openzeppelin/contracts/proxy/beacon/BeaconProxy.sol\";"));
        assert!(code.contains("Token implementation = new Token();"));
        assert!(code.contains("address beaconOwner = msg.sender;"));
        assert!(code.contains("// TODO: Set the beacon owner address"));
        assert!(code.contains("UpgradeableBeacon beacon = new UpgradeableBeacon("));
        assert!(code.contains("address(implementation),"));
        assert!(code.contains("beaconOwner"));
        assert!(code.contains("BeaconProxy proxy = new BeaconProxy("));
        assert!(code.contains("address(beacon),"));
        assert!(code.contains("console.log(\"UpgradeableBeacon deployed at:\", address(beacon));"));
        assert!(code.contains("console.log(\"BeaconProxy deployed at:\", address(proxy));"));
    }

    #[test]
    fn render_beacon_create2_with_constructor() {
        let ctx = RenderContext {
            contract_name: "Counter".to_string(),
            import_path: "../src/Counter.sol".to_string(),
            has_constructor_params: true,
            constructor_params: vec![
                RenderParam {
                    name: "owner".to_string(),
                    solidity_type: "address".to_string(),
                    placeholder: "address(0)".to_string(),
                },
            ],
            constructor_args: "owner".to_string(),
            is_create2: true,
            proxy_contract: "BeaconProxy".to_string(),
            ..Default::default()
        };

        let code = render_proxy("proxy_beacon", BEACON_PROXY_TEMPLATE, &ctx);
        assert!(code.contains("bytes32 salt = bytes32(0);"));
        assert!(code.contains("address owner = address(0);"));
        assert!(code.contains("Counter implementation = new Counter{salt: salt}(owner);"));
        assert!(code.contains("UpgradeableBeacon beacon = new UpgradeableBeacon("));
        assert!(code.contains("BeaconProxy proxy = new BeaconProxy("));
    }

    #[test]
    fn render_beacon_create3() {
        let ctx = RenderContext {
            contract_name: "Token".to_string(),
            import_path: "../src/Token.sol".to_string(),
            is_create3: true,
            proxy_contract: "BeaconProxy".to_string(),
            ..Default::default()
        };

        let code = render_proxy("proxy_beacon", BEACON_PROXY_TEMPLATE, &ctx);
        assert!(code.contains("interface ICreateX"));
        assert!(code.contains("ICreateX constant CREATEX = ICreateX(address(0));"));
        assert!(code.contains("CREATEX.deployCreate3(salt, implInitCode)"));
        assert!(code.contains("Token implementation = Token(implAddr);"));
        assert!(code.contains("UpgradeableBeacon beacon = new UpgradeableBeacon("));
        assert!(code.contains("BeaconProxy proxy = new BeaconProxy("));
    }

    #[test]
    fn render_beacon_custom_proxy_contract() {
        let ctx = RenderContext {
            contract_name: "Counter".to_string(),
            import_path: "../src/Counter.sol".to_string(),
            is_create: true,
            proxy_contract: "MyBeaconProxy".to_string(),
            has_custom_proxy: true,
            ..Default::default()
        };

        let code = render_proxy("proxy_beacon", BEACON_PROXY_TEMPLATE, &ctx);
        assert!(code.contains("import { UpgradeableBeacon }"));
        assert!(code.contains("// TODO: Import your custom proxy contract: MyBeaconProxy"));
        assert!(!code.contains("import { BeaconProxy }"));
        assert!(code.contains("MyBeaconProxy proxy = new MyBeaconProxy("));
    }

    // ── Strategy + proxy composition ────────────────────────────────────

    #[test]
    fn render_create2_uups_composition() {
        let ctx = RenderContext {
            contract_name: "Counter".to_string(),
            import_path: "../src/Counter.sol".to_string(),
            has_constructor_params: true,
            constructor_params: vec![
                RenderParam {
                    name: "initialCount".to_string(),
                    solidity_type: "uint256".to_string(),
                    placeholder: "0".to_string(),
                },
            ],
            constructor_args: "initialCount".to_string(),
            is_create2: true,
            proxy_contract: "ERC1967Proxy".to_string(),
            ..Default::default()
        };

        let code = render_proxy("proxy_uups", UUPS_PROXY_TEMPLATE, &ctx);
        // CREATE2 strategy
        assert!(code.contains("bytes32 salt = bytes32(0);"));
        assert!(code.contains("Counter implementation = new Counter{salt: salt}(initialCount);"));
        // UUPS proxy
        assert!(code.contains("must inherit UUPSUpgradeable"));
        assert!(code.contains("ERC1967Proxy proxy = new ERC1967Proxy("));
    }

    #[test]
    fn render_create3_transparent_composition() {
        let ctx = RenderContext {
            contract_name: "Token".to_string(),
            import_path: "../src/Token.sol".to_string(),
            is_create3: true,
            proxy_contract: "TransparentUpgradeableProxy".to_string(),
            ..Default::default()
        };

        let code = render_proxy("proxy_transparent", TRANSPARENT_PROXY_TEMPLATE, &ctx);
        // CREATE3 strategy
        assert!(code.contains("interface ICreateX"));
        assert!(code.contains("CREATEX.deployCreate3(salt, implInitCode)"));
        assert!(code.contains("Token implementation = Token(implAddr);"));
        // Transparent proxy
        assert!(code.contains("address proxyAdmin = msg.sender;"));
        assert!(code.contains("TransparentUpgradeableProxy proxy = new TransparentUpgradeableProxy("));
        assert!(code.contains("proxyAdmin,"));
    }

    // ── GenDeployOutput serialization ────────────────────────────────────

    #[test]
    fn gen_deploy_output_serializes_to_json() {
        let output = GenDeployOutput {
            contract_name: "Counter".to_string(),
            strategy: "create".to_string(),
            proxy: None,
            output_path: "script/DeployCounter.s.sol".to_string(),
            code: "// generated code".to_string(),
        };

        let json = serde_json::to_value(&output).unwrap();
        assert_eq!(json["contract_name"], "Counter");
        assert_eq!(json["strategy"], "create");
        assert!(json["proxy"].is_null());
        assert_eq!(json["output_path"], "script/DeployCounter.s.sol");
        assert_eq!(json["code"], "// generated code");
    }

    #[test]
    fn gen_deploy_output_with_proxy() {
        let output = GenDeployOutput {
            contract_name: "Token".to_string(),
            strategy: "create2".to_string(),
            proxy: Some("uups".to_string()),
            output_path: "script/DeployToken.s.sol".to_string(),
            code: "// code".to_string(),
        };

        let json = serde_json::to_value(&output).unwrap();
        assert_eq!(json["proxy"], "uups");
    }
}
