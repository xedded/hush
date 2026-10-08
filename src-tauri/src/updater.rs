//! Automatic updates from GitHub Releases. Hush checks shortly after start and
//! then every few hours; the user decides when to install. Every update is
//! verified against the public key in tauri.conf.json before it runs.

use std::time::Duration;

use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

const FIRST_CHECK_AFTER: Duration = Duration::from_secs(15);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub notes: Option<String>,
}

async fn find(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater.check().await.map_err(|e| {
        log::warn!("update check failed: {e}");
        "Det gick inte att söka efter uppdateringar. Kontrollera internetanslutningen.".to_string()
    })?;
    Ok(update.map(|u| UpdateInfo { version: u.version, notes: u.body }))
}

/// Background checks; the UI shows a banner when "update-available" arrives.
pub fn spawn_checks(app: AppHandle) {
    std::thread::Builder::new()
        .name("hush-updates".into())
        .spawn(move || {
            std::thread::sleep(FIRST_CHECK_AFTER);
            loop {
                if let Ok(Some(info)) = tauri::async_runtime::block_on(find(&app)) {
                    let _ = app.emit("update-available", info);
                }
                std::thread::sleep(CHECK_EVERY);
            }
        })
        .expect("spawn update thread");
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    find(&app).await
}

/// Download, verify and install, then restart into the new version.
/// On Windows the installer asks for administrator rights.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Det finns ingen ny version.".to_string())?;
    update.download_and_install(|_, _| {}, || {}).await.map_err(|e| {
        log::error!("update install failed: {e}");
        "Uppdateringen kunde inte installeras.".to_string()
    })?;
    app.restart();
}
