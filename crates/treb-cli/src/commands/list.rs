//! `treb list` command implementation.

#[allow(clippy::too_many_arguments)]
pub async fn run(
    _network: Option<String>,
    _namespace: Option<String>,
    _type: Option<String>,
    _tag: Option<String>,
    _contract: Option<String>,
    _label: Option<String>,
    _fork: bool,
    _no_fork: bool,
    _json: bool,
) -> anyhow::Result<()> {
    Ok(())
}
