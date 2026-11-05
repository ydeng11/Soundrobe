//! Offline export entry point: deliberately never initializes the app runtime.
use soundrobe_lib::commands::artist_inventory::{inventory, markdown};
use soundrobe_lib::commands::recording_discovery;
use soundrobe_lib::infra::audio_evidence;
use soundrobe_lib::state::providers::ProviderState;
use std::{
    io::{self, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let usage = "Usage: soundrobe-inventory ARTIST_ROOT [--worker-data DIR] [--historical COMPOSED_DIR] [--format json|markdown] [--audio-pairs MANIFEST.json | --reuse-proposals MANIFEST.json | --recording-discovery MANIFEST.json --discovery-cache DIR [--allow-network]]\nDefault: reads media and existing evidence only. Writes export to stdout; never starts Soundrobe, watching or tagging. Optional manifests require relative inventoried paths and expected SHA-256 hashes. Audio diagnostics never prove identity. Reuse proposals: left donor, right recipient; current independently rooted receipts and checksum-verified FLAC only; no writes or verification changes. Recording discovery: catalog hypotheses only, writes explicit isolated cache; networking disabled unless --allow-network. Unix FFmpeg/ffprobe; Ctrl-C cancels optional work.";
    let root = args.next().ok_or_else(|| anyhow::anyhow!(usage))?;
    if root == "--help" || root == "-h" {
        println!("{usage}");
        return Ok(());
    }
    let mut worker_data = None;
    let mut historical = None;
    let mut audio_pairs = None;
    let mut reuse_pairs = None;
    let mut discovery = None;
    let mut discovery_cache = None;
    let mut allow_network = false;
    let mut format = "json".to_owned();
    while let Some(flag) = args.next() {
        if flag == "--allow-network" {
            allow_network = true;
            continue;
        }
        let value = args.next().ok_or_else(|| anyhow::anyhow!(usage))?;
        match flag.to_str() {
            Some("--worker-data") => worker_data = Some(PathBuf::from(value)),
            Some("--historical") => historical = Some(PathBuf::from(value)),
            Some("--audio-pairs") => audio_pairs = Some(PathBuf::from(value)),
            Some("--reuse-proposals") => reuse_pairs = Some(PathBuf::from(value)),
            Some("--recording-discovery") => discovery = Some(PathBuf::from(value)),
            Some("--discovery-cache") => discovery_cache = Some(PathBuf::from(value)),
            Some("--format") => format = value.into_string().map_err(|_| anyhow::anyhow!(usage))?,
            _ => anyhow::bail!("{usage}"),
        }
    }
    anyhow::ensure!(format == "json" || format == "markdown", "{usage}");
    anyhow::ensure!(
        [
            audio_pairs.is_some(),
            reuse_pairs.is_some(),
            discovery.is_some()
        ]
        .into_iter()
        .filter(|b| *b)
        .count()
            <= 1,
        "select audio diagnostics, reuse proposals or recording discovery, not multiple modes"
    );
    anyhow::ensure!(
        discovery.is_some() == discovery_cache.is_some() && (!allow_network || discovery.is_some()),
        "recording discovery requires --discovery-cache; --allow-network is discovery-only"
    );
    let reuse = reuse_pairs.is_some();
    let mut report = inventory(
        &PathBuf::from(root),
        worker_data.as_deref(),
        historical.as_deref(),
    )
    .map_err(anyhow::Error::msg)?;
    let discovering = discovery.is_some();
    if let Some(manifest) = audio_pairs.or(reuse_pairs).or(discovery) {
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = Arc::clone(&cancelled);
        let handler = tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                signal.store(true, Ordering::Release);
            }
        });
        let result = if discovering {
            recording_discovery::export(
                &report,
                &manifest,
                discovery_cache.as_deref().unwrap(),
                &ProviderState::new(),
                allow_network,
                &cancelled,
            )
            .await
        } else if reuse {
            audio_evidence::reuse_proposals(&report, &manifest, &cancelled, worker_data.as_deref())
                .await
        } else {
            audio_evidence::export(&report, &manifest, &cancelled).await
        };
        handler.abort();
        report[if discovering {
            "recordingDiscovery"
        } else if reuse {
            "reuseProposals"
        } else {
            "audioEvidence"
        }] = result.map_err(anyhow::Error::msg)?;
    }
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    if format == "markdown" {
        stdout.write_all(markdown(&report).as_bytes())?;
        if discovering {
            writeln!(
                stdout,
                "\n## Recording discovery\n\n{}\n\n```json\n{}\n```",
                recording_discovery::SCOPE,
                serde_json::to_string_pretty(&report["recordingDiscovery"])?
            )?;
        }
    } else {
        serde_json::to_writer_pretty(&mut stdout, &report)?;
        writeln!(stdout)?;
    }
    stdout.flush()?;
    Ok(())
}
