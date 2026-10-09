//! Manual diagnosis on real hardware: listens to a microphone next to the
//! running app, runs the same chain (denoise, gate, embeddings) and prints how
//! each piece of speech scores against the saved voice library. Read-only.
//!
//! `HUSH_TEST_MIC="Jabra Link" cargo test --release --lib live_scores -- --ignored --nocapture`

use std::sync::mpsc;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, StreamTrait};
use df::tract::{DfParams, DfTract, RuntimeParams};
use ndarray::{ArrayView2, ArrayViewMut2};

use super::embedder::{cosine, Embedder, WINDOW_SAMPLES};
use super::library::{Keychain, Store};
use super::service::downsample_48k;
use crate::audio::{devices, gate::Gate, resample::Resampler};

const SECONDS: u64 = 40;
const EMBED_EVERY: usize = 8_000;

fn level_db(x: &[f32]) -> f32 {
    10.0 * (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).max(1e-12).log10()
}

#[test]
#[ignore]
fn live_scores() {
    let wanted = std::env::var("HUSH_TEST_MIC").expect("set HUSH_TEST_MIC");
    let mic = devices::list_inputs().into_iter().find(|i| i.name.contains(&wanted)).expect("mic");
    let dir = std::path::PathBuf::from(std::env::var("APPDATA").unwrap()).join("se.segerstad.hush");
    let lib = Store::new(&dir, Box::new(Keychain)).load().expect("library");
    let settings = crate::settings::Store::new(&dir).load();
    println!("profile: {}, voices: {}, gate {} dBFS", lib.me.is_some(), lib.voices.len(), settings.gate_dbfs);
    let me = lib.me.clone().expect("no profile recorded");

    let dev = devices::find_input(Some(&mic.id)).unwrap();
    let cfg = dev.default_input_config().unwrap();
    let (rate, ch) = (cfg.sample_rate(), cfg.channels() as usize);
    let (tx, rx) = mpsc::channel::<Vec<f32>>();
    let stream = dev
        .build_input_stream::<f32, _, _>(
            cfg.config(),
            move |d: &[f32], _| {
                let _ = tx.send(d.chunks_exact(ch).map(|f| f.iter().sum::<f32>() / ch as f32).collect());
            },
            |e| eprintln!("stream: {e}"),
            None,
        )
        .unwrap();
    stream.play().unwrap();

    let rp = RuntimeParams::default_with_ch(1).with_atten_lim(settings.suppression / 100.0 * 45.0);
    let mut df = DfTract::new(DfParams::default(), &rp).unwrap();
    let hop = df.hop_size;
    let embedder = Embedder::new().unwrap();
    let mut gate = Gate::new(48_000);
    let mut rs = Resampler::new(rate, 48_000);
    let (mut pending, mut out) = (Vec::new(), vec![0.0f32; hop]);
    let (mut turn, mut since): (Vec<f32>, usize) = (Vec::new(), 0);
    let (mut open_hops, mut hops) = (0usize, 0usize);
    let started = Instant::now();
    println!("  t   level   me    best voice");
    while started.elapsed() < Duration::from_secs(SECONDS) {
        let Ok(block) = rx.recv_timeout(Duration::from_millis(200)) else { continue };
        rs.process(&block, &mut pending);
        let mut off = 0;
        while pending.len() - off >= hop {
            let frame = &pending[off..off + hop];
            off += hop;
            df.process(ArrayView2::from_shape((1, hop), frame).unwrap(), ArrayViewMut2::from_shape((1, hop), &mut out[..]).unwrap())
                .unwrap();
            gate.process(&mut out, settings.gate_dbfs);
            hops += 1;
            if !gate.is_open() {
                continue;
            }
            open_hops += 1;
            let s = downsample_48k(&out);
            since += s.len();
            turn.extend_from_slice(&s);
            if turn.len() > WINDOW_SAMPLES {
                turn.drain(..turn.len() - WINDOW_SAMPLES);
            }
            if turn.len() == WINDOW_SAMPLES && since >= EMBED_EVERY {
                since = 0;
                let e = embedder.embed(&turn).unwrap();
                let best = lib
                    .voices
                    .iter()
                    .map(|v| (v.name.as_str(), cosine(&v.print, &e)))
                    .max_by(|a, b| a.1.total_cmp(&b.1));
                println!(
                    "{:5.1} {:6.1}  {:.2}  {}",
                    started.elapsed().as_secs_f32(),
                    level_db(&turn),
                    cosine(&me, &e),
                    best.map_or("-".to_string(), |(n, s)| format!("{n} {s:.2}"))
                );
            }
        }
        pending.drain(..off);
    }
    println!("gate open {:.0} % of the time", open_hops as f32 * 100.0 / hops.max(1) as f32);
}
