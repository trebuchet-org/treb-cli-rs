//! `treb config` command implementation.

pub async fn show(_json: bool) -> anyhow::Result<()> {
    println!("config show: not yet implemented");
    Ok(())
}

pub async fn set(_key: &str, _value: &str) -> anyhow::Result<()> {
    println!("config set: not yet implemented");
    Ok(())
}

pub async fn remove(_key: &str) -> anyhow::Result<()> {
    println!("config remove: not yet implemented");
    Ok(())
}
