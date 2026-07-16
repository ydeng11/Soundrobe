//! Build identity + scaffold health-check command.

/// Static build identity returned by [`app_info`]. Mirrors the information the
/// Electron shell exposes implicitly; the renderer can probe it to detect which
/// native runtime backs `window.api` (Electron vs Tauri) during migration.
#[derive(serde::Serialize)]
pub struct AppInfo {
    /// Tauri shell identifier (`com.ihelio.autotagger`).
    pub identifier: &'static str,
    /// Crate version (kept in lockstep with `package.json`).
    pub version: &'static str,
    /// Native runtime the facade is bound to in this build.
    pub runtime: &'static str,
    /// `true` for the dev shell, `false` for production.
    pub dev: bool,
}

/// Scaffold health-check: `await window.api.appInfo()` via the adapter, or
/// `invoke("app_info")` directly. No Electron equivalent; this is the one
/// scaffold-only command and is the only command wired until slices land.
#[tauri::command]
pub async fn app_info() -> Result<AppInfo, crate::error::ApiError> {
    Ok(AppInfo {
        identifier: "com.ihelio.autotagger",
        version: env!("CARGO_PKG_VERSION"),
        runtime: "tauri",
        dev: cfg!(debug_assertions),
    })
}
