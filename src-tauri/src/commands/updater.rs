use crate::error::ApiError;
use crate::state::updater::UpdaterState;
use crate::state::write_queue::WriteQueue;
use serde::Serialize;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::UpdaterExt;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateInfo {
    current_version: String,
    available_version: String,
    date: Option<String>,
    notes: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppUpdateProgress {
    phase: &'static str,
    downloaded: u64,
    total: Option<u64>,
}

fn updater_enabled(
    debug_build: bool,
    target_os: &str,
    bundle_type_known: bool,
    executable: &Path,
) -> bool {
    if debug_build {
        return false;
    }
    if target_os == "macos" {
        return executable
            .ancestors()
            .any(|path| path.extension().is_some_and(|extension| extension == "app"));
    }
    bundle_type_known
}

fn packaged_production_app() -> bool {
    let executable = match std::env::current_exe() {
        Ok(path) => path,
        Err(_) => return false,
    };
    updater_enabled(
        cfg!(debug_assertions),
        std::env::consts::OS,
        tauri::utils::platform::bundle_type().is_some(),
        &executable,
    )
}

fn update_info(update: &tauri_plugin_updater::Update) -> AppUpdateInfo {
    AppUpdateInfo {
        current_version: update.current_version.clone(),
        available_version: update.version.clone(),
        date: update.date.and_then(|date| {
            date.format(&time::format_description::well_known::Rfc3339)
                .ok()
        }),
        notes: update.body.clone(),
    }
}

#[tauri::command]
pub async fn updater_check(
    app: AppHandle,
    state: State<'_, UpdaterState>,
) -> Result<Option<AppUpdateInfo>, ApiError> {
    if !packaged_production_app() {
        return Err(ApiError::Message(
            "Update checks are disabled outside packaged production builds".into(),
        ));
    }

    let _check = state.lock_check().await;
    let update = app
        .updater()
        .map_err(|error| ApiError::Message(format!("failed to initialize updater: {error}")))?
        .check()
        .await
        .map_err(|error| ApiError::Message(format!("failed to check for updates: {error}")))?;
    let info = update.as_ref().map(update_info);
    state
        .replace(update)
        .await
        .map_err(|message| ApiError::Message(message.into()))?;
    Ok(info)
}

#[tauri::command]
pub async fn updater_install(
    app: AppHandle,
    state: State<'_, UpdaterState>,
    queue: State<'_, WriteQueue>,
) -> Result<(), ApiError> {
    let update = state
        .begin_install()
        .await
        .map_err(|message| ApiError::Message(message.into()))?;
    let progress_app = app.clone();
    let finish_app = app.clone();
    let downloaded = Arc::new(AtomicU64::new(0));
    let total = Arc::new(AtomicU64::new(0));
    let total_known = Arc::new(AtomicBool::new(false));
    let chunk_downloaded = Arc::clone(&downloaded);
    let chunk_total = Arc::clone(&total);
    let chunk_total_known = Arc::clone(&total_known);
    let finish_downloaded = Arc::clone(&downloaded);
    let finish_total = Arc::clone(&total);
    let finish_total_known = Arc::clone(&total_known);

    let installation = queue
        .try_run_exclusive(async move {
            update
                .download_and_install(
                    move |chunk_length, content_length| {
                        let current = chunk_downloaded
                            .fetch_add(chunk_length as u64, Ordering::AcqRel)
                            + chunk_length as u64;
                        if let Some(content_length) = content_length {
                            chunk_total.store(content_length, Ordering::Release);
                            chunk_total_known.store(true, Ordering::Release);
                        }
                        let _ = progress_app.emit(
                            "updater:progress",
                            AppUpdateProgress {
                                phase: "downloading",
                                downloaded: current,
                                total: chunk_total_known
                                    .load(Ordering::Acquire)
                                    .then(|| chunk_total.load(Ordering::Acquire)),
                            },
                        );
                    },
                    move || {
                        let _ = finish_app.emit(
                            "updater:progress",
                            AppUpdateProgress {
                                phase: "installing",
                                downloaded: finish_downloaded.load(Ordering::Acquire),
                                total: finish_total_known
                                    .load(Ordering::Acquire)
                                    .then(|| finish_total.load(Ordering::Acquire)),
                            },
                        );
                    },
                )
                .await
        })
        .await;

    let result = match installation {
        Some(Ok(())) => Ok(()),
        Some(Err(error)) => Err(ApiError::Message(format!(
            "failed to download or install update: {error}"
        ))),
        None => Err(ApiError::Message(
            "Cannot install an update while a protected disk operation is active".into(),
        )),
    };
    state.finish_install(result.is_ok()).await;
    result?;

    #[cfg(not(target_os = "windows"))]
    app.restart();

    #[cfg(target_os = "windows")]
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::updater_enabled;
    use std::path::Path;

    #[test]
    fn updater_is_disabled_for_all_debug_builds() {
        assert!(!updater_enabled(
            true,
            "linux",
            true,
            Path::new("/opt/Soundrobe/soundrobe")
        ));
    }

    #[test]
    fn updater_requires_an_installed_bundle() {
        assert!(!updater_enabled(
            false,
            "linux",
            false,
            Path::new("/work/target/release/soundrobe")
        ));
        assert!(updater_enabled(
            false,
            "linux",
            true,
            Path::new("/usr/bin/soundrobe")
        ));
        assert!(!updater_enabled(
            false,
            "macos",
            true,
            Path::new("/work/target/release/soundrobe")
        ));
        assert!(updater_enabled(
            false,
            "macos",
            true,
            Path::new("/Applications/Soundrobe.app/Contents/MacOS/soundrobe")
        ));
    }
}
