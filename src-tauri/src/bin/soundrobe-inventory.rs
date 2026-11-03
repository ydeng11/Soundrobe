//! Offline export entry point: deliberately never initializes the app runtime.
use soundrobe_lib::commands::artist_inventory::{inventory, markdown};
use std::{
    io::{self, Write},
    path::PathBuf,
};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let usage = "Usage: soundrobe-inventory ARTIST_ROOT [--worker-data DIR] [--historical COMPOSED_DIR] [--format json|markdown]\nReads media and existing evidence only. Writes the export to stdout; does not start Soundrobe, providers, watching or tagging.";
    let root = args.next().ok_or_else(|| anyhow::anyhow!(usage))?;
    if root == "--help" || root == "-h" {
        println!("{usage}");
        return Ok(());
    }
    let mut worker_data = None;
    let mut historical = None;
    let mut format = "json".to_owned();
    while let Some(flag) = args.next() {
        let value = args.next().ok_or_else(|| anyhow::anyhow!(usage))?;
        match flag.to_str() {
            Some("--worker-data") => worker_data = Some(PathBuf::from(value)),
            Some("--historical") => historical = Some(PathBuf::from(value)),
            Some("--format") => format = value.into_string().map_err(|_| anyhow::anyhow!(usage))?,
            _ => anyhow::bail!("{usage}"),
        }
    }
    anyhow::ensure!(format == "json" || format == "markdown", "{usage}");
    let report = inventory(
        &PathBuf::from(root),
        worker_data.as_deref(),
        historical.as_deref(),
    )
    .map_err(anyhow::Error::msg)?;
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    if format == "markdown" {
        stdout.write_all(markdown(&report).as_bytes())?;
    } else {
        serde_json::to_writer_pretty(&mut stdout, &report)?;
        writeln!(stdout)?;
    }
    stdout.flush()?;
    Ok(())
}
