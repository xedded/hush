//! Old radio sound: a narrow telephone-like band and gentle tube-style
//! saturation.

use crate::audio::biquad::{Biquad, BUTTERWORTH_Q};

const LOW_CUT_HZ: f32 = 350.0;
const HIGH_CUT_HZ: f32 = 3_200.0;
const DRIVE: f32 = 3.0;
/// Makes up for the level the band filter takes away; quiet speech ends up about 4 dB louder.
const MAKEUP: f32 = 1.6;

pub struct Radio {
    filters: [Biquad; 4],
}

impl Radio {
    pub fn new(rate: u32) -> Self {
        let rate = rate as f32;
        let hp = Biquad::high_pass(rate, LOW_CUT_HZ, BUTTERWORTH_Q);
        let lp = Biquad::low_pass(rate, HIGH_CUT_HZ, BUTTERWORTH_Q);
        // Two of each for a steeper, more obviously "small speaker" band.
        Self { filters: [hp, hp, lp, lp] }
    }

    pub fn reset(&mut self) {
        self.filters.iter_mut().for_each(Biquad::reset);
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
