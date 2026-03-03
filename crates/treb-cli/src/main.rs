#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("treb v{}", env!("CARGO_PKG_VERSION"));
    Ok(())
}
