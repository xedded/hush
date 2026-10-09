//! Voice filters: change how you sound after the noise has been removed.

pub mod radio;
pub mod shifter;

use radio::Radio;
use shifter::{Shift, Shifter};

pub const PITCH_RANGE: f32 = 12.0;
pub const FORMANT_RANGE: f32 = 50.0;
/// Fade between your own voice and the filtered one when the filter is switched.
const FADE_MS: f32 = 20.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    #[default]
    Natural,
    Robot,
    Radio,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FxSettings {
    pub enabled: bool,
    /// Semitones, -12..=12.
    pub pitch: f32,
    /// Formant shift in percent, -50..=50; 100 % would be an octave.
    pub formant: f32,
    pub style: Style,
}

impl Default for FxSettings {
    fn default() -> Self {
        Self { enabled: false, pitch: 0.0, formant: 0.0, style: Style::Natural }
    }
}

fn clamp(v: f32, range: f32) -> f32 {
    if v.is_finite() { v.clamp(-range, range) } else { 0.0 }
}

impl FxSettings {
    /// The same settings with every value inside its allowed range.
    pub fn sanitized(self) -> Self {
        Self { pitch: clamp(self.pitch, PITCH_RANGE), formant: clamp(self.formant, FORMANT_RANGE), ..self }
    }

    fn shift(&self) -> Shift {
        Shift {
            pitch: 2f32.powf(self.pitch / 12.0),
            formant: 2f32.powf(self.formant / 100.0),
            robot: self.style == Style::Robot,
        }
    }
}

/// Extra delay while a filter is on, in milliseconds at `rate`.
pub fn latency_ms(rate: u32) -> f32 {
    shifter::LATENCY as f32 * 1000.0 / rate as f32
}

pub struct VoiceFx {
    shifter: Shifter,
    radio: Radio,
    /// 0 = only your own voice, 1 = only the filtered one.
    mix: f32,
    /// Mix change per sample: a linear fade that ends exactly at 0 or 1.
    step: f32,
    idle: bool,
    wet: Vec<f32>,
}

impl VoiceFx {
    pub fn new(rate: u32) -> Self {
        Self {
            shifter: Shifter::new(),
            radio: Radio::new(rate),
            mix: 0.0,
            step: 1.0 / (FADE_MS / 1000.0 * rate as f32),
            idle: true,
            wet: Vec::with_capacity(4_096),
        }
    }

    /// True while any filtered sound reaches the output.
    pub fn active(&self) -> bool {
        !self.idle
    }

    fn go_idle(&mut self) {
        self.shifter.reset();
        self.radio.reset();
        self.mix = 0.0;
        self.idle = true;
    }

    /// Filter a block in place. When switched off the block is left untouched.
    pub fn process(&mut self, block: &mut [f32], s: &FxSettings) {
        if !s.enabled && self.mix == 0.0 {
            if !self.idle {
                self.go_idle();
            }
            return;
        }
        self.idle = false;
        self.wet.clear();
        self.wet.extend_from_slice(block);
        self.shifter.process(&mut self.wet, s.shift());
        if s.style == Style::Radio {
            self.wet.iter_mut().for_each(|v| *v = self.radio.tick(*v));
        }
        // A broken sample would otherwise stick in the phase memory for good.
        if self.wet.iter().any(|v| !v.is_finite()) {
            log::warn!("voice filter produced invalid audio; restarting it");
            self.go_idle();
            block.iter_mut().filter(|v| !v.is_finite()).for_each(|v| *v = 0.0);
            return;
        }
        let target = if s.enabled { 1.0 } else { 0.0 };
        for (dry, wet) in block.iter_mut().zip(&self.wet) {
            self.mix = if target > self.mix { (self.mix + self.step).min(1.0) } else { (self.mix - self.step).max(0.0) };
            *dry = (*dry * (1.0 - self.mix) + wet * self.mix).clamp(-1.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::shifter::tests::{buzz, power_at};
    use super::*;

    const RATE: u32 = 48_000;

    fn run(fx: &mut VoiceFx, input: &[f32], s: &FxSettings) -> Vec<f32> {
        let mut out = input.to_vec();
        out.chunks_mut(480).for_each(|b| fx.process(b, s));
        out
    }

    #[test]
    fn switched_off_is_bit_exact_and_adds_no_delay() {
        let input = buzz(140.0, 0.3);
        let mut fx = VoiceFx::new(RATE);
        let s = FxSettings { pitch: 7.0, ..FxSettings::default() };
        assert_eq!(run(&mut fx, &input, &s), input);
        assert!(!fx.active());
    }

    #[test]
    fn switching_on_and_off_fades_without_jumps() {
        let input = buzz(140.0, 1.0);
        let mut fx = VoiceFx::new(RATE);
        let on = FxSettings { enabled: true, pitch: 5.0, ..FxSettings::default() };
        let mut out = run(&mut fx, &input[..24_000], &on);
        assert!(fx.active());
        out.extend(run(&mut fx, &input[24_000..], &FxSettings::default()));
        let biggest_step = out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        let input_step = input.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        assert!(biggest_step < 2.0 * input_step, "jump {biggest_step} vs {input_step}");
        assert!(!fx.active(), "filter should go idle once faded out");
    }

    #[test]
    fn recovers_from_an_invalid_sample() {
        let mut fx = VoiceFx::new(RATE);
        let s = FxSettings { enabled: true, pitch: 3.0, ..FxSettings::default() };
        let mut input = buzz(140.0, 0.5);
        input[2_000] = f32::NAN;
        let out = run(&mut fx, &input, &s);
        let after = run(&mut fx, &buzz(140.0, 0.5), &s);
        assert!(out.iter().chain(&after).all(|v| v.is_finite()));
        assert!(after[12_000..].iter().any(|v| v.abs() > 0.01), "filter stayed silent");
    }

    #[test]
    fn pitch_setting_moves_the_fundamental() {
        let mut fx = VoiceFx::new(RATE);
        let s = FxSettings { enabled: true, pitch: 12.0, ..FxSettings::default() };
        let out = run(&mut fx, &buzz(150.0, 1.0), &s);
        let tail = &out[12_000..];
        assert!(power_at(tail, 300.0) > 10.0 * power_at(tail, 150.0));
    }

    #[test]
    fn radio_style_narrows_the_band() {
        let input = buzz(120.0, 1.0);
        let mut fx = VoiceFx::new(RATE);
        let s = FxSettings { enabled: true, style: Style::Radio, ..FxSettings::default() };
        let out = run(&mut fx, &input, &s);
        let (i, o) = (&input[12_000..], &out[12_000..]);
        assert!(power_at(o, 120.0) / power_at(o, 1_200.0) < 0.1 * power_at(i, 120.0) / power_at(i, 1_200.0));
    }

    #[test]
    fn settings_are_clamped_and_parse_with_defaults() {
        let s = FxSettings { pitch: 40.0, formant: f32::NAN, ..FxSettings::default() }.sanitized();
        assert_eq!((s.pitch, s.formant), (PITCH_RANGE, 0.0));
        let parsed: FxSettings = serde_json::from_str(r#"{"style":"robot"}"#).unwrap();
        assert_eq!(parsed, FxSettings { style: Style::Robot, ..FxSettings::default() });
    }
}
