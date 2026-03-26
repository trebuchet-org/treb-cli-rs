//! Focused CLI tests for `treb registry tag` query scoping.

use assert_cmd::cargo::cargo_bin_cmd;
use chrono::Utc;
use predicates::prelude::*;
use std::{collections::HashMap, fs};
use treb_core::types::{
    ArtifactInfo, Deployment, DeploymentMethod, DeploymentStrategy, DeploymentType,
    VerificationInfo, VerificationStatus,
};

fn treb() -> assert_cmd::Command {
    cargo_bin_cmd!("treb-cli")
}

const MINIMAL_FOUNDRY_TOML: &str = "[profile.default]\n";

fn init_empty_project(tmp: &tempfile::TempDir) {
    fs::write(tmp.path().join("foundry.toml"), MINIMAL_FOUNDRY_TOML).unwrap();
    treb().arg("init").current_dir(tmp.path()).assert().success();
}

fn init_project_with_custom_deployments(
    tmp: &tempfile::TempDir,
    deployments: impl IntoIterator<Item = Deployment>,
) {
    init_empty_project(tmp);

    let mut registry = treb_registry::Registry::open(tmp.path()).expect("registry should open");
    for deployment in deployments {
        registry.insert_deployment(deployment).expect("deployment insert should succeed");
    }
}

fn make_tag_deployment(
    namespace: &str,
    chain_id: u64,
    contract_name: &str,
    label: &str,
    address: &str,
    tags: Option<Vec<&str>>,
) -> Deployment {
    let ts = Utc::now();

    Deployment {
        id: format!("{namespace}/{chain_id}/{contract_name}:{label}"),
        namespace: namespace.to_string(),
        chain_id,
        contract_name: contract_name.to_string(),
        label: label.to_string(),
        address: address.to_string(),
        deployment_type: DeploymentType::Singleton,
        execution: None,
        transaction_id: format!("tx-{namespace}-{chain_id}-{contract_name}"),
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
            path: "contracts/Test.sol".to_string(),
            compiler_version: "0.8.24".to_string(),
            bytecode_hash: "0xabc".to_string(),
            script_path: "script/Deploy.s.sol".to_string(),
            git_commit: "abc123".to_string(),
        },
        verification: VerificationInfo {
            status: VerificationStatus::Unverified,
            etherscan_url: String::new(),
            verified_at: None,
            reason: String::new(),
            verifiers: HashMap::new(),
        },
        tags: tags.map(|entries| entries.into_iter().map(str::to_string).collect()),
        created_at: ts,
        updated_at: ts,
    }
}

#[test]
fn tag_show_resolves_with_namespace_scope() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_custom_deployments(
        &tmp,
        [
            make_tag_deployment(
                "mainnet",
                42220,
                "Counter",
                "v1",
                "0x0000000000000000000000000000000000001111",
                Some(vec!["stable"]),
            ),
            make_tag_deployment(
                "staging",
                42220,
                "Counter",
                "v1",
                "0x0000000000000000000000000000000000002222",
                Some(vec!["beta"]),
            ),
        ],
    );

    treb()
        .args(["registry", "tag", "--namespace", "mainnet", "Counter"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("mainnet/42220/Counter:v1"))
        .stdout(predicate::str::contains("stable"))
        .stdout(predicate::str::contains("beta").not());
}

#[test]
fn tag_add_scopes_by_namespace_and_network() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_custom_deployments(
        &tmp,
        [
            make_tag_deployment(
                "mainnet",
                42220,
                "Counter",
                "v1",
                "0x0000000000000000000000000000000000001111",
                None,
            ),
            make_tag_deployment(
                "mainnet",
                1,
                "Counter",
                "v1",
                "0x0000000000000000000000000000000000002222",
                None,
            ),
        ],
    );

    treb()
        .args(["registry", "tag", "--add", "v2", "-s", "mainnet", "-n", "42220", "Counter"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("mainnet/42220/Counter:v1"));

    let registry = treb_registry::Registry::open(tmp.path()).unwrap();
    assert_eq!(
        registry.get_deployment("mainnet/42220/Counter:v1").unwrap().tags,
        Some(vec!["v2".to_string()])
    );
    assert_eq!(registry.get_deployment("mainnet/1/Counter:v1").unwrap().tags, None);
}

#[test]
fn tag_remove_scopes_by_namespace() {
    let tmp = tempfile::tempdir().unwrap();
    init_project_with_custom_deployments(
        &tmp,
        [
            make_tag_deployment(
                "mainnet",
                42220,
                "Counter",
                "v1",
                "0x0000000000000000000000000000000000001111",
                Some(vec!["v2"]),
            ),
            make_tag_deployment(
                "staging",
                42220,
                "Counter",
                "v1",
                "0x0000000000000000000000000000000000002222",
                Some(vec!["v2"]),
            ),
        ],
    );

    treb()
        .args(["registry", "tag", "--remove", "v2", "--namespace", "mainnet", "Counter"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("mainnet/42220/Counter:v1"));

    let registry = treb_registry::Registry::open(tmp.path()).unwrap();
    assert_eq!(registry.get_deployment("mainnet/42220/Counter:v1").unwrap().tags, None);
    assert_eq!(
        registry.get_deployment("staging/42220/Counter:v1").unwrap().tags,
        Some(vec!["v2".to_string()])
    );
}
