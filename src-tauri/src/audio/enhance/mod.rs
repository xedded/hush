//! Voice enhancement, the kind of processing that ships with studio
//! microphones: low cut -> tone (EQ) -> de-esser -> compressor -> automatic
//! level -> limiter. Runs after noise suppression. Every stage is a minimum
//! phase filter or a gain, so it adds no delay.

pub mod dynamics;

use crate::audio::biquad::{Biquad, BUTTERWORTH_Q};
use dynamics::{AutoLevel, Compressor, DeEsser, Limiter};

const FADE_MS: f32 = 20.0;
const EQ_BANDS: usize = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Preset {
    /// Low cut and level only: you, just steadier.
    Natural,
    /// The full chain tuned for speech intelligibility.
    #[default]
    Clear,
    /// More body and softer highs, like a podcast voice.
    Warm,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EnhanceSettings {
    pub enabled: bool,
    pub preset: Preset,
    /// Strength, 0..=100.
    pub amount: f32,
}

impl Default for EnhanceSettings {
    fn default() -> Self {
        Self { enabled: false, preset: Preset::Clear, amount: 50.0 }
    }
}

impl EnhanceSettings {
    pub fn sanitized(self) -> Self {
        let amount = if self.amount.is_finite() { self.amount.clamp(0.0, 100.0) } else { 50.0 };
        Self { amount, ..self }
    }
}

#[derive(Clone, Copy)]
enum Band {
    Peak { hz: f32, q: f32, db: f32 },
    LowShelf { hz: f32, db: f32 },
    HighShelf { hz: f32, db: f32 },
}

/// Everything a preset sets, at a given strength (0..=1).
struct Design {
    low_cut_hz: f32,
    eq: [Band; EQ_BANDS],
    de_ess_db: f32,
    threshold_db: f32,
    ratio: f32,
}

const FLAT: Band = Band::Peak { hz: 1_000.0, q: 1.0, db: 0.0 };

fn design(preset: Preset, s: f32) -> Design {
    match preset {
        Preset::Natural => Design { low_cut_hz: 80.0, eq: [FLAT; EQ_BANDS], de_ess_db: 0.0, threshold_db: 0.0, ratio: 1.0 },
        Preset::Clear => Design {
            low_cut_hz: 100.0,
            eq: [
                // Less boxiness, more presence.
                Band::Peak { hz: 300.0, q: 1.0, db: -4.0 * s },
                Band::Peak { hz: 4_000.0, q: 0.8, db: 5.0 * s },
                Band::HighShelf { hz: 10_000.0, db: 1.5 * s },
            ],
            de_ess_db: 8.0 * s,
            threshold_db: -26.0,
            ratio: 1.0 + 2.5 * s,
        },
        Preset::Warm => Design {
            low_cut_hz: 80.0,
            eq: [
                Band::LowShelf { hz: 200.0, db: 3.5 * s },
                Band::Peak { hz: 3_000.0, q: 0.9, db: 1.5 * s },
                Band::HighShelf { hz: 8_000.0, db: -3.0 * s },
            ],
            de_ess_db: 6.0 * s,
            threshold_db: -28.0,
            ratio: 1.0 + 3.0 * s,
        },
    }
}

fn band_filter(rate: f32, band: Band) -> Biquad {
    match band {
        Band::Peak { hz, q, db } => Biquad::peaking(rate, hz, q, db),
        Band::LowShelf { hz, db } => Biquad::low_shelf(rate, hz, db),
        Band::HighShelf { hz, db } => Biquad::high_shelf(rate, hz, db),
    }
}

pub struct Enhancer {
    rate: f32,
    low_cut: [Biquad; 2],
    eq: [Biquad; EQ_BANDS],
    de_esser: DeEsser,
    compressor: Compressor,
    level: AutoLevel,
    limiter: Limiter,
    /// Settings the filters are tuned to.
    tuned: Option<(Preset, f32)>,
    mix: f32,
    step: f32,
    idle: bool,
    wet: Vec<f32>,
}

impl Enhancer {
    pub fn new(rate: u32) -> Self {
        let rate = rate as f32;
        Self {
            rate,
            low_cut: [Biquad::identity(); 2],
            eq: [Biquad::identity(); EQ_BANDS],
            de_esser: DeEsser::new(rate),
            compressor: Compressor::new(rate),
            level: AutoLevel::new(rate),
            limiter: Limiter::new(rate),
            tuned: None,
            mix: 0.0,
            step: 1.0 / (FADE_MS / 1000.0 * rate),
            idle: true,
            wet: Vec::with_capacity(4_096),
        }
    }

    fn tune(&mut self, preset: Preset, amount: f32) {
        if self.tuned == Some((preset, amount)) {
            return;
        }
        self.tuned = Some((preset, amount));
        let d = design(preset, amount / 100.0);
        // Two Butterworth sections: a steep 24 dB per octave cut.
        let cut = Biquad::high_pass(self.rate, d.low_cut_hz, BUTTERWORTH_Q);
        self.low_cut.iter_mut().for_each(|f| f.retune(cut));
        for (f, band) in self.eq.iter_mut().zip(d.eq) {
            f.retune(band_filter(self.rate, band));
        }
        self.de_esser.set_depth_db(d.de_ess_db);
        self.compressor.configure(d.threshold_db, d.ratio);
    }

    fn reset(&mut self) {
        self.low_cut.iter_mut().chain(self.eq.iter_mut()).for_each(Biquad::reset);
        self.de_esser.reset();
        self.compressor.reset();
        self.level.reset();
        self.limiter.reset();
        self.mix = 0.0;
        self.idle = true;
    }

    /// Enhance a block in place. `speech` tells the automatic level when to
    /// learn: only while the voice gate is open and the speaker may pass.
    pub fn process(&mut self, block: &mut [f32], s: &EnhanceSettings, speech: bool) {
        if !s.enabled && self.mix == 0.0 {
            if !self.idle {
                self.reset();
            }
            return;
        }
        self.idle = false;
        self.tune(s.preset, s.amount);
        self.wet.clear();
        self.wet.extend_from_slice(block);
        for v in self.wet.iter_mut() {
            let mut x = self.low_cut.iter_mut().chain(self.eq.iter_mut()).fold(*v, |x, f| f.tick(x));
            x = self.de_esser.tick(x);
            *v = self.compressor.tick(x);
        }
        self.level.process(&mut self.wet, speech);
        for v in self.wet.iter_mut() {
            *v = self.limiter.tick(*v);
        }
        if self.wet.iter().any(|v| !v.is_finite()) {
            log::warn!("voice enhancement produced invalid audio; restarting it");
            self.reset();
            block.iter_mut().filter(|v| !v.is_finite()).for_each(|v| *v = 0.0);
            return;
        }
        let target = if s.enabled { 1.0 } else { 0.0 };
        for (dry, wet) in block.iter_mut().zip(&self.wet) {
            self.mix = if target > self.mix { (self.mix + self.step).min(1.0) } else { (self.mix - self.step).max(0.0) };
            *dry = *dry * (1.0 - self.mix) + wet * self.mix;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::fx::shifter::tests::{buzz, power_at, sine};

    const RATE: u32 = 48_000;

    fn run(e: &mut Enhancer, input: &[f32], s: &EnhanceSettings) -> Vec<f32> {
        let mut out = input.to_vec();
        out.chunks_mut(480).for_each(|b| e.process(b, s, true));
        out
    }

    fn on(preset: Preset, amount: f32) -> EnhanceSettings {
        EnhanceSettings { enabled: true, preset, amount }
    }

    #[test]
    fn switched_off_is_bit_exact() {
        let input = buzz(140.0, 0.3);
        assert_eq!(run(&mut Enhancer::new(RATE), &input, &EnhanceSettings::default()), input);
    }

    #[test]
    fn every_preset_cuts_rumble() {
        for preset in [Preset::Natural, Preset::Clear, Preset::Warm] {
            let input: Vec<f32> = sine(40.0, 2.0, 0.05).iter().zip(sine(500.0, 2.0, 0.05)).map(|(a, b)| a + b).collect();
            let out = run(&mut Enhancer::new(RATE), &input, &on(preset, 50.0));
            let tail = &out[48_000..];
            let rumble = 10.0 * (power_at(tail, 40.0) / power_at(tail, 500.0)).log10();
            assert!(rumble < -20.0, "{preset:?}: rumble only {rumble:.1} dB below the voice");
        }
    }

    /// Ratio of presence (4 kHz) to body (300 Hz) after processing, in dB.
    fn presence_db(preset: Preset, amount: f32) -> f32 {
        let input: Vec<f32> = sine(300.0, 2.0, 0.05).iter().zip(sine(4_000.0, 2.0, 0.05)).map(|(a, b)| a + b).collect();
        let out = run(&mut Enhancer::new(RATE), &input, &on(preset, amount));
        let tail = &out[48_000..];
        10.0 * (power_at(tail, 4_000.0) / power_at(tail, 300.0)).log10()
    }

    #[test]
    fn clear_adds_presence_and_warm_adds_body() {
        let (natural, clear, warm) = (presence_db(Preset::Natural, 100.0), presence_db(Preset::Clear, 100.0), presence_db(Preset::Warm, 100.0));
        println!("presence: natural {natural:.1}, clear {clear:.1}, warm {warm:.1} dB");
        assert!(clear > natural + 5.0);
        assert!(warm < clear);
    }

    #[test]
    fn strength_scales_the_effect() {
        assert!(presence_db(Preset::Clear, 100.0) > presence_db(Preset::Clear, 30.0) + 3.0);
    }

    #[test]
    fn quiet_speech_comes_up_and_peaks_stay_below_the_ceiling() {
        let quiet = buzz(140.0, 4.0).iter().map(|v| v * 0.1).collect::<Vec<_>>();
        let out = run(&mut Enhancer::new(RATE), &quiet, &on(Preset::Clear, 50.0));
        let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
        assert!(rms(&out[144_000..]) > 2.0 * rms(&quiet[144_000..]));
        let loud = buzz(140.0, 1.0).iter().map(|v| v * 6.0).collect::<Vec<_>>();
        let out = run(&mut Enhancer::new(RATE), &loud, &on(Preset::Warm, 100.0));
        // After the 20 ms fade-in, when only the processed signal is heard.
        assert!(out[960..].iter().all(|v| v.abs() <= dynamics::CEILING + 1e-4));
    }

    #[test]
    fn changing_settings_mid_stream_does_not_click() {
        let input = buzz(140.0, 1.0);
        let mut e = Enhancer::new(RATE);
        let mut out = run(&mut e, &input[..24_000], &on(Preset::Clear, 80.0));
        out.extend(run(&mut e, &input[24_000..36_000], &on(Preset::Warm, 20.0)));
        out.extend(run(&mut e, &input[36_000..], &EnhanceSettings::default()));
        let step = |x: &[f32]| x.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        assert!(step(&out[4_800..]) < 3.0 * step(&input), "jump {} vs {}", step(&out), step(&input));
    }

    #[test]
    fn settings_parse_with_defaults_and_clamp() {
        let s: EnhanceSettings = serde_json::from_str(r#"{"preset":"warm"}"#).unwrap();
        assert_eq!(s, EnhanceSettings { preset: Preset::Warm, ..EnhanceSettings::default() });
        assert_eq!(EnhanceSettings { amount: 400.0, ..s }.sanitized().amount, 100.0);
    }
}
