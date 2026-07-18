//! Debug-mode state, JSONL persistence, and renderer event forwarding.

use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use tauri::{AppHandle, Emitter};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone)]
pub(crate) struct GeneralLogWriter {
    file: Arc<Mutex<File>>,
}

pub(crate) struct GeneralLogHandle {
    file: Arc<Mutex<File>>,
}

impl GeneralLogHandle {
    fn lock(&self) -> io::Result<MutexGuard<'_, File>> {
        self.file
            .lock()
            .map_err(|_| io::Error::other("general log mutex poisoned"))
    }
}

impl Write for GeneralLogHandle {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.lock()?.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.lock()?.flush()
    }
}

impl<'a> MakeWriter<'a> for GeneralLogWriter {
    type Writer = GeneralLogHandle;

    fn make_writer(&'a self) -> Self::Writer {
        GeneralLogHandle {
            file: Arc::clone(&self.file),
        }
    }
}

pub(crate) fn general_log_writer(
    home: &std::path::Path,
) -> io::Result<(PathBuf, GeneralLogWriter)> {
    let directory = home.join(".auto-tagger");
    fs::create_dir_all(&directory)?;
    let path = directory.join("auto-tagger.log");
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    Ok((
        path,
        GeneralLogWriter {
            file: Arc::new(Mutex::new(file)),
        },
    ))
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub tag: String,
    pub level: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug)]
struct DebugInner {
    enabled: bool,
    log_file: Option<PathBuf>,
    truncated: HashSet<PathBuf>,
}

#[derive(Debug)]
pub struct DebugState {
    log_dir: PathBuf,
    inner: Mutex<DebugInner>,
    forwarded: AtomicU64,
}

impl DebugState {
    pub fn new(home: PathBuf, enabled: bool) -> Self {
        let state = Self {
            log_dir: home.join(".auto-tagger"),
            inner: Mutex::new(DebugInner {
                enabled: false,
                log_file: None,
                truncated: HashSet::new(),
            }),
            forwarded: AtomicU64::new(0),
        };
        state.set_enabled(enabled);
        state
    }

    pub fn set_enabled(&self, enabled: bool) {
        let Ok(mut inner) = self.inner.lock() else {
            tracing::error!("debug state mutex poisoned");
            return;
        };
        inner.enabled = enabled;
        if !enabled {
            return;
        }
        let _ = fs::create_dir_all(&self.log_dir);
        let date = OffsetDateTime::now_utc().date().to_string();
        let path = self.log_dir.join(format!("auto-tag-debug-{date}.log"));
        if inner.truncated.insert(path.clone()) {
            let _ = fs::write(&path, []);
        }
        inner.log_file = Some(path);
    }

    pub fn enabled(&self) -> bool {
        self.inner
            .lock()
            .map(|inner| inner.enabled)
            .unwrap_or(false)
    }

    pub fn forwarded_count(&self) -> u64 {
        self.forwarded.load(Ordering::Acquire)
    }

    pub fn log_file(&self) -> Option<PathBuf> {
        self.inner
            .lock()
            .ok()
            .and_then(|inner| inner.log_file.clone())
    }

    pub fn emit(
        &self,
        app: &AppHandle,
        level: &str,
        tag: &str,
        message: impl Into<String>,
        data: Option<Value>,
    ) {
        let (enabled, log_file) = self
            .inner
            .lock()
            .map(|inner| (inner.enabled, inner.log_file.clone()))
            .unwrap_or((false, None));
        if !enabled {
            return;
        }
        let entry = LogEntry {
            timestamp: OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .unwrap_or_else(|_| OffsetDateTime::now_utc().unix_timestamp().to_string()),
            tag: tag.to_string(),
            level: level.to_string(),
            message: message.into(),
            data,
        };
        if let Some(path) = log_file {
            if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
                if let Ok(line) = serde_json::to_string(&entry) {
                    let _ = writeln!(file, "{line}");
                }
            }
        }
        self.forwarded.fetch_add(1, Ordering::AcqRel);
        let _ = app.emit("debug:log", entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn home() -> PathBuf {
        std::env::temp_dir().join(format!(
            "soundrobe-debug-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn enable_creates_daily_file_and_toggle_does_not_retruncate() {
        let home = home();
        let state = DebugState::new(home.clone(), false);
        assert!(!state.enabled());
        assert_eq!(state.log_file(), None);
        state.set_enabled(true);
        assert!(state.enabled());
        let path = state.log_file().unwrap();
        assert!(path.starts_with(home.join(".auto-tagger")));
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("auto-tag-debug-"));
        fs::write(&path, b"keep diagnostics\n").unwrap();
        state.set_enabled(false);
        assert!(!state.enabled());
        state.set_enabled(true);
        assert_eq!(fs::read(&path).unwrap(), b"keep diagnostics\n");
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn initially_enabled_state_prepares_log_file() {
        let home = home();
        let state = DebugState::new(home.clone(), true);
        assert!(state.enabled());
        assert!(state.log_file().unwrap().exists());
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn general_log_writer_appends_in_place_without_truncating() {
        let home = home();
        let directory = home.join(".auto-tagger");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("auto-tagger.log");
        fs::write(&path, b"previous session\n").unwrap();

        let (actual_path, writer) = general_log_writer(&home).unwrap();
        let mut handle = writer.make_writer();
        handle.write_all(b"current session\n").unwrap();
        handle.flush().unwrap();

        assert_eq!(actual_path, path);
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "previous session\ncurrent session\n"
        );
        fs::remove_dir_all(home).unwrap();
    }
}
