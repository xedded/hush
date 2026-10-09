//! Old radio sound: a narrow telephone-like band and gentle tube-style
//! saturation.

use std::f32::consts::{FRAC_1_SQRT_2, TAU};

const LOW_CUT_HZ: f32 = 350.0;
const HIGH_CUT_HZ: f32 = 3_200.0;
const DRIVE: f32 = 3.0;
/// Makes up for the level the band filter takes away; quiet speech ends up about 4 dB louder.
const MAKEUP: f32 = 1.6;

/// RBJ cookbook biquad, transposed direct form II.
#[derive(Clone, Copy)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    z: [f32; 2],
}

impl Biquad {
    fn new(rate: f32, hz: f32, high_pass: bool) -> Self {
        let w = TAU * hz / rate;
        let alpha = w.sin() / (2.0 * FRAC_1_SQRT_2);
        let cos = w.cos();
        let a0 = 1.0 + alpha;
        let (b0, b1) = if high_pass { ((1.0 + cos) / 2.0, -(1.0 + cos)) } else { ((1.0 - cos) / 2.0, 1.0 - cos) };
        Self { b: [b0 / a0, b1 / a0, b0 / a0], a: [-2.0 * cos / a0, (1.0 - alpha) / a0], z: [0.0; 2] }
    }

    fn tick(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        // After the voice gate closes the state decays towards denormals, which are slow on x86.
        for z in &mut self.z {
            if z.abs() < 1e-20 {
                *z = 0.0;
            }
        }
        y
    }
}

pub struct Radio {
    filters: [Biquad; 4],
}

impl Radio {
    pub fn new(rate: u32) -> Self {
        let rate = rate as f32;
        let hp = Biquad::new(rate, LOW_CUT_HZ, true);
        let lp = Biquad::new(rate, HIGH_CUT_HZ, false);
        // Two of each for a steeper, more obviously "small speaker" band.
        Self { filters: [hp, hp, lp, lp] }
    }

    pub fn reset(&mut self) {
        self.filters.iter_mut().for_each(|f| f.z = [0.0; 2]);
    }

    pub fn tick(&mut self, x: f32) -> f32 {
        let banded = self.filters.iter_mut().fold(x, |v, f| f.tick(v));
        (banded * DRIVE).tanh() / DRIVE * MAKEUP
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::fx::shifter::tests::{power_at, sine, RATE};

    fn through(input: &[f32]) -> Vec<f32> {
        let mut r = Radio::new(RATE as u32);
        input.iter().map(|&v| r.tick(v)).collect()
    }

    #[test]
    fn keeps_the_speech_band_and_cuts_the_rest() {
        let gain_db = |hz: f32| {
            let input = sine(hz, 0.5, 0.05);
            let out = through(&input);
            10.0 * (power_at(&out[4_800..], hz) / power_at(&input[4_800..], hz)).log10()
        };
        let (low, mid, high) = (gain_db(80.0), gain_db(1_000.0), gain_db(9_000.0));
        println!("80 Hz {low:.1} dB, 1 kHz {mid:.1} dB, 9 kHz {high:.1} dB");
        assert!(mid > -3.0, "speech band lost: {mid:.1} dB");
        assert!(low < mid - 20.0 && high < mid - 20.0);
    }

    #[test]
    fn loud_input_saturates_instead_of_clipping() {
        let out = through(&sine(1_000.0, 0.2, 1.0));
        assert!(out.iter().all(|v| v.abs() <= MAKEUP / DRIVE + 1e-6));
    }
}
