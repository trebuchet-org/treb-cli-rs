//! Golden-file integration tests for `treb version`.

mod framework;

use framework::context::TestContext;
use framework::integration_test::{run_integration_test, IntegrationTest};
use framework::normalizer::ShortHexNormalizer;

/// Human-readable version output matches golden file.
#[test]
fn version_human() {
    let ctx = TestContext::new("project");

    let test = IntegrationTest::new("version_human")
        .test(&["version"])
        .extra_normalizer(Box::new(ShortHexNormalizer));

    run_integration_test(&test, &ctx);
}

/// JSON version output matches golden file.
#[test]
fn version_json() {
    let ctx = TestContext::new("project");

    let test = IntegrationTest::new("version_json")
        .test(&["version", "--json"])
        .extra_normalizer(Box::new(ShortHexNormalizer));

    run_integration_test(&test, &ctx);
}
