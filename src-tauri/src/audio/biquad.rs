//! Second-order IIR filters (RBJ audio EQ cookbook), transposed direct form II.

use std::f32::consts::{FRAC_1_SQRT_2, TAU};

/// Q of a Butterworth section: the flattest pass band.
pub const BUTTERWORTH_Q: f32 = FRAC_1_SQRT_2;
/// Below this the filter state is flushed to zero; denormals are slow on x86.
const DENORMAL: f32 = 1e-20;

#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    z: [f32; 2],
}

struct Design {
    cos: f32,
    alpha: f32,
}

fn design(rate: f32, hz: f32, q: f32) -> Design {
    let w = TAU * hz.clamp(1.0, rate * 0.49) / rate;
    Design { cos: w.cos(), alpha: w.sin() / (2.0 * q) }
}

impl Biquad {
    fn normalized(b: [f32; 3], a: [f32; 3]) -> Self {
        Self { b: [b[0] / a[0], b[1] / a[0], b[2] / a[0]], a: [a[1] / a[0], a[2] / a[0]], z: [0.0; 2] }
    }

    /// Passes everything unchanged.
    pub fn identity() -> Self {
        Self { b: [1.0, 0.0, 0.0], a: [0.0, 0.0], z: [0.0; 2] }
    }

    pub fn high_pass(rate: f32, hz: f32, q: f32) -> Self {
        let Design { cos, alpha } = design(rate, hz, q);
        let b0 = (1.0 + cos) / 2.0;
        Self::normalized([b0, -(1.0 + cos), b0], [1.0 + alpha, -2.0 * cos, 1.0 - alpha])
    }

    pub fn low_pass(rate: f32, hz: f32, q: f32) -> Self {
        let Design { cos, alpha } = design(rate, hz, q);
        let b0 = (1.0 - cos) / 2.0;
        Self::normalized([b0, 1.0 - cos, b0], [1.0 + alpha, -2.0 * cos, 1.0 - alpha])
    }

    /// Bell-shaped boost or cut around `hz`.
    pub fn peaking(rate: f32, hz: f32, q: f32, gain_db: f32) -> Self {
        let Design { cos, alpha } = design(rate, hz, q);
        let a = 10f32.powf(gain_db / 40.0);
        Self::normalized(
            [1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a],
            [1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a],
        )
    }

    /// Boost or cut everything below `hz`.
    pub fn low_shelf(rate: f32, hz: f32, gain_db: f32) -> Self {
        Self::shelf(rate, hz, gain_db, false)
    }

    /// Boost or cut everything above `hz`.
    pub fn high_shelf(rate: f32, hz: f32, gain_db: f32) -> Self {
        Self::shelf(rate, hz, gain_db, true)
    }

    fn shelf(rate: f32, hz: f32, gain_db: f32, high: bool) -> Self {
        let Design { cos, alpha } = design(rate, hz, BUTTERWORTH_Q);
        let a = 10f32.powf(gain_db / 40.0);
        let k = 2.0 * a.sqrt() * alpha;
        // The high shelf is the low shelf with the sign of the cosine terms flipped.
        let s = if high { -1.0 } else { 1.0 };
        let (ap, am) = (a + 1.0, a - 1.0);
        Self::normalized(
            [a * (ap - s * am * cos + k), s * 2.0 * a * (am - s * ap * cos), a * (ap - s * am * cos - k)],
            [ap + s * am * cos + k, -s * 2.0 * (am + s * ap * cos), ap + s * am * cos - k],
        )
    }

    pub fn reset(&mut self) {
        self.z = [0.0; 2];
    }

    /// Take over the response of `other` but keep the running state, so a
    /// setting can change while sound flows without a click.
    pub fn retune(&mut self, other: Biquad) {
        self.b = other.b;
        self.a = other.a;
    }

    pub fn tick(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        for z in &mut self.z {
            if z.abs() < DENORMAL {
                *z = 0.0;
            }
        }
        y
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub const RATE: f32 = 48_000.0;

    /// Steady-state gain of a filter at one frequency, in dB.
    pub fn gain_db(mut f: Biquad, hz: f32) -> f32 {
        let n = RATE as usize / 2;
        let out: Vec<f32> = (0..n).map(|i| f.tick((TAU * hz * i as f32 / RATE).sin())).collect();
        let peak = out[n / 2..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        20.0 * peak.log10()
    }

    #[test]
    fn pass_and_stop_bands() {
        let hp = Biquad::high_pass(RATE, 100.0, BUTTERWORTH_Q);
        assert!(gain_db(hp, 25.0) < -20.0 && gain_db(hp, 1_000.0).abs() < 0.2);
        let lp = Biquad::low_pass(RATE, 3_000.0, BUTTERWORTH_Q);
        assert!(gain_db(lp, 12_000.0) < -20.0 && gain_db(lp, 300.0).abs() < 0.2);
    }

    #[test]
    fn peaking_and_shelves_reach_their_gain() {
        let bell = Biquad::peaking(RATE, 4_000.0, 1.0, 6.0);
        assert!((gain_db(bell, 4_000.0) - 6.0).abs() < 0.3 && gain_db(bell, 200.0).abs() < 0.3);
        let low = Biquad::low_shelf(RATE, 200.0, 4.0);
        assert!((gain_db(low, 40.0) - 4.0).abs() < 0.3 && gain_db(low, 5_000.0).abs() < 0.3);
        let high = Biquad::high_shelf(RATE, 6_000.0, -5.0);
        let (top, bottom) = (gain_db(high, 16_000.0), gain_db(high, 300.0));
        // RBJ shelves overshoot slightly before settling at the exact gain at Nyquist.
        assert!((top + 5.0).abs() < 1.0 && bottom.abs() < 0.3, "high shelf {top:.2} / {bottom:.2} dB");
    }
}
