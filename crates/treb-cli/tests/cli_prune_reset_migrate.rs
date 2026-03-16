//! Integration tests for `treb prune`, `treb reset`, and `treb migrate`.

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::{collections::HashMap, fs};

use chrono::Utc;
use treb_core::types::{
    ArtifactInfo, DeploymentMethod, DeploymentStrategy, DeploymentType, TransactionStatus,
    VerificationInfo, VerificationStatus,
};

fn treb() -> assert_cmd::Command {
    cargo_bin_cmd!("treb-cli")
}

const MINIMAL_FOUNDRY_TOML: &str = "[profile.default]\n";

const V1_TREB_TOML: &str = r#"[ns.default.senders.deployer]
type = "private_key"
address = "0xDeployerAddr"
private_key = "0xDeployerKey"
"#;

const V2_TREB_TOML: &str = "[accounts.deployer]\ntype = \"private_key\"\n";

// ── Fixture builders ──────────────────────────────────────────────────────────

fn make_deployment(
    id: &str,
    tx_id: &str,
    chain_id: u64,
    namespace: &str,
) -> treb_core::types::Deployment {
    let ts = Utc::now();
    treb_core::types::Deployment {
        id: id.to_string(),
        namespace: namespace.to_string(),
        chain_id,
        contract_name: "TestContract".to_string(),
        label: "v1".to_string(),
        address: format!("0x{:040x}", chain_id),
        deployment_type: DeploymentType::Singleton,
        transaction_id: tx_id.to_string(),
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
        tags: None,
        created_at: ts,
        updated_at: ts,
    }
}

fn make_transaction(
    id: &str,
    dep_ids: Vec<String>,
    chain_id: u64,
) -> treb_core::types::Transaction {
    let ts = Utc::now();
    treb_core::types::Transaction {
        id: id.to_string(),
        chain_id,
        hash: format!("0x{:064x}", 0u64),
        status: TransactionStatus::Executed,
        block_number: 1000,
        sender: "0x56fD3F2bEE130e9867942D0F463a16fBE49B8d81".to_string(),
        nonce: 0,
        deployments: dep_ids,
        operations: vec![],
        safe_context: None,
        broadcast_file: None,
        environment: "testnet".to_string(),
        created_at: ts,
    }
}

/// Initialize a project with `foundry.toml` and an empty registry.
/// Returns the registry handle for inserting test data.
fn init_project(tmp: &tempfile::TempDir) -> treb_registry::Registry {
    fs::write(tmp.path().join("foundry.toml"), MINIMAL_FOUNDRY_TOML).unwrap();
    treb_registry::Registry::init(tmp.path()).expect("registry init should succeed")
}

// ── treb prune ────────────────────────────────────────────────────────────────

