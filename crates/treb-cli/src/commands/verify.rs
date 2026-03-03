//! `treb verify` command implementation.

use anyhow::bail;

/// Run the verify command.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    deployment: Option<String>,
    all: bool,
    verifier: &str,
    _verifier_url: Option<String>,
    _verifier_api_key: Option<String>,
    _force: bool,
    _watch: bool,
    _retries: u32,
    _delay: u64,
    _json: bool,
) -> anyhow::Result<()> {
    // Validate that either a deployment or --all is provided.
    if deployment.is_none() && !all {
        bail!(
            "either a <DEPLOYMENT> argument or --all flag is required\n\n\
             Usage:\n  treb verify <DEPLOYMENT>\n  treb verify --all"
        );
    }

    // Validate verifier value.
    match verifier {
        "etherscan" | "sourcify" | "blockscout" => {}
        other => {
            bail!(
                "unknown verifier '{}': expected one of etherscan, sourcify, blockscout",
                other
            );
        }
    }

    if all {
        eprintln!("verify --all: not yet implemented");
    } else if let Some(ref dep) = deployment {
        eprintln!("verify {}: not yet implemented", dep);
    }

    Ok(())
}
