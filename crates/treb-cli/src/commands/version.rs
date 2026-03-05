//! `treb version` command implementation.

use serde::Serialize;
use treb_forge::detect_forge_version;

use crate::output;

/// All version fields collected at compile time and runtime.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
    pub version: String,
    pub commit: String,
    pub date: String,
    pub rust_version: String,
    pub forge_version: String,
    pub foundry_version: String,
    pub treb_sol_commit: String,
}

pub async fn run(json: bool) -> anyhow::Result<()> {
    let forge = detect_forge_version();

    let info = VersionInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        commit: env!("TREB_GIT_COMMIT").to_string(),
        date: env!("TREB_BUILD_DATE").to_string(),
        rust_version: env!("TREB_RUST_VERSION").to_string(),
        forge_version: forge.display_string(),
        foundry_version: env!("TREB_FOUNDRY_VERSION").to_string(),
        treb_sol_commit: env!("TREB_SOL_COMMIT").to_string(),
    };

    if json {
        output::print_json(&info)?;
    } else {
        output::print_kv(&[
            ("Version", &info.version),
            ("Commit", &info.commit),
            ("Date", &info.date),
            ("Rust Version", &info.rust_version),
            ("Forge Version", &info.forge_version),
            ("Foundry Version", &info.foundry_version),
            ("treb-sol Commit", &info.treb_sol_commit),
        ]);
    }

    Ok(())
}
