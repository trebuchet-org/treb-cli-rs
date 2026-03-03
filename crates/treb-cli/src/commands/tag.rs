pub async fn run(
    deployment: &str,
    add: Option<String>,
    remove: Option<String>,
    json: bool,
) -> anyhow::Result<()> {
    let _ = (deployment, add, remove, json);
    println!("tag: not yet implemented");
    Ok(())
}
