//! Read-only recovery for FLAC containers rejected by fpcalc. Every decoded
//! byte must match STREAMINFO before any resulting fingerprint is accepted.

use super::*;
use md5::{Digest, Md5};
use std::fs::{File, Metadata};
use std::io::{Read, Seek};
use tokio::io::AsyncWriteExt;

const MAX_PCM_BYTES: u64 = 256 * 1024 * 1024;

// Stable held-file identity is currently implemented for Unix only. Other
// hosts retain direct fingerprinting and leave failed comparisons unresolved.
#[derive(Clone)]
pub(super) struct SourceSnapshot(Metadata);

pub(super) fn source_snapshot(path: &Path) -> Option<SourceSnapshot> {
    if !cfg!(unix)
        || !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("flac"))
    {
        return None;
    }
    let metadata = std::fs::metadata(path).ok()?;
    (metadata.is_file() && metadata.modified().is_ok()).then_some(SourceSnapshot(metadata))
}

impl SourceSnapshot {
    fn matches(&self, current: Metadata) -> bool {
        if !cfg!(unix) {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if current.dev() != self.0.dev()
                || current.ino() != self.0.ino()
                || current.ctime() != self.0.ctime()
                || current.ctime_nsec() != self.0.ctime_nsec()
            {
                return false;
            }
        }
        current.is_file()
            && current.len() == self.0.len()
            && current.modified().ok() == self.0.modified().ok()
            && current.created().ok() == self.0.created().ok()
    }
}

struct FlacInput {
    file: File,
    original: SourceSnapshot,
    rate: u32,
    channels: u32,
    format: &'static str,
    bytes: u64,
    md5: [u8; 16],
}

impl FlacInput {
    fn open(path: &Path, original: &SourceSnapshot) -> Result<Self, AcoustidError> {
        let failed = AcoustidError::ProcessFailed;
        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("flac"))
        {
            return Err(failed);
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // A path replaced by a FIFO must not block this runtime worker
            // before fstat can reject it as non-regular.
            options.custom_flags(libc::O_NONBLOCK);
        }
        let mut file = options.open(path).map_err(|_| failed)?;
        if !original.matches(file.metadata().map_err(|_| failed)?) {
            return Err(failed);
        }
        let mut header = [0; 42];
        file.read_exact(&mut header).map_err(|_| failed)?;
        if &header[..4] != b"fLaC" || header[4] & 0x7f != 0 || header[5..8] != [0, 0, 34] {
            return Err(failed);
        }
        let format = u64::from_be_bytes(header[18..26].try_into().map_err(|_| failed)?);
        let rate = (format >> 44) as u32;
        let channels = ((format >> 41) & 7) as u32 + 1;
        let bits = ((format >> 36) & 31) + 1;
        let pcm_format = match bits {
            16 => "s16le",
            24 => "s24le",
            _ => return Err(failed),
        };
        let samples = format & 0xf_ffff_ffff;
        let md5: [u8; 16] = header[26..42].try_into().map_err(|_| failed)?;
        let bytes = samples
            .checked_mul(u64::from(channels))
            .and_then(|n| n.checked_mul(bits / 8))
            .ok_or(failed)?;
        if rate == 0 || channels > 2 || bytes == 0 || bytes > MAX_PCM_BYTES || md5 == [0; 16] {
            return Err(failed);
        }
        file.rewind().map_err(|_| failed)?;
        Ok(Self {
            file,
            original: original.clone(),
            rate,
            channels,
            format: pcm_format,
            bytes,
            md5,
        })
    }

    fn unchanged(&self, path: &Path) -> bool {
        self.file
            .metadata()
            .is_ok_and(|metadata| self.original.matches(metadata))
            && std::fs::metadata(path).is_ok_and(|metadata| self.original.matches(metadata))
    }
}

fn ffmpeg_executable() -> PathBuf {
    if let Some(configured) = std::env::var_os("AUTO_TAG_FFMPEG_PATH") {
        return configured.into();
    }
    [
        "/opt/homebrew/bin/ffmpeg",
        "/usr/local/bin/ffmpeg",
        "/opt/local/bin/ffmpeg",
        "/usr/bin/ffmpeg",
    ]
    .into_iter()
    .map(Path::new)
    .find(|path| path.is_file())
    .map(Path::to_path_buf)
    .unwrap_or_else(|| PathBuf::from("ffmpeg"))
}

