//! Level processing for the voice: de-esser, compressor, automatic level and limiter.

use crate::audio::biquad::{Biquad, BUTTERWORTH_Q};

fn coef(ms: f32, rate: f32) -> f32 {
    (-1.0 / (ms / 1000.0 * rate)).exp()
}

fn db(lin: f32) -> f32 {
    20.0 * lin.max(1e-9).log10()
}

fn lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Turns down sharp s and sh sounds. The band above `SPLIT_HZ` is reduced
/// only while it dominates the signal, so normal brightness stays.
pub struct DeEsser {
    split: Biquad,
    band_env: f32,
    full_env: f32,
    gain: f32,
    attack: f32,
    release: f32,
    /// Strongest reduction, as a gain (1.0 = off).
    floor: f32,
}

const SPLIT_HZ: f32 = 5_000.0;
/// The band counts as sibilance once it holds this share of the level.
const SIBILANCE_SHARE: f32 = 0.45;

impl DeEsser {
    pub fn new(rate: f32) -> Self {
        Self {
            split: Biquad::low_pass(rate, SPLIT_HZ, BUTTERWORTH_Q),
            band_env: 0.0,
            full_env: 0.0,
            gain: 1.0,
            attack: coef(1.0, rate),
            release: coef(60.0, rate),
            floor: 1.0,
        }
    }

    pub fn set_depth_db(&mut self, depth: f32) {
        self.floor = lin(-depth.max(0.0));
    }

    pub fn reset(&mut self) {
        self.split.reset();
        (self.band_env, self.full_env, self.gain) = (0.0, 0.0, 1.0);
    }

    pub fn tick(&mut self, x: f32) -> f32 {
        // The high band is defined as what the low pass leaves out, so taking
        // part of it away is exact: out = low + gain * high.
        let band = x - self.split.tick(x);
        let follow = |env: f32, v: f32, a: f32, r: f32| {
            let c = if v > env { a } else { r };
            v + c * (env - v)
        };
        self.band_env = follow(self.band_env, band.abs(), self.attack, self.release);
        self.full_env = follow(self.full_env, x.abs(), self.attack, self.release);
        let share = self.band_env / self.full_env.max(1e-6);
        let target = if share > SIBILANCE_SHARE { (SIBILANCE_SHARE / share).max(self.floor) } else { 1.0 };
        let c = if target < self.gain { self.attack } else { self.release };
        self.gain = target + c * (self.gain - target);
        // Remove part of the high band: a dynamic high shelf.
        x - band * (1.0 - self.gain)
    }
}

/// Evens out loud and quiet speech. Feed-forward, soft knee, no make-up gain:
/// the automatic level after it brings the result back up.
pub struct Compressor {
    env_db: f32,
    attack: f32,
    release: f32,
    threshold_db: f32,
    ratio: f32,
}

const KNEE_DB: f32 = 6.0;

impl Compressor {
    pub fn new(rate: f32) -> Self {
        Self { env_db: -120.0, attack: coef(5.0, rate), release: coef(150.0, rate), threshold_db: 0.0, ratio: 1.0 }
    }

    pub fn configure(&mut self, threshold_db: f32, ratio: f32) {
        self.threshold_db = threshold_db;
        self.ratio = ratio.max(1.0);
    }

    pub fn reset(&mut self) {
        self.env_db = -120.0;
    }

    /// Gain reduction in dB for a level, with a soft knee around the threshold.
    fn reduction_db(&self, level_db: f32) -> f32 {
        let over = level_db - self.threshold_db;
        let slope = 1.0 - 1.0 / self.ratio;
        if over <= -KNEE_DB / 2.0 {
            0.0
        } else if over >= KNEE_DB / 2.0 {
            over * slope
        } else {
            slope * (over + KNEE_DB / 2.0).powi(2) / (2.0 * KNEE_DB)
        }
    }

    pub fn tick(&mut self, x: f32) -> f32 {
        if self.ratio <= 1.0 {
            return x;
        }
        let level = db(x.abs());
        let c = if level > self.env_db { self.attack } else { self.release };
        self.env_db = level + c * (self.env_db - level);
        x * lin(-self.reduction_db(self.env_db))
    }
}

/// Brings speech to a steady level, slowly, and only while someone speaks,
/// so pauses are not pumped up into audible noise.
pub struct AutoLevel {
    rms: f32,
    gain_db: f32,
    level_coef: f32,
    gain_step_db: f32,
}

/// Where speech should sit: comfortable in a meeting, with room for peaks.
pub const TARGET_DB: f32 = -20.0;
const MAX_BOOST_DB: f32 = 12.0;
const MAX_CUT_DB: f32 = -9.0;
/// Below this the block is treated as a pause even if the gate is open.
const SPEECH_FLOOR_DB: f32 = -55.0;
/// Fastest change of the gain, in dB per second.
const GAIN_SPEED_DB_S: f32 = 6.0;

