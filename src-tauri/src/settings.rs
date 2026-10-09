//! User settings persisted as JSON in the app config directory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::audio::enhance::EnhanceSettings;
use crate::audio::fx::FxSettings;
use crate::audio::params::Mode;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub input_id: Option<String>,
    pub active: bool,
    pub mode: Mode,
    pub suppression: f32,
    pub gate_dbfs: f32,
    /// The voice gate per microphone id: each microphone and room needs its own.
    pub gate_by_input: BTreeMap<String, f32>,
    pub voice_fx: FxSettings,
    pub voice_enhance: EnhanceSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            input_id: None,
            active: true,
            mode: Mode::Noise,
            suppression: 72.0,
            gate_dbfs: -42.0,
            gate_by_input: BTreeMap::new(),
            voice_fx: FxSettings::default(),
            voice_enhance: EnhanceSettings::default(),
        }
    }
}

pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(dir: &Path) -> Self {
        Self { path: dir.join("settings.json") }
    }

    /// Missing or unreadable settings fall back to defaults; a corrupt file is not fatal.
    pub fn load(&self) -> Settings {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                log::warn!("ignoring corrupt settings {}: {e}", self.path.display());
                Settings::default()
            }),
            Err(_) => Settings::default(),
        }
    }

    pub fn save(&self, s: &Settings) {
        let result = self
            .path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| {
                let json = serde_json::to_string_pretty(s).map_err(std::io::Error::other)?;
                let tmp = self.path.with_extension("json.tmp");
                std::fs::write(&tmp, json)?;
                std::fs::rename(&tmp, &self.path)
            });
        if let Err(e) = result {
            log::warn!("could not save settings to {}: {e}", self.path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hush-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn missing_file_gives_defaults() {
        assert_eq!(Store::new(&temp_dir("missing")).load(), Settings::default());
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = temp_dir("roundtrip");
        let store = Store::new(&dir);
        let s = Settings { input_id: Some("wasapi:{abc}".into()), mode: Mode::Me, suppression: 40.0, ..Settings::default() };
        store.save(&s);
        assert_eq!(store.load(), s);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_or_partial_file_is_tolerated() {
        let dir = temp_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::new(&dir);
        std::fs::write(dir.join("settings.json"), "{ not json").unwrap();
        assert_eq!(store.load(), Settings::default());
        std::fs::write(dir.join("settings.json"), r#"{"suppression": 10}"#).unwrap();
        assert_eq!(store.load().suppression, 10.0);
        assert_eq!(store.load().gate_dbfs, -42.0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