pub(super) async fn fingerprint(
    tools: FingerprintTools<'_>,
    path: &Path,
    job: &FingerprintJob<'_>,
    original: Option<&SourceSnapshot>,
) -> Result<Vec<u8>, AcoustidError> {
    job.check()?;
    let input = FlacInput::open(path, original.ok_or(AcoustidError::ProcessFailed)?)?;
    // Force the held file's FLAC demuxer and native signed PCM precision. No
    // filename/protocol interpretation, resampling, conversion or media writes.
    let executable = tools
        .ffmpeg
        .map(Path::to_path_buf)
        .unwrap_or_else(ffmpeg_executable);
    let mut decoder = Command::new(executable)
        .args([
            "-nostdin",
            "-v",
            "error",
            "-noauto_conversion_filters",
            "-threads",
            "1",
            "-f",
            "flac",
            "-i",
            "pipe:0",
            "-map",
            "0:a:0",
            "-c:a",
        ])
        .arg(format!("pcm_{}", input.format))
        // Reject format negotiation that would change rate/channels. The MD5
        // covers PCM bytes but does not encode their playback interpretation.
        .arg("-ar")
        .arg(input.rate.to_string())
        .arg("-ac")
        .arg(input.channels.to_string())
        .args(["-f", input.format, "pipe:1"])
        .stdin(Stdio::from(
            input
                .file
                .try_clone()
                .map_err(|_| AcoustidError::ProcessFailed)?,
        ))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| AcoustidError::ProcessFailed)?;
    let fingerprint = Command::new(tools.fpcalc)
        .args([
            "-json",
            "-raw",
            "-length",
            "0",
            "-format",
            input.format,
            "-rate",
        ])
        .arg(input.rate.to_string())
        .arg("-channels")
        .arg(input.channels.to_string())
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let mut fingerprint = match fingerprint {
        Ok(child) => child,
        Err(_) => {
            let _ = decoder.kill().await;
            let _ = decoder.wait().await;
            return Err(AcoustidError::ProcessFailed);
        }
    };
    let result = async {
        let pcm = decoder.stdout.take().ok_or(AcoustidError::ProcessFailed)?;
        let decoder_stderr = decoder.stderr.take().ok_or(AcoustidError::ProcessFailed)?;
        let stdin = fingerprint
            .stdin
            .take()
            .ok_or(AcoustidError::ProcessFailed)?;
        let stdout = fingerprint
            .stdout
            .take()
            .ok_or(AcoustidError::ProcessFailed)?;
        let stderr = fingerprint
            .stderr
            .take()
            .ok_or(AcoustidError::ProcessFailed)?;
        job.wait(async {
            let (output, errors, decode_errors, (), decode_status, fp_status) = tokio::try_join!(
                async {
                    read_limited(stdout, job.limits.stdout)
                        .await
                        .map_err(|_| AcoustidError::ProcessFailed)
                },
                async {
                    read_limited(stderr, job.limits.stderr)
                        .await
                        .map_err(|_| AcoustidError::ProcessFailed)
                },
                async {
                    read_limited(decoder_stderr, job.limits.stderr)
                        .await
                        .map_err(|_| AcoustidError::ProcessFailed)
                },
                verify_pcm(pcm, stdin, &input),
                async {
                    decoder
                        .wait()
                        .await
                        .map_err(|_| AcoustidError::ProcessFailed)
                },
                async {
                    fingerprint
                        .wait()
                        .await
                        .map_err(|_| AcoustidError::ProcessFailed)
                },
            )?;
            if output.1 || errors.1 || decode_errors.1 {
                return Err(AcoustidError::OutputTooLarge);
            }
            if !decode_status.success() || !fp_status.success() || !input.unchanged(path) {
                return Err(AcoustidError::ProcessFailed);
            }
            Ok(output.0)
        })
        .await
    }
    .await;
    if result.is_err() {
        let _ = decoder.kill().await;
        let _ = fingerprint.kill().await;
        let _ = decoder.wait().await;
        let _ = fingerprint.wait().await;
    }
    result
}

async fn verify_pcm(
    mut pcm: tokio::process::ChildStdout,
    mut stdin: tokio::process::ChildStdin,
    input: &FlacInput,
) -> Result<(), AcoustidError> {
    let mut hash = Md5::new();
    let mut bytes = 0_u64;
    let mut buffer = [0; 8192];
    loop {
        let count = pcm
            .read(&mut buffer)
            .await
            .map_err(|_| AcoustidError::ProcessFailed)?;
        if count == 0 {
            break;
        }
        bytes = bytes
            .checked_add(count as u64)
            .filter(|n| *n <= input.bytes)
            .ok_or(AcoustidError::ProcessFailed)?;
        stdin
            .write_all(&buffer[..count])
            .await
            .map_err(|_| AcoustidError::ProcessFailed)?;
        hash.update(&buffer[..count]);
    }
    stdin
        .shutdown()
        .await
        .map_err(|_| AcoustidError::ProcessFailed)?;
    if bytes != input.bytes || hash.finalize().as_slice() != input.md5 {
        return Err(AcoustidError::ProcessFailed);
    }
    Ok(())
}
