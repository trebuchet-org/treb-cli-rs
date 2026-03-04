//! Golden-file integration tests for `treb gen-deploy`.
//!
//! Tests exercise CREATE, CREATE2, CREATE3 strategies, library deployment,
//! no-constructor contracts, JSON output, proxy variants (ERC1967, UUPS,
//! Transparent, Beacon), strategy+proxy combinations, custom proxy override,
//! and error paths using the `gen-deploy-project` fixture which contains
//! Counter (with constructor), SimpleContract (no constructor), and MathLib
//! (library).

mod framework;

use framework::context::TestContext;
use framework::integration_test::{run_integration_test, IntegrationTest};
use framework::normalizer::{CompilerOutputNormalizer, PathNormalizer};

/// CREATE strategy — contract with constructor (Counter).
///
/// Verifies generated Solidity contains `new Counter(initialCount, _owner)`,
/// constructor placeholder scaffolding, and proper import path.
#[test]
fn gen_deploy_create() {
    let ctx = TestContext::new("gen-deploy-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("gen_deploy_create")
        .test(&[
            "gen-deploy",
            "Counter",
            "--strategy",
            "create",
            "--output",
            "script/DeployCounter.s.sol",
        ])
        .output_artifact("script/DeployCounter.s.sol")
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(CompilerOutputNormalizer));

    run_integration_test(&test, &ctx);
}

/// CREATE strategy — contract without constructor (SimpleContract).
///
/// Verifies generated script uses simple `new SimpleContract()` without
/// constructor argument scaffolding.
#[test]
fn gen_deploy_create_no_constructor() {
    let ctx = TestContext::new("gen-deploy-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("gen_deploy_create_no_constructor")
        .test(&[
            "gen-deploy",
            "SimpleContract",
            "--strategy",
            "create",
            "--output",
            "script/DeploySimpleContract.s.sol",
        ])
        .output_artifact("script/DeploySimpleContract.s.sol")
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(CompilerOutputNormalizer));

    run_integration_test(&test, &ctx);
}

/// CREATE strategy — library deployment (MathLib).
///
/// Verifies generated script uses raw bytecode assembly pattern for library
/// deployment instead of `new` operator.
#[test]
fn gen_deploy_create_library() {
    let ctx = TestContext::new("gen-deploy-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("gen_deploy_create_library")
        .test(&[
            "gen-deploy",
            "MathLib",
            "--strategy",
            "create",
            "--output",
            "script/DeployMathLib.s.sol",
        ])
        .output_artifact("script/DeployMathLib.s.sol")
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(CompilerOutputNormalizer));

    run_integration_test(&test, &ctx);
}

/// CREATE2 strategy — salt-based deployment (Counter).
///
/// Verifies generated Solidity contains `bytes32 salt`, `{salt: salt}`
/// syntax, and constructor argument scaffolding.
#[test]
fn gen_deploy_create2() {
    let ctx = TestContext::new("gen-deploy-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("gen_deploy_create2")
        .test(&[
            "gen-deploy",
            "Counter",
            "--strategy",
            "create2",
            "--output",
            "script/DeployCounter_create2.s.sol",
        ])
        .output_artifact("script/DeployCounter_create2.s.sol")
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(CompilerOutputNormalizer));

    run_integration_test(&test, &ctx);
}

/// CREATE3 strategy — ICreateX.deployCreate3() (Counter).
///
/// Verifies generated Solidity contains ICreateX interface, CREATEX constant,
/// `deployCreate3(salt, initCode)` call, and constructor encoding.
#[test]
fn gen_deploy_create3() {
    let ctx = TestContext::new("gen-deploy-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("gen_deploy_create3")
        .test(&[
            "gen-deploy",
            "Counter",
            "--strategy",
            "create3",
            "--output",
            "script/DeployCounter_create3.s.sol",
        ])
        .output_artifact("script/DeployCounter_create3.s.sol")
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(CompilerOutputNormalizer));

    run_integration_test(&test, &ctx);
}

/// JSON output mode — valid JSON with expected fields.
///
/// Verifies JSON output contains contract_name, strategy, proxy, output_path,
/// and code keys. No file is written in JSON mode.
#[test]
fn gen_deploy_json() {
    let ctx = TestContext::new("gen-deploy-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("gen_deploy_json")
        .test(&["gen-deploy", "Counter", "--json"])
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(CompilerOutputNormalizer));

    run_integration_test(&test, &ctx);
}

// ── Proxy variant tests ──────────────────────────────────────────────────

/// ERC1967 proxy — standard upgradeable proxy (Counter).
///
/// Verifies generated Solidity imports ERC1967Proxy from OpenZeppelin,
/// deploys implementation + proxy with initialization data.
#[test]
fn gen_deploy_erc1967() {
    let ctx = TestContext::new("gen-deploy-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("gen_deploy_erc1967")
        .test(&[
            "gen-deploy",
            "Counter",
            "--proxy",
            "erc1967",
            "--output",
            "script/DeployCounter_erc1967.s.sol",
        ])
        .output_artifact("script/DeployCounter_erc1967.s.sol")
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(CompilerOutputNormalizer));

    run_integration_test(&test, &ctx);
}

