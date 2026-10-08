//! Application state shared by commands, the tray and the telemetry loop.

use std::sync::{Arc, Mutex, MutexGuard};

use crate::audio::devices::{self, InputInfo};
use crate::audio::engine::{Engine, EngineInfo};
use crate::audio::params::{Mode, Params};
use crate::audio::telemetry::Telemetry;
use crate::settings::{Settings, Store};
use crate::speaker::library::Store as VoiceStore;
use crate::speaker::service::Speakers;
use crate::virtual_mic;

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiState {
    pub inputs: Vec<InputInfo>,
    /// Device in use, or the saved choice when the engine is not running.
    pub selected_input: Option<String>,
    pub active: bool,
    pub muted: bool,
    pub mode: Mode,
    pub suppression: f32,
    pub gate_dbfs: f32,
    pub engine: Option<EngineInfo>,
    pub error: Option<String>,
    pub virtual_mic: virtual_mic::Status,
    /// What meeting apps call the virtual microphone on this platform.
    pub virtual_mic_name: &'static str,
    pub platform: &'static str,
}

pub struct AppState {
    pub params: Arc<Params>,
    pub telemetry: Arc<Telemetry>,
    pub speakers: Speakers,
    engine: Mutex<Option<Engine>>,
    settings: Mutex<Settings>,
    error: Mutex<Option<String>>,
    store: Store,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic while holding one of these locks leaves plain data behind; keep going.
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl AppState {
    pub fn new(store: Store, voices: VoiceStore) -> Self {
        let s = store.load();
        let params = Arc::new(Params::new(s.active, s.mode, s.suppression, s.gate_dbfs));
        Self {
            speakers: Speakers::start(voices, params.clone()),
            params,
            telemetry: Arc::new(Telemetry::default()),
            engine: Mutex::new(None),
            settings: Mutex::new(s),
            error: Mutex::new(None),
            store,
        }
    }

    /// (Re)start the engine on the saved input. The old engine is stopped first
    /// so the microphone is released before it is opened again.
    pub fn restart_engine(&self) {
        let mut engine = lock(&self.engine);
        *engine = None;
        let input_id = lock(&self.settings).input_id.clone();
        match Engine::start(input_id, self.params.clone(), self.telemetry.clone(), Some(self.speakers.link())) {
            Ok(e) => {
                *engine = Some(e);
                *lock(&self.error) = None;
            }
            Err(e) => {
                log::error!("engine start failed: {e:#}");
                *lock(&self.error) = Some(e.to_string());
            }
        }
    }

    /// Returns true if a running engine reported a failure and was shut down.
    pub fn reap_failed_engine(&self) -> bool {
        let mut engine = lock(&self.engine);
        let Some(msg) = engine.as_ref().and_then(Engine::failure) else { return false };
        *engine = None;
        *lock(&self.error) = Some(msg);
        true
    }

    pub fn engine_running(&self) -> bool {
        lock(&self.engine).is_some()
    }

    pub fn ui_state(&self) -> UiState {
        let saved = lock(&self.settings).input_id.clone();
        let engine = lock(&self.engine).as_ref().map(|e| e.info().clone());
        UiState {
            inputs: devices::list_inputs(),
            selected_input: engine.as_ref().map(|e| e.input_id.clone()).or(saved),
            active: self.params.active(),
            muted: self.params.muted(),
            mode: self.params.mode(),
            suppression: self.params.suppression(),
            gate_dbfs: self.params.gate_dbfs(),
            engine,
            error: lock(&self.error).clone(),
            virtual_mic: virtual_mic::status(),
            virtual_mic_name: devices::VIRTUAL_MIC_NAME,
            platform: std::env::consts::OS,
        }
    }

    fn update(&self, f: impl FnOnce(&mut Settings)) {
        let snapshot = {
            let mut s = lock(&self.settings);
            f(&mut s);
            s.clone()
        };
        self.store.save(&snapshot);
    }

    pub fn select_input(&self, id: String) {
        self.update(|s| s.input_id = Some(id));
        self.restart_engine();
    }

    pub fn set_active(&self, v: bool) {
        self.params.set_active(v);
        self.update(|s| s.active = v);
    }

    pub fn set_mode(&self, m: Mode) {
        self.params.set_mode(m);
        self.update(|s| s.mode = m);
    }

    pub fn set_suppression(&self, v: f32) {
        self.params.set_suppression(v);
        let v = self.params.suppression();
        self.update(|s| s.suppression = v);
    }

    pub fn set_gate(&self, v: f32) {
        self.params.set_gate_dbfs(v);
        let v = self.params.gate_dbfs();
        self.update(|s| s.gate_dbfs = v);
    }

    /// Muting is deliberately not persisted: Hush always starts with the microphone live.
    pub fn set_muted(&self, v: bool) {
        self.params.set_muted(v);
    }
}

