//! Tray icon and the global mute shortcut. Hush keeps running in the tray when
//! the window is closed, since meetings need the virtual microphone alive.

use std::sync::Arc;

use tauri::menu::{CheckMenuItem, MenuBuilder, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::state::AppState;

pub const MUTE_SHORTCUT: (Modifiers, Code) = (Modifiers::CONTROL.union(Modifiers::ALT), Code::KeyM);

struct TrayItems {
    mute: CheckMenuItem<Wry>,
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Keep the tray checkbox in step when mute changes elsewhere.
pub fn sync_mute(app: &AppHandle, muted: bool) {
    if let Some(items) = app.try_state::<TrayItems>() {
        let _ = items.mute.set_checked(muted);
    }
}

fn toggle_mute(app: &AppHandle) {
    let state = app.state::<Arc<AppState>>();
    let muted = !state.params.muted();
    state.set_muted(muted);
    sync_mute(app, muted);
    let _ = app.emit("state", state.ui_state());
}

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Öppna Hush", true, None::<&str>)?;
    let mute = CheckMenuItem::with_id(app, "mute", "Stäng av mikrofon", true, false, Some("Ctrl+Alt+M"))?;
    let quit = MenuItem::with_id(app, "quit", "Avsluta Hush", true, None::<&str>)?;
    let menu = MenuBuilder::new(app).item(&open).item(&mute).separator().item(&quit).build()?;

    let icon = app.default_window_icon().cloned().ok_or_else(|| tauri::Error::AssetNotFound("icon".into()))?;
    TrayIconBuilder::with_id("hush")
        .icon(icon)
        .tooltip("Hush")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main(app),
            "mute" => toggle_mute(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;
    app.manage(TrayItems { mute });
    Ok(())
}

pub fn shortcut_plugin() -> tauri::plugin::TauriPlugin<Wry> {
    let mute = Shortcut::new(Some(MUTE_SHORTCUT.0), MUTE_SHORTCUT.1);
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(move |app, shortcut, event| {
            if event.state() == ShortcutState::Pressed && shortcut == &mute {
                toggle_mute(app);
            }
        })
        .build()
}

pub fn register_shortcuts(app: &AppHandle) {
    let mute = Shortcut::new(Some(MUTE_SHORTCUT.0), MUTE_SHORTCUT.1);
    if let Err(e) = app.global_shortcut().register(mute) {
        // Another program may own the combination; Hush still works without it.
        log::warn!("could not register Ctrl+Alt+M: {e}");
    }
}
