//! Commands invoked from the UI. All are async so device work and model
//! loading never run on the main (window) thread.

use std::sync::Arc;

use tauri::{AppHandle, State};

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

#[tauri::command]
pub async fn set_muted(app: AppHandle, state: AppStateRef<'_>, value: bool) -> Result<(), String> {
    state.set_muted(value);
    tray::sync_mute(&app, value);
    Ok(())
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