impl AutoLevel {
    pub fn new(rate: f32) -> Self {
        Self { rms: 0.0, gain_db: 0.0, level_coef: coef(400.0, rate), gain_step_db: GAIN_SPEED_DB_S / rate }
    }

    pub fn reset(&mut self) {
        self.rms = 0.0;
        self.gain_db = 0.0;
    }

    #[cfg(test)]
    pub fn gain_db(&self) -> f32 {
        self.gain_db
    }

    pub fn process(&mut self, block: &mut [f32], speech: bool) {
        let mut target = self.gain_db;
        if speech {
            for &v in block.iter() {
                self.rms = v * v + self.level_coef * (self.rms - v * v);
            }
            let level = 10.0 * self.rms.max(1e-12).log10();
            if level > SPEECH_FLOOR_DB {
                // The level is measured before this stage, so the needed gain is the gap to the target.
                target = (TARGET_DB - level).clamp(MAX_CUT_DB, MAX_BOOST_DB);
            }
        }
        let start = self.gain_db;
        let step = (target - start).clamp(-self.gain_step_db * block.len() as f32, self.gain_step_db * block.len() as f32);
        self.gain_db = start + step;
        let n = block.len().max(1) as f32;
        for (i, v) in block.iter_mut().enumerate() {
            *v *= lin(start + step * (i + 1) as f32 / n);
        }
    }
}

/// Last stage: never lets a peak past the ceiling.
pub struct Limiter {
    env: f32,
    release: f32,
}

pub const CEILING: f32 = 0.89; // -1 dBFS

impl Limiter {
    pub fn new(rate: f32) -> Self {
        Self { env: 0.0, release: coef(80.0, rate) }
    }

    pub fn reset(&mut self) {
        self.env = 0.0;
    }

    pub fn tick(&mut self, x: f32) -> f32 {
        let a = x.abs();
        self.env = if a > self.env { a } else { a + self.release * (self.env - a) };
        if self.env > CEILING { x * CEILING / self.env } else { x }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::fx::shifter::tests::{power_at, sine, RATE};

    fn level_db(x: &[f32]) -> f32 {
        10.0 * (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).log10()
    }

    #[test]
    fn de_esser_tames_sibilance_and_leaves_the_voice() {
        let run = |hz: f32| {
            let mut d = DeEsser::new(RATE);
            d.set_depth_db(8.0);
            let input = sine(hz, 0.5, 0.3);
            let out: Vec<f32> = input.iter().map(|&v| d.tick(v)).collect();
            10.0 * (power_at(&out[12_000..], hz) / power_at(&input[12_000..], hz)).log10()
        };
        let (voice, hiss) = (run(500.0), run(7_500.0));
        println!("500 Hz {voice:.1} dB, 7.5 kHz {hiss:.1} dB");
        assert!(voice.abs() < 0.5, "voice changed {voice:.1} dB");
        assert!(hiss < -4.0, "sibilance only {hiss:.1} dB");
    }

    #[test]
    fn compressor_follows_its_ratio_above_the_threshold() {
        let mut c = Compressor::new(RATE);
        c.configure(-30.0, 3.0);
        let mut out_at = |amp: f32| {
            c.reset();
            let out: Vec<f32> = sine(1_000.0, 0.5, amp).iter().map(|&v| c.tick(v)).collect();
            level_db(&out[12_000..])
        };
        // 20 dB more in, well above the threshold, gives about 20 / 3 dB more out.
        let rise = out_at(0.5) - out_at(0.05);
        assert!((rise - 20.0 / 3.0).abs() < 1.5, "output rose {rise:.1} dB");
    }

    #[test]
    fn auto_level_lifts_quiet_and_lowers_loud_speech() {
        for (amp, expect_up) in [(0.01f32, true), (0.5, false)] {
            let mut a = AutoLevel::new(RATE);
            let mut x = sine(300.0, 4.0, amp);
            x.chunks_mut(480).for_each(|b| a.process(b, true));
            let (before, after) = (20.0 * (amp / 2f32.sqrt()).log10(), level_db(&x[x.len() - 48_000..]));
            println!("{before:.1} dB -> {after:.1} dB");
            assert_eq!(after > before, expect_up);
            assert!((after - TARGET_DB).abs() < (before - TARGET_DB).abs());
        }
    }

    #[test]
    fn auto_level_holds_still_in_pauses() {
        let mut a = AutoLevel::new(RATE);
        let mut x = sine(300.0, 1.0, 0.001);
        x.chunks_mut(480).for_each(|b| a.process(b, false));
        assert_eq!(a.gain_db(), 0.0);
    }

    #[test]
    fn limiter_holds_the_ceiling() {
        let mut l = Limiter::new(RATE);
        let out: Vec<f32> = sine(200.0, 0.3, 1.5).iter().map(|&v| l.tick(v)).collect();
        assert!(out.iter().all(|v| v.abs() <= CEILING + 1e-6));
        let mut l = Limiter::new(RATE);
        let quiet = sine(200.0, 0.1, 0.3);
        assert!(quiet.iter().all(|&v| l.tick(v) == v));
    }
}
