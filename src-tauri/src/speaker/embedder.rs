//! Speaker embeddings ("voiceprints") with the WeSpeaker CAM++ model
//! (VoxCeleb, large-margin fine-tuned; CC BY 4.0). One embedding describes
//! about 1.5 s of speech; cosine similarity compares two of them.

use anyhow::{anyhow, Result};
use tract_onnx::prelude::*;

use super::fbank::{self, Fbank, Frame, NUM_MELS};

/// Frames per embedding window: 1.5 s of audio.
pub const WINDOW_FRAMES: usize = 150;
/// Samples (16 kHz) per embedding window.
pub const WINDOW_SAMPLES: usize = fbank::samples_for(WINDOW_FRAMES);

static MODEL: &[u8] = include_bytes!("../../models/campplus_lm.onnx");

type Plan = SimplePlan<TypedFact, Box<dyn TypedOp>, Graph<TypedFact, Box<dyn TypedOp>>>;

pub struct Embedder {
    plan: Plan,
    fbank: Fbank,
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

pub fn normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
}

/// Average several embeddings into one unit-length voiceprint.
pub fn mean(embeddings: &[Vec<f32>]) -> Option<Vec<f32>> {
    let first = embeddings.first()?;
    let mut acc = vec![0.0f32; first.len()];
    for e in embeddings {
        for (a, v) in acc.iter_mut().zip(e) {
            *a += v;
        }
    }
    normalize(&mut acc);
    Some(acc)
}

impl Embedder {
    pub fn new() -> Result<Self> {
        let plan = tract_onnx::onnx()
            .model_for_read(&mut std::io::Cursor::new(MODEL))?
            .with_input_fact(0, f32::fact([1, WINDOW_FRAMES, NUM_MELS]).into())?
            .into_optimized()?
            .into_runnable()
            .map_err(|e| anyhow!("speaker model: {e}"))?;
        Ok(Self { plan, fbank: Fbank::new() })
    }

    /// Embed exactly one window of 16 kHz speech (`WINDOW_SAMPLES` long).
    pub fn embed(&self, samples: &[f32]) -> Result<Vec<f32>> {
        if samples.len() < WINDOW_SAMPLES {
            return Err(anyhow!("need {WINDOW_SAMPLES} samples, got {}", samples.len()));
        }
        let mut frames: Vec<Frame> = self.fbank.compute(&samples[samples.len() - WINDOW_SAMPLES..]);
        frames.truncate(WINDOW_FRAMES);
        fbank::mean_normalize(&mut frames);
        let input = tract_ndarray::Array3::from_shape_fn((1, WINDOW_FRAMES, NUM_MELS), |(_, t, m)| frames[t][m]);
        let out = self.plan.run(tvec!(Tensor::from(input).into()))?;
        let mut v = out[0].as_slice::<f32>()?.to_vec();
        normalize(&mut v);
        Ok(v)
    }

    /// Embed a longer recording as overlapping windows (for enrollment and tests).
    pub fn embed_all(&self, samples: &[f32], hop: usize) -> Result<Vec<Vec<f32>>> {
        let mut out = Vec::new();
        let mut start = 0;
        while start + WINDOW_SAMPLES <= samples.len() {
            out.push(self.embed(&samples[start..start + WINDOW_SAMPLES])?);
            start += hop;
        }
        Ok(out)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Reads the 16 kHz mono 16-bit fixtures in tests/fixtures.
    pub fn read_fixture(name: &str) -> Vec<f32> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let data = bytes.windows(4).position(|w| w == b"data").expect("data chunk") + 8;
        bytes[data..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32_768.0).collect()
    }

    #[test]
    fn same_voice_scores_higher_than_a_different_voice() {
        let e = Embedder::new().expect("model loads");
        let print = |name: &str| mean(&e.embed_all(&read_fixture(name), 8_000).unwrap()).unwrap();
        let (david_a, david_b, zira) = (print("david_a.wav"), print("david_b.wav"), print("zira_a.wav"));
        let same = cosine(&david_a, &david_b);
        let other = cosine(&david_a, &zira);
        println!("same voice {same:.3}, other voice {other:.3}");
        assert!(same > 0.6, "same voice only {same:.3}");
        assert!(same - other > 0.3, "same {same:.3} vs other {other:.3}");
    }

    #[test]
    fn embedding_is_unit_length_and_fast_enough() {
        let e = Embedder::new().unwrap();
        let audio = read_fixture("zira_a.wav");
        let started = std::time::Instant::now();
        let v = e.embed(&audio[..WINDOW_SAMPLES]).unwrap();
        let took = started.elapsed();
        println!("one embedding: {took:?}, dim {}", v.len());
        assert!((v.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-4);
        assert!(took < std::time::Duration::from_millis(400));
    }

    #[test]
    fn cosine_and_mean_basics() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert_eq!(cosine(&[1.0, 0.0], &[0.0, 1.0]), 0.0);
        let m = mean(&[vec![1.0, 0.0], vec![0.0, 1.0]]).unwrap();
        assert!((m[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!(mean(&[]).is_none());
    }
}
