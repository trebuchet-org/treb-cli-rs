#![allow(dead_code)]

//! Shared helpers for golden-file integration tests.

use std::path::Path;

fn seed_registry_from_fixture(project_root: &Path, fixture_path: &Path) {
    let fixture_json =
        std::fs::read_to_string(fixture_path).expect("registry fixture should exist");

    // Write deployments directly to .treb/deployments.json.
    let deployments_path = project_root.join(".treb/deployments.json");
    std::fs::write(&deployments_path, &fixture_json)
        .expect("should write deployments.json to .treb/");

    // Rebuild the lookup index using the registry API.
    let registry =
        treb_registry::Registry::open(project_root).expect("registry should open after seeding");
    registry.rebuild_lookup_index().expect("lookup index rebuild should succeed");

    std::fs::write(
        project_root.join(".treb/config.local.json"),
        "{\n  \"namespace\": \"mainnet\",\n  \"network\": \"\"\n}\n",
    )
    .expect("should select the fixture namespace in local config");
}

/// Seed the registry with fixture deployments from `deployments_map.json`.
///
/// Reads the deployments fixture from `treb-core/tests/fixtures/`, writes it
/// to `.treb/deployments.json` in the given project root, and rebuilds the
/// lookup index so queries work correctly.
///
/// Designed for use as a `post_setup_hook` in [`IntegrationTest`]:
///
/// ```ignore
/// IntegrationTest::new("test_name")
///     .setup(&["init"])
///     .post_setup_hook(|ctx| helpers::seed_registry(ctx.path()))
/// ```
pub fn seed_registry(project_root: &Path) {
    let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../treb-core/tests/fixtures/deployments_map.json");
    seed_registry_from_fixture(project_root, &fixture_path);
}

/// Seed the registry with Go-created compatibility fixtures.
///
/// Writes the bare JSON map from `treb-registry/tests/fixtures/go-compat/`
/// into `.treb/deployments.json` and rebuilds the lookup index.
/// Sets namespace to `mainnet` in local config (matching 5 of 13 fixture entries).
pub fn seed_go_compat_registry(project_root: &Path) {
    let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../treb-registry/tests/fixtures/go-compat/deployments.json");
    seed_registry_from_fixture(project_root, &fixture_path);
}