/// UUPS proxy — UUPSUpgradeable pattern (Counter).
///
/// Verifies generated Solidity contains UUPSUpgradeable note,
/// ERC1967Proxy import, and implementation deployment.
#[test]
fn gen_deploy_uups() {
    let ctx = TestContext::new("gen-deploy-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("gen_deploy_uups")
        .test(&[
            "gen-deploy",
            "Counter",
            "--proxy",
            "uups",
            "--output",
            "script/DeployCounter_uups.s.sol",
        ])
        .output_artifact("script/DeployCounter_uups.s.sol")
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(CompilerOutputNormalizer));

    run_integration_test(&test, &ctx);
}

/// Transparent proxy — TransparentUpgradeableProxy (Counter).
///
/// Verifies generated Solidity imports TransparentUpgradeableProxy,
/// includes proxyAdmin address, and deploys implementation + proxy.
#[test]
fn gen_deploy_transparent() {
    let ctx = TestContext::new("gen-deploy-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("gen_deploy_transparent")
        .test(&[
            "gen-deploy",
            "Counter",
            "--proxy",
            "transparent",
            "--output",
            "script/DeployCounter_transparent.s.sol",
        ])
        .output_artifact("script/DeployCounter_transparent.s.sol")
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(CompilerOutputNormalizer));

    run_integration_test(&test, &ctx);
}

/// Beacon proxy — UpgradeableBeacon + BeaconProxy (Counter).
///
/// Verifies generated Solidity imports both UpgradeableBeacon and
/// BeaconProxy, deploys implementation + beacon + proxy.
#[test]
fn gen_deploy_beacon() {
    let ctx = TestContext::new("gen-deploy-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("gen_deploy_beacon")
        .test(&[
            "gen-deploy",
            "Counter",
            "--proxy",
            "beacon",
            "--output",
            "script/DeployCounter_beacon.s.sol",
        ])
        .output_artifact("script/DeployCounter_beacon.s.sol")
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(CompilerOutputNormalizer));

    run_integration_test(&test, &ctx);
}

/// CREATE2 + UUPS combination (Counter).
///
/// Verifies generated Solidity contains both CREATE2 salt-based deployment
/// and UUPS proxy pattern.
#[test]
fn gen_deploy_create2_uups() {
    let ctx = TestContext::new("gen-deploy-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("gen_deploy_create2_uups")
        .test(&[
            "gen-deploy",
            "Counter",
            "--strategy",
            "create2",
            "--proxy",
            "uups",
            "--output",
            "script/DeployCounter_create2_uups.s.sol",
        ])
        .output_artifact("script/DeployCounter_create2_uups.s.sol")
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(CompilerOutputNormalizer));

    run_integration_test(&test, &ctx);
}

/// Custom proxy contract override (Counter + ERC1967).
///
/// Verifies generated Solidity contains custom proxy name `MyCustomProxy`
/// and a TODO import comment instead of OpenZeppelin import.
#[test]
fn gen_deploy_custom_proxy_contract() {
    let ctx = TestContext::new("gen-deploy-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("gen_deploy_custom_proxy_contract")
        .test(&[
            "gen-deploy",
            "Counter",
            "--proxy",
            "erc1967",
            "--proxy-contract",
            "MyCustomProxy",
            "--output",
            "script/DeployCounter_custom_proxy.s.sol",
        ])
        .output_artifact("script/DeployCounter_custom_proxy.s.sol")
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(CompilerOutputNormalizer));

    run_integration_test(&test, &ctx);
}

// ── Error path tests ────────────────────────────────────────────────────

/// Error: missing contract — artifact not found in compilation output.
///
/// Verifies error message mentions "not found" and lists available contracts.
#[test]
fn gen_deploy_error_missing_contract() {
    let ctx = TestContext::new("gen-deploy-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("gen_deploy_error_missing_contract")
        .test(&["gen-deploy", "NonExistentContract"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(CompilerOutputNormalizer));

    run_integration_test(&test, &ctx);
}

/// Error: invalid strategy — not one of create, create2, create3.
///
/// Verifies error message lists valid strategies.
#[test]
fn gen_deploy_error_invalid_strategy() {
    let ctx = TestContext::new("gen-deploy-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("gen_deploy_error_invalid_strategy")
        .test(&["gen-deploy", "Counter", "--strategy", "invalid"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(CompilerOutputNormalizer));

    run_integration_test(&test, &ctx);
}

/// Error: invalid proxy — not one of erc1967, uups, transparent, beacon.
///
/// Verifies error message lists valid proxy patterns.
#[test]
fn gen_deploy_error_invalid_proxy() {
    let ctx = TestContext::new("gen-deploy-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("gen_deploy_error_invalid_proxy")
        .test(&["gen-deploy", "Counter", "--proxy", "invalid"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(CompilerOutputNormalizer));

    run_integration_test(&test, &ctx);
}

/// Error: library with proxy — libraries cannot be deployed behind proxies.
///
/// Verifies error message about library proxy incompatibility.
#[test]
fn gen_deploy_error_library_proxy() {
    let ctx = TestContext::new("gen-deploy-project");
    let path_normalizer = PathNormalizer::new(vec![ctx.path().display().to_string()]);

    let test = IntegrationTest::new("gen_deploy_error_library_proxy")
        .test(&["gen-deploy", "MathLib", "--proxy", "erc1967"])
        .expect_err(true)
        .extra_normalizer(Box::new(path_normalizer))
        .extra_normalizer(Box::new(CompilerOutputNormalizer));

    run_integration_test(&test, &ctx);
}
