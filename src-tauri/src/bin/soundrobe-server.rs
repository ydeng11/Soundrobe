#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() == Some(std::ffi::OsStr::new("--nominate-library")) {
        let root = args
            .next()
            .filter(|_| args.next().is_none())
            .ok_or_else(|| {
                anyhow::anyhow!("Usage: soundrobe-server --nominate-library <library-directory>")
            })?;
        soundrobe_lib::server::nominate_library(std::path::Path::new(&root))
    } else {
        soundrobe_lib::server::run().await
    }
}
