//! Shared helpers for golden-file integration tests.

use std::path::Path;

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
    let fixture_json =
        std::fs::read_to_string(&fixture_path).expect("deployments_map.json fixture should exist");

    // Write deployments directly to .treb/deployments.json.
    let deployments_path = project_root.join(".treb/deployments.json");
    std::fs::write(&deployments_path, &fixture_json)
        .expect("should write deployments.json to .treb/");

    // Rebuild the lookup index using the registry API.
    let registry =
        treb_registry::Registry::open(project_root).expect("registry should open after seeding");
    registry.rebuild_lookup_index().expect("lookup index rebuild should succeed");
}
