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
        println!("treb {}", info.version);

        let has_commit = info.commit != "unknown";
        let has_date = info.date != "unknown";

        if has_commit || has_date {
            println!();
            if has_commit {
                let short_commit =
                    if info.commit.len() > 7 { &info.commit[..7] } else { &info.commit };
                println!("commit: {short_commit}");
            }
            if has_date {
                let formatted_date = output::format_build_date(&info.date);
                println!("built:  {formatted_date}");
            }
        }
    }

    Ok(())
}
