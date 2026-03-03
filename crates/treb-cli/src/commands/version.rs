//! `treb version` command implementation.

use serde::Serialize;
use treb_forge::detect_forge_version;

use crate::output;

/// All version fields collected at compile time and runtime.
#[derive(Serialize)]
pub struct VersionInfo {
    pub version: String,
    pub git_commit: String,
    pub build_date: String,
    pub rust_version: String,
    pub forge_version: String,
}

pub async fn run(json: bool) -> anyhow::Result<()> {
    let forge = detect_forge_version();

    let info = VersionInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        git_commit: env!("TREB_GIT_COMMIT").to_string(),
        build_date: env!("TREB_BUILD_DATE").to_string(),
        rust_version: env!("TREB_RUST_VERSION").to_string(),
        forge_version: forge.display_string(),
    };

    if json {
        output::print_json(&info)?;
    } else {
        output::print_kv(&[
            ("Version", &info.version),
            ("Git Commit", &info.git_commit),
            ("Build Date", &info.build_date),
            ("Rust Version", &info.rust_version),
            ("Forge Version", &info.forge_version),
        ]);
    }

    Ok(())
}
