//! Kaldi-compatible log mel filterbank features, the input the speaker model
//! was trained on (WeSpeaker recipe: 16 kHz, 25 ms frames, 10 ms shift,
//! 80 mel bins, Povey window, pre-emphasis 0.97, samples scaled to int16 range).

use std::sync::Arc;

use realfft::{RealFftPlanner, RealToComplex};

pub const SAMPLE_RATE: usize = 16_000;
pub const NUM_MELS: usize = 80;
pub const FRAME_LEN: usize = 400;
pub const FRAME_SHIFT: usize = 160;
const FFT_LEN: usize = 512;
const PREEMPH: f32 = 0.97;
const LOW_FREQ: f32 = 20.0;
/// WeSpeaker feeds int16-scaled samples to Kaldi's fbank.
const INT16_SCALE: f32 = 32_768.0;

pub type Frame = [f32; NUM_MELS];

pub struct Fbank {
    window: Vec<f32>,
    /// Per mel bin: first FFT bin index and the triangle weights from there.
    filters: Vec<(usize, Vec<f32>)>,
    fft: Arc<dyn RealToComplex<f32>>,
}

fn mel(hz: f32) -> f32 {
    1127.0 * (1.0 + hz / 700.0).ln()
}

/// Number of frames Kaldi produces for `n` samples with snip_edges = true.
pub fn frame_count(n: usize) -> usize {
    if n < FRAME_LEN {
        0
    } else {
        1 + (n - FRAME_LEN) / FRAME_SHIFT
    }
}

/// Samples needed for `frames` frames.
pub const fn samples_for(frames: usize) -> usize {
    if frames == 0 {
        0
    } else {
        FRAME_LEN + (frames - 1) * FRAME_SHIFT
    }
}

impl Default for Fbank {
    fn default() -> Self {
        Self::new()
    }
}

impl Fbank {
    pub fn new() -> Self {
        let window = (0..FRAME_LEN)
            .map(|i| {
                let hann = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (FRAME_LEN - 1) as f32).cos();
                hann.powf(0.85)
            })
            .collect();

        let bins = FFT_LEN / 2;
        let (mel_low, mel_high) = (mel(LOW_FREQ), mel(SAMPLE_RATE as f32 / 2.0));
        let delta = (mel_high - mel_low) / (NUM_MELS + 1) as f32;
        let filters = (0..NUM_MELS)
            .map(|m| {
                let left = mel_low + m as f32 * delta;
                let center = left + delta;
                let right = center + delta;
                let weights: Vec<(usize, f32)> = (0..bins)
                    .filter_map(|i| {
                        let f = mel(i as f32 * SAMPLE_RATE as f32 / FFT_LEN as f32);
                        if f <= left || f >= right {
                            None
                        } else if f <= center {
                            Some((i, (f - left) / (center - left)))
                        } else {
                            Some((i, (right - f) / (right - center)))
                        }
                    })
                    .collect();
                let start = weights.first().map_or(0, |w| w.0);
                (start, weights.into_iter().map(|w| w.1).collect())
            })
            .collect();

        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(FFT_LEN);
        Self { window, filters, fft }
    }

    /// Compute log mel frames for 16 kHz mono samples in [-1, 1].
    pub fn compute(&self, samples: &[f32]) -> Vec<Frame> {
        let mut buf = self.fft.make_input_vec();
        let mut spec = self.fft.make_output_vec();
        let mut power = vec![0.0f32; FFT_LEN / 2];
        (0..frame_count(samples.len()))
            .map(|f| {
                let frame = &samples[f * FRAME_SHIFT..f * FRAME_SHIFT + FRAME_LEN];
                let mean = frame.iter().sum::<f32>() / FRAME_LEN as f32;
                for (b, &s) in buf.iter_mut().zip(frame) {
                    *b = (s - mean) * INT16_SCALE;
                }
                for i in (1..FRAME_LEN).rev() {
                    buf[i] -= PREEMPH * buf[i - 1];
                }
                buf[0] -= PREEMPH * buf[0];
                for (b, w) in buf.iter_mut().zip(&self.window) {
                    *b *= w;
                }
                buf[FRAME_LEN..].fill(0.0);
                self.fft.process(&mut buf, &mut spec).expect("fft sizes match");
                for (p, c) in power.iter_mut().zip(&spec) {
                    *p = c.norm_sqr();
                }
                let mut out = [0.0f32; NUM_MELS];
                for (o, (start, weights)) in out.iter_mut().zip(&self.filters) {
                    let energy: f32 = weights.iter().zip(&power[*start..]).map(|(w, p)| w * p).sum();
                    *o = energy.max(f32::EPSILON).ln();
                }
                out
            })
            .collect()
    }
}

/// Subtract the per-bin mean over time (cepstral mean normalisation), as WeSpeaker does.
pub fn mean_normalize(frames: &mut [Frame]) {
    if frames.is_empty() {
        return;
    }
    let mut mean = [0.0f32; NUM_MELS];
    for f in frames.iter() {
        for (m, v) in mean.iter_mut().zip(f) {
            *m += v;
        }
    }
    let n = frames.len() as f32;
    for f in frames.iter_mut() {
        for (v, m) in f.iter_mut().zip(&mean) {
            *v -= m / n;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_count_matches_kaldi_snip_edges() {
        assert_eq!(frame_count(399), 0);
        assert_eq!(frame_count(400), 1);
        assert_eq!(frame_count(16_000), 98);
        assert_eq!(samples_for(98), 400 + 97 * 160);
        assert_eq!(frame_count(samples_for(150)), 150);
    }

    #[test]
    fn tone_energy_lands_in_the_matching_mel_bin() {
        let fb = Fbank::new();
        let tone: Vec<f32> = (0..16_000).map(|i| 0.3 * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 16_000.0).sin()).collect();
        let frames = fb.compute(&tone);
        let mid = &frames[frames.len() / 2];
        let loudest = (0..NUM_MELS).max_by(|&a, &b| mid[a].total_cmp(&mid[b])).unwrap();
        // 1 kHz sits roughly a third of the way up the mel scale between 20 Hz and 8 kHz.
        let expected = ((mel(1000.0) - mel(LOW_FREQ)) / ((mel(8000.0) - mel(LOW_FREQ)) / 81.0)) as usize;
        assert!((loudest as i64 - expected as i64).abs() <= 1, "loudest bin {loudest}, expected about {expected}");
    }

    #[test]
    fn silence_gives_floor_values_without_nan() {
        let frames = Fbank::new().compute(&vec![0.0; 4000]);
        assert!(frames.iter().flatten().all(|v| v.is_finite()));
    }

    #[test]
    fn mean_normalize_centres_each_bin() {
        let mut frames = vec![[1.0f32; NUM_MELS], [3.0f32; NUM_MELS]];
        mean_normalize(&mut frames);
        assert_eq!(frames[0][0], -1.0);
        assert_eq!(frames[1][79], 1.0);
    }
}
