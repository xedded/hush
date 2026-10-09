//! Pitch and formant shifting with a phase vocoder.
//!
//! Each frame's spectrum is split into a smooth envelope (the formants, what
//! makes a voice sound like a particular person) and the fine structure that
//! carries the pitch. The fine structure is moved by the pitch ratio and the
//! envelope by the formant ratio, so the two can be changed independently:
//! a higher pitch with the same formants still sounds like you, only higher.
//! The envelope comes from cepstral smoothing of the log magnitude.

use std::f32::consts::{PI, TAU};
use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

/// 21 ms frames at 48 kHz: long enough to resolve the harmonics of a low voice.
pub const FRAME: usize = 1024;
const OVERLAP: usize = 4;
pub const HOP: usize = FRAME / OVERLAP;
const BINS: usize = FRAME / 2 + 1;
/// Write position a new frame starts filling from.
const START: usize = FRAME - HOP;
/// Delay through the shifter, in samples: a sample is complete once the last
/// frame that contains it has been overlap-added.
pub const LATENCY: usize = FRAME;
/// Cepstral coefficients kept for the envelope. Must stay below the pitch
/// period of the highest voice (400 Hz is 120 samples at 48 kHz).
const LIFTER: usize = 50;
/// Hann squared, overlapped four times, sums to 1.5.
const OLA_GAIN: f32 = 1.5;
const LOG_FLOOR: f32 = 1e-9;

pub struct Shifter {
    forward: Arc<dyn RealToComplex<f32>>,
    inverse: Arc<dyn ComplexToReal<f32>>,
    window: Vec<f32>,
    input: Vec<f32>,
    output: Vec<f32>,
    accum: Vec<f32>,
    rover: usize,
    last_phase: Vec<f32>,
    sum_phase: Vec<f32>,
    // Scratch, reused every frame.
    time: Vec<f32>,
    spec: Vec<Complex<f32>>,
    mag: Vec<f32>,
    freq: Vec<f32>,
    env: Vec<f32>,
    syn_mag: Vec<f32>,
    syn_freq: Vec<f32>,
}

/// What a frame should turn into.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shift {
    /// Frequency ratio for the pitch, 2.0 is one octave up.
    pub pitch: f32,
    /// Frequency ratio for the formants, above 1.0 sounds smaller and brighter.
    pub formant: f32,
    /// Replace every phase with a fixed one: a flat, buzzing machine voice.
    pub robot: bool,
}

fn wrap(phase: f32) -> f32 {
    phase - TAU * ((phase + PI) / TAU).floor()
}

