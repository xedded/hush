mod audio;
mod commands;
mod settings;
mod speaker;
mod state;
mod tray;
mod updater;
pub mod virtual_mic;
#[cfg(windows)]
mod win_endpoint;

use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager, WindowEvent};

use state::AppState;

const TELEMETRY_INTERVAL: Duration = Duration::from_millis(33);
/// How often a stopped engine is retried, e.g. after the headset was unplugged.
const RETRY_INTERVAL: Duration = Duration::from_secs(5);
/// How often the virtual cable's install state is re-read.
const MIC_CHECK_INTERVAL: Duration = Duration::from_secs(3);

/// One telemetry event: audio levels plus who is speaking ("me", a voice id or "unknown").
#[derive(Clone, serde::Serialize)]
struct Tick {
    #[serde(flatten)]
    levels: audio::telemetry::Snapshot,
    speaking: Option<String>,
    /// "Listen to yourself" is playing; it switches itself off if the headphones go away.
    monitoring: bool,
}

/// Push levels to the UI and restart the engine after device loss.
fn spawn_monitor(app: AppHandle, state: Arc<AppState>) {
    std::thread::Builder::new()
        .name("hush-monitor".into())
        .spawn(move || {
            let mut last_retry = Instant::now();
            let mut last_mic_check = Instant::now();
            let mut mic_status = virtual_mic::status();
            let mut voices_version = 0;
            loop {
                std::thread::sleep(TELEMETRY_INTERVAL);
                // Hide or show the setup panel as soon as the virtual cable appears or goes away.
                if last_mic_check.elapsed() >= MIC_CHECK_INTERVAL {
                    last_mic_check = Instant::now();
                    let now = virtual_mic::status();
                    if now != mic_status {
                        mic_status = now;
                        let _ = app.emit("state", state.ui_state());
                    }
                }
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
                let version = state.speakers.version();
                if version != voices_version {
                    voices_version = version;
                    let _ = app.emit("voices", state.speakers.view());
                }
                if let Some(result) = state.speakers.take_enroll_result() {
                    let _ = app.emit("enrollment-done", result);
                }
                let _ = app.emit("telemetry", Tick {
                    levels: state.telemetry.take(),
                    speaking: state.speakers.speaking(),
                    monitoring: state.params.monitor(),
                });
            }
        })
        .expect("spawn monitor thread");
}

/// Log to a file in the app log folder (and the console in development).
/// Two files of 2 MB are kept, enough for days of normal use.
fn log_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};
    tauri_plugin_log::Builder::new()
        .targets([Target::new(TargetKind::LogDir { file_name: Some("hush".into()) }), Target::new(TargetKind::Stdout)])
        .level(log::LevelFilter::Info)
        // The model runtime is chatty below warnings.
        .level_for("tract_core", log::LevelFilter::Warn)
        .level_for("tract_onnx", log::LevelFilter::Warn)
        .level_for("tract_hir", log::LevelFilter::Warn)
        .max_file_size(2_000_000)
        .rotation_strategy(RotationStrategy::KeepSome(2))
        .timezone_strategy(TimezoneStrategy::UseLocal)
        .build()
}

/// Panics end up in the log file too, so a crash can be diagnosed afterwards.
fn log_panics() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panic: {info}");
        default(info);
    }));
}

/// Passed by the login item: start in the tray without showing the window.
const HIDDEN_ARG: &str = "--hidden";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(log_plugin())
        .plugin(tray::shortcut_plugin())
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Started at login with --hidden: Hush goes straight to the tray.
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec![HIDDEN_ARG])))
        .setup(|app| {
            log_panics();
            log::info!("Hush {} on {} {}", app.package_info().version, std::env::consts::OS, std::env::consts::ARCH);
            let dir = app.path().app_config_dir()?;
            let voices = speaker::library::Store::new(&app.path().app_data_dir()?, Box::new(speaker::library::Keychain));
            let state = Arc::new(AppState::new(settings::Store::new(&dir), voices));
            app.manage(state.clone());

            tray::install(app.handle())?;
            // The window starts hidden (tauri.conf.json) so a login start never flashes it.
            if !std::env::args().any(|a| a == HIDDEN_ARG) {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                }
            }
            tray::register_shortcuts(app.handle());
            updater::spawn_checks(app.handle().clone());

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
            commands::get_autostart,
            commands::open_log_folder,
            commands::set_autostart,
            commands::select_input,
            commands::restart_engine,
            commands::set_active,
            commands::set_mode,
            commands::set_suppression,
            commands::set_gate,
            commands::measure_levels,
            commands::apply_gate_calibration,
            commands::set_muted,
            commands::open_vbcable_page,
            commands::rename_virtual_mic,
            commands::get_voices,
            commands::rename_voice,
            commands::set_voice_default,
            commands::delete_voice,
            commands::set_unknown_policy,
            commands::start_enrollment,
            commands::cancel_enrollment,
            commands::get_stats,
            commands::reset_stats,
            commands::set_voice_fx,
            commands::set_monitor,
            commands::set_voice_enhance,
            updater::check_update,
            updater::install_update,
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
