//! Voice gate: closes the microphone when the (already denoised) signal falls
//! below the "Känslighet för omgivning" threshold. Uses hysteresis and a hold
//! time so word endings are not chopped, and smooths gain changes to avoid clicks.

const HYSTERESIS_DB: f32 = 6.0;
const HOLD_MS: f32 = 180.0;
const OPEN_MS: f32 = 2.0;
const CLOSE_MS: f32 = 120.0;
const DETECTOR_RELEASE_MS: f32 = 40.0;
/// Gain applied while closed. Not total silence, which sounds like a dropped call.
const FLOOR_DB: f32 = -50.0;

pub struct Gate {
    envelope: f32,
    gain: f32,
    open: bool,
    hold_left: usize,
    hold_samples: usize,
    detector_release: f32,
    open_coef: f32,
    close_coef: f32,
    floor: f32,
}

fn coef(ms: f32, sample_rate: f32) -> f32 {
    (-1.0 / (ms / 1000.0 * sample_rate)).exp()
}

pub fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

impl Gate {
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate as f32;
        Self {
            envelope: 0.0,
            gain: db_to_lin(FLOOR_DB),
            open: false,
            hold_left: 0,
            hold_samples: (HOLD_MS / 1000.0 * sr) as usize,
            detector_release: coef(DETECTOR_RELEASE_MS, sr),
            open_coef: coef(OPEN_MS, sr),
            close_coef: coef(CLOSE_MS, sr),
            floor: db_to_lin(FLOOR_DB),
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Process a block in place with the given threshold in dBFS.
    pub fn process(&mut self, block: &mut [f32], threshold_dbfs: f32) {
        let open_at = db_to_lin(threshold_dbfs);
        let close_at = db_to_lin(threshold_dbfs - HYSTERESIS_DB);
        for s in block.iter_mut() {
            let level = s.abs();
            self.envelope = if level > self.envelope {
                level
            } else {
                level + self.detector_release * (self.envelope - level)
            };

            if self.envelope >= open_at {
                self.open = true;
                self.hold_left = self.hold_samples;
            } else if self.envelope < close_at {
                if self.hold_left > 0 {
                    self.hold_left -= 1;
                } else {
                    self.open = false;
                }
            }

            let (target, c) = if self.open { (1.0, self.open_coef) } else { (self.floor, self.close_coef) };
            self.gain = target + c * (self.gain - target);
            *s *= self.gain;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    fn tone(amp: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| amp * (i as f32 * 0.05).sin()).collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn quiet_signal_is_attenuated() {
        let mut g = Gate::new(SR);
        let mut x = tone(db_to_lin(-55.0), SR as usize);
        let before = rms(&x);
        g.process(&mut x, -40.0);
        assert!(!g.is_open());
        let tail = &x[x.len() / 2..];
        assert!(rms(tail) < before * db_to_lin(-40.0));
    }

    #[test]
    fn loud_signal_passes_nearly_unchanged() {
        let mut g = Gate::new(SR);
        let mut x = tone(db_to_lin(-20.0), SR as usize);
        let reference = x.clone();
        g.process(&mut x, -40.0);
        assert!(g.is_open());
        let tail = x.len() / 2..;
        assert!((rms(&x[tail.clone()]) - rms(&reference[tail])).abs() < 1e-3);
    }

    #[test]
    fn gate_holds_briefly_after_speech_stops() {
        let mut g = Gate::new(SR);
        let mut speech = tone(db_to_lin(-20.0), SR as usize / 4);
        g.process(&mut speech, -40.0);
        // 50 ms of silence is shorter than the hold time: still open.
        let mut gap = vec![0.0; SR as usize / 20];
        g.process(&mut gap, -40.0);
        assert!(g.is_open());
        // Half a second of silence: closed.
        let mut long_gap = vec![0.0; SR as usize / 2];
        g.process(&mut long_gap, -40.0);
        assert!(!g.is_open());
    }

    #[test]
    fn hysteresis_keeps_gate_open_just_below_threshold() {
        let mut g = Gate::new(SR);
        let mut loud = tone(db_to_lin(-30.0), SR as usize / 4);
        g.process(&mut loud, -40.0);
        // 3 dB under the threshold is still inside the hysteresis band.
        let mut slightly_quieter = tone(db_to_lin(-43.0), SR as usize);
        g.process(&mut slightly_quieter, -40.0);
        assert!(g.is_open());
    }
}
