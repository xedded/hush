//! Commands invoked from the UI. All are async so device work and model
//! loading never run on the main (window) thread.

use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::audio::enhance::EnhanceSettings;
use crate::audio::fx::FxSettings;
use crate::audio::params::Mode;
use crate::state::{AppState, UiState};
use crate::tray;

type AppStateRef<'a> = State<'a, Arc<AppState>>;

#[tauri::command]
pub async fn get_state(state: AppStateRef<'_>) -> Result<UiState, String> {
    Ok(state.ui_state())
}

#[tauri::command]
pub async fn select_input(state: AppStateRef<'_>, id: String) -> Result<UiState, String> {
    if id.trim().is_empty() {
        return Err("Ogiltig mikrofon.".into());
    }
    state.select_input(id);
    Ok(state.ui_state())
}

#[tauri::command]
pub async fn restart_engine(state: AppStateRef<'_>) -> Result<UiState, String> {
    state.restart_engine();
    Ok(state.ui_state())
}

#[tauri::command]
pub async fn set_active(state: AppStateRef<'_>, value: bool) -> Result<(), String> {
    state.set_active(value);
    Ok(())
}

#[tauri::command]
pub async fn set_mode(state: AppStateRef<'_>, value: Mode) -> Result<(), String> {
    state.set_mode(value);
    Ok(())
}

#[tauri::command]
pub async fn set_suppression(state: AppStateRef<'_>, value: f32) -> Result<(), String> {
    state.set_suppression(value);
    Ok(())
}

#[tauri::command]
pub async fn set_gate(state: AppStateRef<'_>, value: f32) -> Result<(), String> {
    state.set_gate(value);
    Ok(())
}

/// One calibration phase: the gate detector's level every 10 ms for `seconds`.
#[tauri::command]
pub async fn measure_levels(state: AppStateRef<'_>, seconds: f32) -> Result<Vec<f32>, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.measure_levels(seconds))
        .await
        .map_err(|e| e.to_string())?
}

/// Set the voice gate from a quiet phase and a speaking phase; returns the new threshold.
#[tauri::command]
pub async fn apply_gate_calibration(state: AppStateRef<'_>, quiet: Vec<f32>, speech: Vec<f32>) -> Result<f32, String> {
    let threshold = crate::audio::calibrate::suggest(&quiet, &speech)?;
    log::info!("voice gate calibrated to {threshold} dBFS");
    Ok(state.set_gate(threshold))
}

#[tauri::command]
pub async fn set_muted(app: AppHandle, state: AppStateRef<'_>, value: bool) -> Result<(), String> {
    state.set_muted(value);
    tray::sync_mute(&app, value);
    Ok(())
}

#[tauri::command]
pub async fn set_voice_fx(state: AppStateRef<'_>, value: FxSettings) -> Result<FxSettings, String> {
    state.set_voice_fx(value);
    Ok(state.params.fx())
}

#[tauri::command]
pub async fn set_voice_enhance(state: AppStateRef<'_>, value: EnhanceSettings) -> Result<EnhanceSettings, String> {
    state.set_voice_enhance(value);
    Ok(state.params.enhance())
}

/// Play the outgoing sound on the default playback device.
#[tauri::command]
pub async fn set_monitor(state: AppStateRef<'_>, value: bool) -> Result<(), String> {
    state.set_monitor(value)
}

/// Show the log folder in Explorer or Finder, for sending a log when something goes wrong.
#[tauri::command]
pub async fn open_log_folder(app: AppHandle) -> Result<(), String> {
    use tauri::Manager;
    let dir = app.path().app_log_dir().map_err(|e| e.to_string())?;
    let opener = if cfg!(target_os = "macos") { "open" } else if cfg!(windows) { "explorer" } else { "xdg-open" };
    std::process::Command::new(opener).arg(&dir).spawn().map(|_| ()).map_err(|e| {
        log::warn!("could not open {}: {e}", dir.display());
        "Loggmappen kunde inte öppnas.".to_string()
    })
}

#[tauri::command]
pub async fn get_autostart(app: AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

/// Start Hush in the tray when the user logs in.
#[tauri::command]
pub async fn set_autostart(app: AppHandle, value: bool) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    let launcher = app.autolaunch();
    let result = if value { launcher.enable() } else { launcher.disable() };
    result.map_err(|e| {
        log::warn!("autostart: {e}");
        "Autostart kunde inte ändras.".to_string()
    })?;
    launcher.is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_vbcable_page() -> Result<(), String> {
    crate::virtual_mic::open_download_page().map_err(|_| "Webbläsaren kunde inte öppnas.".to_string())
}

/// Rename "CABLE Output" to "Hush Microphone". Asks for administrator rights.
#[tauri::command]
pub async fn rename_virtual_mic(state: AppStateRef<'_>) -> Result<UiState, String> {
    crate::virtual_mic::rename_elevated().map_err(|e| {
        log::warn!("rename failed: {e:#}");
        "Namnet kunde inte ändras. Administratörsbehörighet krävs.".to_string()
    })?;
    Ok(state.ui_state())
}

// ---------- Voices ----------

use crate::speaker::library::Policy;
use crate::speaker::service::VoicesView;

fn voice_result(state: &AppState, r: anyhow::Result<()>) -> Result<VoicesView, String> {
    r.map_err(|e| e.to_string())?;
    Ok(state.speakers.view())
}

#[tauri::command]
pub async fn get_voices(state: AppStateRef<'_>) -> Result<VoicesView, String> {
    Ok(state.speakers.view())
}

#[tauri::command]
pub async fn rename_voice(state: AppStateRef<'_>, id: String, name: String) -> Result<VoicesView, String> {
    voice_result(&state, state.speakers.rename(&id, &name))
}

#[tauri::command]
pub async fn set_voice_default(state: AppStateRef<'_>, id: String, policy: Policy) -> Result<VoicesView, String> {
    voice_result(&state, state.speakers.set_default(&id, policy))
}

#[tauri::command]
pub async fn delete_voice(state: AppStateRef<'_>, id: String) -> Result<VoicesView, String> {
    voice_result(&state, state.speakers.delete(&id))
}

#[tauri::command]
pub async fn set_unknown_policy(state: AppStateRef<'_>, policy: Policy) -> Result<VoicesView, String> {
    voice_result(&state, state.speakers.set_unknown(policy))
}

/// Recording needs the denoiser running, so the profile matches what Hush hears later.
#[tauri::command]
pub async fn start_enrollment(state: AppStateRef<'_>) -> Result<VoicesView, String> {
    if !state.engine_running() || !state.params.processing() {
        return Err("Slå på Hush och välj Dämpa bakgrundsljud innan du spelar in.".into());
    }
    state.speakers.start_enrollment();
    Ok(state.speakers.view())
}

#[tauri::command]
pub async fn cancel_enrollment(state: AppStateRef<'_>) -> Result<VoicesView, String> {
    state.speakers.cancel_enrollment();
    Ok(state.speakers.view())
}

// ---------- Statistics ----------

#[tauri::command]
pub async fn get_stats(state: AppStateRef<'_>) -> Result<crate::speaker::stats::StatsView, String> {
    Ok(state.speakers.stats())
}

#[tauri::command]
pub async fn reset_stats(state: AppStateRef<'_>) -> Result<crate::speaker::stats::StatsView, String> {
    state.speakers.reset_stats().map_err(|e| e.to_string())?;
    Ok(state.speakers.stats())
}
