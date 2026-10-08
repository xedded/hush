//! Live parameters shared between the UI thread and the audio threads.
//! Everything is atomic so the audio path never blocks on a lock.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};

/// Strongest attenuation the suppression slider reaches, in dB.
pub const MAX_ATTENUATION_DB: f32 = 45.0;
pub const GATE_MIN_DBFS: f32 = -60.0;
pub const GATE_MAX_DBFS: f32 = -20.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Microphone passes through untouched.
    Off,
    /// Neural noise suppression plus voice gate.
    Noise,
    /// Only the enrolled voice. Until speaker recognition lands this behaves like `Noise`.
    Me,
}

impl Mode {
    fn to_u8(self) -> u8 {
        match self {
            Mode::Off => 0,
            Mode::Noise => 1,
            Mode::Me => 2,
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            0 => Mode::Off,
            2 => Mode::Me,
            _ => Mode::Noise,
        }
    }
}

#[derive(Default)]
struct AtomicF32(AtomicU32);

impl AtomicF32 {
    fn new(v: f32) -> Self {
        Self(AtomicU32::new(v.to_bits()))
    }
    fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
    fn store(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
    }
}

pub struct Params {
    active: AtomicBool,
    muted: AtomicBool,
    mode: AtomicU8,
    /// Suppression slider position, 0..=100.
    suppression: AtomicF32,
    gate_dbfs: AtomicF32,
}

impl Params {
    pub fn new(active: bool, mode: Mode, suppression: f32, gate_dbfs: f32) -> Self {
        let p = Self {
            active: AtomicBool::new(active),
            muted: AtomicBool::new(false),
            mode: AtomicU8::new(mode.to_u8()),
            suppression: AtomicF32::default(),
            gate_dbfs: AtomicF32::new(GATE_MIN_DBFS),
        };
        p.set_suppression(suppression);
        p.set_gate_dbfs(gate_dbfs);
        p
    }

    pub fn active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }
    pub fn set_active(&self, v: bool) {
        self.active.store(v, Ordering::Relaxed);
    }

    pub fn muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }
    pub fn set_muted(&self, v: bool) {
        self.muted.store(v, Ordering::Relaxed);
    }

    pub fn mode(&self) -> Mode {
        Mode::from_u8(self.mode.load(Ordering::Relaxed))
    }
    pub fn set_mode(&self, m: Mode) {
        self.mode.store(m.to_u8(), Ordering::Relaxed);
    }

    pub fn suppression(&self) -> f32 {
        self.suppression.load()
    }
    pub fn set_suppression(&self, v: f32) {
        let v = if v.is_finite() { v.clamp(0.0, 100.0) } else { 0.0 };
        self.suppression.store(v);
    }

    /// Attenuation limit handed to DeepFilterNet, derived from the slider.
    pub fn attenuation_db(&self) -> f32 {
        suppression_to_db(self.suppression())
    }

    pub fn gate_dbfs(&self) -> f32 {
        self.gate_dbfs.load()
    }
    pub fn set_gate_dbfs(&self, v: f32) {
        let v = if v.is_finite() { v.clamp(GATE_MIN_DBFS, GATE_MAX_DBFS) } else { GATE_MIN_DBFS };
        self.gate_dbfs.store(v);
    }

    /// True when the processing chain (suppression and gate) should run.
    pub fn processing(&self) -> bool {
        self.active() && self.mode() != Mode::Off
    }
}

pub fn suppression_to_db(slider: f32) -> f32 {
    slider.clamp(0.0, 100.0) / 100.0 * MAX_ATTENUATION_DB
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slider_maps_linearly_to_attenuation() {
        assert_eq!(suppression_to_db(0.0), 0.0);
        assert_eq!(suppression_to_db(100.0), MAX_ATTENUATION_DB);
        assert!((suppression_to_db(72.0) - 32.4).abs() < 1e-4);
    }

    #[test]
    fn setters_clamp_out_of_range_values() {
        let p = Params::new(true, Mode::Noise, 150.0, -90.0);
        assert_eq!(p.suppression(), 100.0);
        assert_eq!(p.gate_dbfs(), GATE_MIN_DBFS);
        p.set_gate_dbfs(f32::NAN);
        assert_eq!(p.gate_dbfs(), GATE_MIN_DBFS);
        p.set_suppression(-3.0);
        assert_eq!(p.suppression(), 0.0);
    }

    #[test]
    fn processing_requires_active_and_mode() {
        let p = Params::new(true, Mode::Noise, 50.0, -40.0);
        assert!(p.processing());
        p.set_mode(Mode::Off);
        assert!(!p.processing());
        p.set_mode(Mode::Me);
        p.set_active(false);
        assert!(!p.processing());
    }
}
