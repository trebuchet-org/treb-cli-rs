//! Golden-file integration tests for `treb gen-deploy`.
//!
//! Tests exercise CREATE, CREATE2, CREATE3 strategies, library deployment,
//! no-constructor contracts, and JSON output using the `gen-deploy-project`
//! fixture which contains Counter (with constructor), SimpleContract (no
//! constructor), and MathLib (library).

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
