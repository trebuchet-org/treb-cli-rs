//! `treb register` command implementation.

pub async fn run(
    _tx_hash: &str,
    _network: Option<String>,
    _rpc_url: Option<String>,
    _address: Option<String>,
    _contract: Option<String>,
    _contract_name: Option<String>,
    _label: Option<String>,
    _namespace: Option<String>,
    _skip_verify: bool,
    _json: bool,
) -> anyhow::Result<()> {
    println!("register: not yet implemented");
    Ok(())
}