impl Shifter {
    pub fn new() -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let forward = planner.plan_fft_forward(FRAME);
        let inverse = planner.plan_fft_inverse(FRAME);
        let window = (0..FRAME).map(|n| 0.5 - 0.5 * (TAU * n as f32 / FRAME as f32).cos()).collect();
        Self {
            spec: forward.make_output_vec(),
            forward,
            inverse,
            window,
            input: vec![0.0; FRAME],
            output: vec![0.0; FRAME],
            accum: vec![0.0; FRAME],
            rover: START,
            last_phase: vec![0.0; BINS],
            sum_phase: vec![0.0; BINS],
            time: vec![0.0; FRAME],
            mag: vec![0.0; BINS],
            freq: vec![0.0; BINS],
            env: vec![0.0; BINS],
            syn_mag: vec![0.0; BINS],
            syn_freq: vec![0.0; BINS],
        }
    }

    /// Forget all history, as if freshly created.
    pub fn reset(&mut self) {
        for buf in [&mut self.input, &mut self.output, &mut self.accum] {
            buf.fill(0.0);
        }
        self.last_phase.fill(0.0);
        self.sum_phase.fill(0.0);
        self.rover = START;
    }

    /// Process a block in place. Output lags input by `LATENCY` samples.
    pub fn process(&mut self, block: &mut [f32], shift: Shift) {
        for s in block.iter_mut() {
            self.input[self.rover] = *s;
            *s = self.output[self.rover - START];
            self.rover += 1;
            if self.rover == FRAME {
                self.frame(shift);
                self.rover = START;
            }
        }
    }

    fn frame(&mut self, shift: Shift) {
        self.analyse();
        self.envelope();
        self.move_partials(shift);
        self.synthesise(shift.robot);
        self.input.copy_within(HOP.., 0);
    }

    /// Magnitude and true frequency (in bins) of every bin.
    fn analyse(&mut self) {
        for ((t, x), w) in self.time.iter_mut().zip(&self.input).zip(&self.window) {
            *t = x * w;
        }
        // Lengths are fixed at construction, so the transform cannot fail.
        let _ = self.forward.process(&mut self.time, &mut self.spec);
        let expected = TAU * HOP as f32 / FRAME as f32;
        for k in 0..BINS {
            let c = self.spec[k];
            let phase = c.im.atan2(c.re);
            let delta = wrap(phase - self.last_phase[k] - k as f32 * expected);
            self.last_phase[k] = phase;
            self.mag[k] = c.norm();
            self.freq[k] = k as f32 + delta * OVERLAP as f32 / TAU;
        }
    }

    /// Smooth spectral envelope: low quefrencies of the log magnitude.
    fn envelope(&mut self) {
        for (s, m) in self.spec.iter_mut().zip(&self.mag) {
            *s = Complex::new((m + LOG_FLOOR).ln(), 0.0);
        }
        let _ = self.inverse.process(&mut self.spec, &mut self.time);
        let scale = 1.0 / FRAME as f32;
        for (n, c) in self.time.iter_mut().enumerate() {
            let keep = !(LIFTER..=FRAME - LIFTER).contains(&n);
            *c = if keep { *c * scale } else { 0.0 };
        }
        let _ = self.forward.process(&mut self.time, &mut self.spec);
        for (e, s) in self.env.iter_mut().zip(&self.spec) {
            *e = s.re.exp();
        }
    }

    /// Move the fine structure by the pitch ratio, then lay the envelope,
    /// stretched by the formant ratio, back on top.
    fn move_partials(&mut self, shift: Shift) {
        self.syn_mag.fill(0.0);
        self.syn_freq.fill(0.0);
        for k in 0..BINS {
            let target = (k as f32 * shift.pitch).round() as usize;
            // When shifting down several bins land on one; summing them would
            // boost the level, so the strongest one wins.
            let excitation = self.mag[k] / self.env[k].max(LOG_FLOOR);
            if target < BINS && excitation >= self.syn_mag[target] {
                self.syn_mag[target] = excitation;
                self.syn_freq[target] = self.freq[k] * shift.pitch;
            }
        }
        for (j, m) in self.syn_mag.iter_mut().enumerate() {
            *m *= envelope_at(&self.env, j as f32 / shift.formant);
        }
    }

    fn synthesise(&mut self, robot: bool) {
        let expected = TAU * HOP as f32 / FRAME as f32;
        for k in 0..BINS {
            // Kept running in robot mode too, so switching back does not click.
            let delta = (self.syn_freq[k] - k as f32) * TAU / OVERLAP as f32;
            self.sum_phase[k] = wrap(self.sum_phase[k] + delta + k as f32 * expected);
            let phase = if robot {
                // Zero phase, centred in the frame: one pulse per hop.
                if k % 2 == 0 { 0.0 } else { PI }
            } else {
                self.sum_phase[k]
            };
            self.spec[k] = Complex::from_polar(self.syn_mag[k], phase);
        }
        // The real transform needs purely real DC and Nyquist bins.
        self.spec[0].im = 0.0;
        self.spec[BINS - 1].im = 0.0;
        let _ = self.inverse.process(&mut self.spec, &mut self.time);
        let scale = 1.0 / (FRAME as f32 * OLA_GAIN);
        for ((a, t), w) in self.accum.iter_mut().zip(&self.time).zip(&self.window) {
            *a += t * w * scale;
        }
        self.output[..HOP].copy_from_slice(&self.accum[..HOP]);
        self.accum.copy_within(HOP.., 0);
        self.accum[FRAME - HOP..].fill(0.0);
    }
}

