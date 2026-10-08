//! Streaming linear resampler for microphones that do not run at 48 kHz
//! (Bluetooth headsets at 16 kHz, some webcams at 44.1 kHz). DeepFilterNet
//! needs 48 kHz. Linear interpolation is adequate here: the denoiser removes
//! most imaging, and these sources are band-limited well below Nyquist anyway.

pub struct Resampler {
    /// Input samples advanced per output sample.
    step: f64,
    /// Position of the next output sample relative to `prev`, in [0, 1).
    pos: f64,
    prev: f32,
    primed: bool,
}

impl Resampler {
    pub fn new(from_hz: u32, to_hz: u32) -> Self {
        Self { step: from_hz as f64 / to_hz as f64, pos: 0.0, prev: 0.0, primed: false }
    }

    pub fn is_passthrough(&self) -> bool {
        self.step == 1.0
    }

    /// Resample `input`, appending to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.is_passthrough() {
            out.extend_from_slice(input);
            return;
        }
        for &cur in input {
            if !self.primed {
                self.prev = cur;
                self.primed = true;
                continue;
            }
            // Emit every output sample that falls between `prev` and `cur`.
            while self.pos < 1.0 {
                let t = self.pos as f32;
                out.push(self.prev + (cur - self.prev) * t);
                self.pos += self.step;
            }
            self.pos -= 1.0;
            self.prev = cur;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_rate_is_passthrough() {
        let mut r = Resampler::new(48_000, 48_000);
        let mut out = Vec::new();
        r.process(&[0.1, 0.2, 0.3], &mut out);
        assert_eq!(out, vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn upsampling_16k_triples_sample_count() {
        let mut r = Resampler::new(16_000, 48_000);
        let input: Vec<f32> = (0..1600).map(|i| (i as f32 * 0.01).sin()).collect();
        let mut out = Vec::new();
        r.process(&input, &mut out);
        assert!((out.len() as i64 - 4800).abs() <= 3, "got {}", out.len());
    }

    #[test]
    fn output_count_stays_correct_across_chunks() {
        let mut r = Resampler::new(44_100, 48_000);
        let mut out = Vec::new();
        for _ in 0..100 {
            r.process(&vec![0.5; 441], &mut out);
        }
        assert!((out.len() as i64 - 48_000).abs() <= 2, "got {}", out.len());
        assert!(out.iter().all(|v| (v - 0.5).abs() < 1e-6));
    }

    #[test]
    fn interpolates_between_samples() {
        let mut r = Resampler::new(24_000, 48_000);
        let mut out = Vec::new();
        r.process(&[0.0, 1.0, 0.0], &mut out);
        assert_eq!(out, vec![0.0, 0.5, 1.0, 0.5]);
    }
}
