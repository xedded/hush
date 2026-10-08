mod audio;
mod commands;
mod settings;
mod state;
mod tray;
pub mod virtual_mic;

use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager, WindowEvent};

use state::AppState;

const TELEMETRY_INTERVAL: Duration = Duration::from_millis(33);
/// How often a stopped engine is retried, e.g. after the headset was unplugged.
const RETRY_INTERVAL: Duration = Duration::from_secs(5);

/// Push levels to the UI and restart the engine after device loss.
fn spawn_monitor(app: AppHandle, state: Arc<AppState>) {
    std::thread::Builder::new()
        .name("hush-monitor".into())
        .spawn(move || {
            let mut last_retry = Instant::now();
            loop {
                std::thread::sleep(TELEMETRY_INTERVAL);
                if state.reap_failed_engine() {
                    last_retry = Instant::now();
                    let _ = app.emit("state", state.ui_state());
                }
                if !state.engine_running() && last_retry.elapsed() >= RETRY_INTERVAL {
                    last_retry = Instant::now();
                    state.restart_engine();
                    if state.engine_running() {
                        let _ = app.emit("state", state.ui_state());
                    }
                }
                let _ = app.emit("telemetry", state.telemetry.take());
            }
        })
        .expect("spawn monitor thread");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tray::shortcut_plugin())
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            let state = Arc::new(AppState::new(settings::Store::new(&dir)));
            app.manage(state.clone());

            tray::install(app.handle())?;
            tray::register_shortcuts(app.handle());

            // Loading the model takes a moment; let the window appear first.
            let (handle, starter) = (app.handle().clone(), state.clone());
            std::thread::spawn(move || {
                starter.restart_engine();
                let _ = handle.emit("state", starter.ui_state());
                spawn_monitor(handle, starter);
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window keeps Hush running in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::select_input,
            commands::restart_engine,
            commands::set_active,
            commands::set_mode,
            commands::set_suppression,
            commands::set_gate,
            commands::set_muted,
            commands::open_vbcable_page,
            commands::rename_virtual_mic,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Hush");
}

/// Installer entry points (`--name-virtual-mic` / `--restore-virtual-mic`, run elevated).
/// Exit code 0 on success or when there is nothing to do.
pub fn name_virtual_mic(restore: bool) -> i32 {
    let result = if restore { virtual_mic::restore_name() } else { virtual_mic::ensure_named() };
    match result {
        Ok(outcome) => {
            println!("{outcome:?}");
            0
        }
        Err(e) => {
            eprintln!("{e:#}");
            1
        }
    }
}