#[test]
fn prune_dry_run_outputs_candidates_and_does_not_modify_files() {
    let tmp = tempfile::tempdir().unwrap();
    let mut registry = init_project(&tmp);

    // Insert a deployment with a broken transaction reference.
    registry.insert_deployment(make_deployment("dep-broken", "tx-missing", 1, "default")).unwrap();

    let orig_deployments = fs::read_to_string(tmp.path().join(".treb/deployments.json")).unwrap();

    treb()
        .args(["prune", "--dry-run"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("dep-broken"));

    // Deployments file must be unchanged.
    let after_deployments = fs::read_to_string(tmp.path().join(".treb/deployments.json")).unwrap();
    assert_eq!(orig_deployments, after_deployments, "dry-run must not modify deployments.json");

    // No prune backup should have been created.
    let backups_dir = tmp.path().join(".treb/backups");
    let prune_backups: Vec<_> = if backups_dir.exists() {
        fs::read_dir(&backups_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("prune-"))
            .collect()
    } else {
        vec![]
    };
    assert!(prune_backups.is_empty(), "dry-run should not create a prune backup");
}

#[test]
fn prune_yes_removes_broken_entry_and_creates_backup() {
    let tmp = tempfile::tempdir().unwrap();
    let mut registry = init_project(&tmp);

    // Insert a deployment with a broken transaction reference.
    registry.insert_deployment(make_deployment("dep-broken", "tx-missing", 1, "default")).unwrap();

    let output = treb().args(["prune", "--yes"]).current_dir(tmp.path()).output().unwrap();

    assert!(output.status.success(), "prune --yes should succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();

    // Output reports prune success without exposing the backup path.
    assert!(
        stdout.contains("Running in non-interactive mode. Proceeding with prune..."),
        "stdout should include the non-interactive proceed line: {stdout}"
    );
    assert!(
        stdout.contains("✅ Successfully pruned 1 items."),
        "stdout should report prune success: {stdout}"
    );
    assert!(
        !stdout.to_lowercase().contains("backup"),
        "stdout should not mention backup path: {stdout}"
    );

    // A backup directory should exist under .treb/backups/.
    let backups_dir = tmp.path().join(".treb/backups");
    assert!(backups_dir.exists(), ".treb/backups/ should exist");
    let prune_backups: Vec<_> = fs::read_dir(&backups_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("prune-"))
        .collect();
    assert_eq!(prune_backups.len(), 1, "exactly one prune backup should be created");

    // The broken deployment should be gone.
    let registry_after = treb_registry::Registry::open(tmp.path()).unwrap();
    assert!(
        registry_after.get_deployment("dep-broken").is_none(),
        "dep-broken should be removed after prune --yes"
    );
}

#[test]
fn prune_non_interactive_proceeds_without_yes_and_creates_backup() {
    let tmp = tempfile::tempdir().unwrap();
    let mut registry = init_project(&tmp);

    registry.insert_deployment(make_deployment("dep-broken", "tx-missing", 1, "default")).unwrap();

    let output = treb()
        .args(["prune"])
        .env("TREB_NON_INTERACTIVE", "true")
        .current_dir(tmp.path())
        .output()
        .unwrap();

    assert!(output.status.success(), "prune should succeed in non-interactive mode");
    let stdout = String::from_utf8(output.stdout).unwrap();

    assert!(
        stdout.contains("Running in non-interactive mode. Proceeding with prune..."),
        "stdout should include the non-interactive proceed line: {stdout}"
    );
    assert!(
        stdout.contains("✅ Successfully pruned 1 items."),
        "stdout should report prune success: {stdout}"
    );

    let registry_after = treb_registry::Registry::open(tmp.path()).unwrap();
    assert!(
        registry_after.get_deployment("dep-broken").is_none(),
        "dep-broken should be removed after non-interactive prune"
    );
}

#[test]
fn prune_dry_run_on_clean_registry_outputs_nothing_to_prune() {
    let tmp = tempfile::tempdir().unwrap();
    let mut registry = init_project(&tmp);

    // Insert a consistent entry (deployment + transaction that reference each other).
    registry.insert_transaction(make_transaction("tx-1", vec!["dep-1".to_string()], 1)).unwrap();
    registry.insert_deployment(make_deployment("dep-1", "tx-1", 1, "default")).unwrap();

    treb()
        .args(["prune", "--dry-run"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Nothing to prune."));
}

// ── treb reset ────────────────────────────────────────────────────────────────

#[test]
fn reset_yes_empties_all_stores_and_creates_backup() {
    let tmp = tempfile::tempdir().unwrap();
    let mut registry = init_project(&tmp);

    registry.insert_deployment(make_deployment("dep-1", "", 1, "default")).unwrap();
    registry.insert_deployment(make_deployment("dep-2", "", 42220, "staging")).unwrap();

    let output = treb().args(["reset", "--yes"]).current_dir(tmp.path()).output().unwrap();

    assert!(output.status.success(), "reset --yes should succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();

    assert!(
        stdout.contains("Found 2 items to reset for namespace 'default' across all networks:"),
        "stdout should describe the mixed-chain reset scope without inventing a default network: {stdout}"
    );
    assert!(
        stdout.contains("  Deployments:        2"),
        "stdout should include aligned reset counts: {stdout}"
    );
    assert!(
        stdout.contains("Running in non-interactive mode. Proceeding with reset..."),
        "stdout should include the non-interactive proceed line: {stdout}"
    );
    assert!(
        stdout.contains("Successfully reset 2 items from the registry."),
        "stdout should contain the plain reset success line: {stdout}"
    );
    assert!(!stdout.contains("31337"), "stdout should not claim chain 31337: {stdout}");
    assert!(
        !stdout.to_lowercase().contains("backup"),
        "stdout should not mention backup paths: {stdout}"
    );

    // Backup directory should exist.
    let backups_dir = tmp.path().join(".treb/backups");
    assert!(backups_dir.exists(), ".treb/backups/ should exist");
    let reset_backups: Vec<_> = fs::read_dir(&backups_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("reset-"))
        .collect();
    assert_eq!(reset_backups.len(), 1, "exactly one reset backup should be created");

    // Registry should be empty.
    let registry_after = treb_registry::Registry::open(tmp.path()).unwrap();
    assert_eq!(
        registry_after.deployment_count(),
        0,
        "all deployments should be removed after full reset"
    );
}

#[test]
fn reset_network_removes_only_matching_chain() {
    let tmp = tempfile::tempdir().unwrap();
    let mut registry = init_project(&tmp);

    // Two deployments on different chains.
    registry.insert_deployment(make_deployment("dep-chain1", "", 1, "default")).unwrap();
    registry.insert_deployment(make_deployment("dep-chain42220", "", 42220, "default")).unwrap();

    treb().args(["reset", "--network", "1", "--yes"]).current_dir(tmp.path()).assert().success();

    let registry_after = treb_registry::Registry::open(tmp.path()).unwrap();
    assert!(
        registry_after.get_deployment("dep-chain1").is_none(),
        "dep-chain1 (chain 1) should be removed"
    );
    assert!(
        registry_after.get_deployment("dep-chain42220").is_some(),
        "dep-chain42220 (chain 42220) should remain"
    );
}

#[test]
fn reset_namespace_removes_only_matching_namespace() {
    let tmp = tempfile::tempdir().unwrap();
    let mut registry = init_project(&tmp);

    // Two deployments in different namespaces.
    registry.insert_deployment(make_deployment("dep-default", "", 1, "default")).unwrap();
    registry.insert_deployment(make_deployment("dep-staging", "", 1, "staging")).unwrap();

    treb()
        .args(["reset", "--namespace", "default", "--yes"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let registry_after = treb_registry::Registry::open(tmp.path()).unwrap();
    assert!(
        registry_after.get_deployment("dep-default").is_none(),
        "dep-default should be removed (namespace=default)"
    );
    assert!(
        registry_after.get_deployment("dep-staging").is_some(),
        "dep-staging should remain (different namespace)"
    );
}

// ── treb migrate config ───────────────────────────────────────────────────────

#[test]
fn migrate_config_dry_run_v1_prints_v2_content_without_modifying_file() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("foundry.toml"), MINIMAL_FOUNDRY_TOML).unwrap();
    fs::write(tmp.path().join("treb.toml"), V1_TREB_TOML).unwrap();

    let original = fs::read_to_string(tmp.path().join("treb.toml")).unwrap();

    let output =
        treb().args(["migrate", "config", "--dry-run"]).current_dir(tmp.path()).output().unwrap();

    assert!(output.status.success(), "migrate config --dry-run should succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();

    // stdout should contain v2 TOML structure (accounts section).
    assert!(
        stdout.contains("accounts"),
        "dry-run stdout should contain v2 TOML accounts section: {stdout}"
    );

    // treb.toml must be unchanged.
    let after = fs::read_to_string(tmp.path().join("treb.toml")).unwrap();
    assert_eq!(original, after, "dry-run must not modify treb.toml");

    // No backup file should be created.
    let backups: Vec<_> = fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("treb.toml.bak-"))
        .collect();
    assert!(backups.is_empty(), "dry-run must not create a backup file");
}

#[test]
fn migrate_config_v1_rewrites_file_and_creates_backup() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("foundry.toml"), MINIMAL_FOUNDRY_TOML).unwrap();
    fs::write(tmp.path().join("treb.toml"), V1_TREB_TOML).unwrap();

    let original = fs::read_to_string(tmp.path().join("treb.toml")).unwrap();

    treb().args(["migrate", "config"]).current_dir(tmp.path()).assert().success().stdout(
        predicate::str::contains("✓ treb.toml written successfully")
            .and(predicate::str::contains("Next steps:")),
    );

    // treb.toml should now be v2 format.
    let after = fs::read_to_string(tmp.path().join("treb.toml")).unwrap();
    assert_ne!(original, after, "treb.toml should have been rewritten to v2");

    let format = treb_config::detect_treb_config_format(tmp.path());
    assert_eq!(
        format,
        treb_config::TrebConfigFormat::V2,
        "treb.toml should be detected as v2 after migration"
    );

    // A backup file should exist containing the original v1 content.
    let backups: Vec<_> = fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("treb.toml.bak-"))
        .collect();
    assert_eq!(backups.len(), 1, "exactly one backup file should be created");
    let backup_content = fs::read_to_string(backups[0].path()).unwrap();
    assert_eq!(backup_content, original, "backup should contain original v1 content");
}

#[test]
fn migrate_config_already_v2_outputs_message_and_no_file_changes() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("foundry.toml"), MINIMAL_FOUNDRY_TOML).unwrap();
    fs::write(tmp.path().join("treb.toml"), V2_TREB_TOML).unwrap();

    let original = fs::read_to_string(tmp.path().join("treb.toml")).unwrap();

    treb()
        .args(["migrate", "config"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("already v2"));

    // File must be unchanged.
    let after = fs::read_to_string(tmp.path().join("treb.toml")).unwrap();
    assert_eq!(original, after, "already-v2 file must not be modified");

    // No backup should be created.
    let backups: Vec<_> = fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("treb.toml.bak-"))
        .collect();
    assert!(backups.is_empty(), "no backup should be created for already-v2 config");
}

// ── treb migrate ──────────────────────────────────────────────────────────────

#[test]
fn migrate_help_does_not_list_registry_subcommand() {
    let output =
        treb().args(["migrate", "--help"]).output().expect("failed to run treb migrate --help");

    assert!(output.status.success(), "migrate --help should succeed");

    let stdout = String::from_utf8(output.stdout).expect("help output should be utf-8");
    assert!(stdout.contains("config"), "migrate help should list the config subcommand: {stdout}");
    assert!(
        !stdout.contains("registry"),
        "migrate help should not mention the removed registry subcommand: {stdout}"
    );
}

#[test]
fn migrate_registry_subcommand_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("foundry.toml"), MINIMAL_FOUNDRY_TOML).unwrap();

    treb()
        .args(["migrate", "registry"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Error: unknown command \"registry\" for \"treb migrate\"",
        ))
        .stderr(predicate::str::contains("Run 'treb migrate --help' for usage."));
}