/// Linear interpolation into the envelope; past the top it stays at the last value.
fn envelope_at(env: &[f32], bin: f32) -> f32 {
    let last = env.len() - 1;
    if bin >= last as f32 {
        return env[last];
    }
    let i = bin.max(0.0) as usize;
    let frac = bin - i as f32;
    env[i] * (1.0 - frac) + env[i + 1] * frac
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub const RATE: f32 = 48_000.0;

    impl Shift {
        pub const NONE: Shift = Shift { pitch: 1.0, formant: 1.0, robot: false };
    }

    pub fn sine(hz: f32, seconds: f32, amp: f32) -> Vec<f32> {
        let n = (RATE * seconds) as usize;
        (0..n).map(|i| (TAU * hz * i as f32 / RATE).sin() * amp).collect()
    }

    /// Band-limited sawtooth: a crude stand-in for a voiced sound.
    pub fn buzz(hz: f32, seconds: f32) -> Vec<f32> {
        let n = (RATE * seconds) as usize;
        let harmonics = (RATE / 2.0 / hz) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / RATE;
                (1..=harmonics).map(|h| (TAU * hz * h as f32 * t).sin() / h as f32).sum::<f32>() * 0.2
            })
            .collect()
    }

    /// Energy at one frequency (Goertzel).
    pub fn power_at(x: &[f32], hz: f32) -> f32 {
        let w = TAU * hz / RATE;
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for &v in x {
            let s = v + 2.0 * w.cos() * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        (s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2) / x.len() as f32
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    fn run(input: &[f32], shift: Shift) -> Vec<f32> {
        let mut s = Shifter::new();
        let mut out = input.to_vec();
        for block in out.chunks_mut(480) {
            s.process(block, shift);
        }
        out
    }

    /// A phase vocoder keeps the sound, not the exact waveform (phases drift
    /// per bin), so compare level and harmonics rather than samples.
    #[test]
    fn no_shift_keeps_level_and_harmonics() {
        let input = buzz(140.0, 1.0);
        let out = run(&input, Shift::NONE);
        let (i, o) = (&input[8_000..], &out[8_000..]);
        let level_db = 20.0 * (rms(o) / rms(i)).log10();
        assert!(level_db.abs() < 1.0, "level changed {level_db:.2} dB");
        for h in 1..=10 {
            let hz = 140.0 * h as f32;
            let diff = 10.0 * (power_at(o, hz) / power_at(i, hz)).log10();
            assert!(diff.abs() < 2.0, "harmonic {h} changed {diff:.2} dB");
        }
    }

    /// Presets range from Troll (-10 st) to Helium (+9 st); none should jump in level.
    #[test]
    fn shifting_keeps_the_level() {
        let input = buzz(140.0, 1.0);
        for semitones in [-10.0f32, -5.0, 5.0, 9.0] {
            let out = run(&input, Shift { pitch: 2f32.powf(semitones / 12.0), ..Shift::NONE });
            let level_db = 20.0 * (rms(&out[8_000..]) / rms(&input[8_000..])).log10();
            println!("{semitones:+} st: {level_db:+.1} dB");
            assert!(level_db.abs() < 4.0, "{semitones} st changed level {level_db:.1} dB");
        }
    }

    #[test]
    fn octave_up_moves_a_tone_to_twice_the_frequency() {
        let out = run(&sine(440.0, 1.0, 0.3), Shift { pitch: 2.0, ..Shift::NONE });
        let tail = &out[8_000..];
        let (at_880, at_440) = (power_at(tail, 880.0), power_at(tail, 440.0));
        assert!(at_880 > 30.0 * at_440, "880 Hz {at_880:e} vs 440 Hz {at_440:e}");
    }

    #[test]
    fn octave_down_moves_a_tone_to_half_the_frequency() {
        let out = run(&sine(440.0, 1.0, 0.3), Shift { pitch: 0.5, ..Shift::NONE });
        let tail = &out[8_000..];
        assert!(power_at(tail, 220.0) > 30.0 * power_at(tail, 440.0));
    }

    /// Brighter formants lift the upper harmonics while the pitch stays put.
    #[test]
    fn formant_shift_changes_brightness_not_pitch() {
        let input = buzz(150.0, 1.0);
        let base = run(&input, Shift::NONE);
        let bright = run(&input, Shift { formant: 1.4, ..Shift::NONE });
        let (b, x) = (&base[8_000..], &bright[8_000..]);
        let high = |y: &[f32]| (10..20).map(|h| power_at(y, 150.0 * h as f32)).sum::<f32>();
        let low = |y: &[f32]| power_at(y, 150.0);
        assert!(high(x) / low(x) > 1.5 * high(b) / low(b), "formant shift did not brighten");
        assert!(low(x) > 10.0 * power_at(x, 75.0), "fundamental moved");
    }

    #[test]
    fn robot_voice_is_audible_and_bounded() {
        let out = run(&buzz(140.0, 0.5), Shift { robot: true, ..Shift::NONE });
        let level = rms(&out[8_000..]);
        assert!(level > 0.01 && out.iter().all(|v| v.is_finite() && v.abs() < 2.0), "rms {level}");
    }

    #[test]
    fn silence_stays_silent() {
        let out = run(&vec![0.0; 9_600], Shift { pitch: 1.5, formant: 0.8, robot: false });
        assert!(out.iter().all(|v| v.abs() < 1e-6));
    }
}
