#[tokio::main]
async fn main() -> anyhow::Result<()> {
    soundrobe_lib::server::run().await
}
