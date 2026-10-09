//! "Listen to yourself": a copy of the outgoing sound on the default playback
//! device, so the user can hear what the meeting hears.
//!
//! Opening and closing a device can take hundreds of milliseconds, far longer
//! than the virtual microphone's buffer, so the stream lives on a thread of its
//! own. The processing thread only ever pushes into a ring.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{Stream, StreamConfig};
use ringbuf::traits::{Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};

use super::devices;
use super::engine::{build_output, ENGINE_RATE, OUTPUT_PREFILL};
use super::resample::Resampler;

/// One second at the highest common device rate. Whatever piles up while the
/// device opens is trimmed by the output callback once it runs.
const RING: usize = 96_000;

type Failure = Arc<Mutex<Option<String>>>;

pub struct Monitor {
    prod: HeapProd<f32>,
    /// Device rate, known once the stream thread has opened the device.
    rate: mpsc::Receiver<u32>,
    resampler: Option<Resampler>,
    buf: Vec<f32>,
    failure: Failure,
    /// Dropping this tells the stream thread to close the device.
    _stop: mpsc::Sender<()>,
}

fn fail(failure: &Failure, msg: String) {
    if let Ok(mut f) = failure.lock() {
        f.get_or_insert(msg);
    }
}

/// Open the default playback device. Runs on the stream thread.
fn play(cons: HeapCons<f32>, failure: Failure) -> Result<(Stream, u32)> {
    let dev = devices::monitor_output().map_err(anyhow::Error::msg)?;
    let cfg = dev.default_output_config().context("no output config")?;
    let rate = cfg.sample_rate();
    let cfg = StreamConfig { channels: cfg.channels(), sample_rate: rate, buffer_size: cpal::BufferSize::Default };
    let stream = build_output(&dev, cfg, cons, failure)?;
    stream.play().context("output did not start")?;
    Ok((stream, rate))
}

impl Monitor {
    /// Starts opening the default playback device and returns at once.
    /// If that fails, `failed()` turns true shortly after.
    pub fn open() -> Self {
        let (mut prod, cons) = HeapRb::<f32>::new(RING).split();
        prod.push_iter(std::iter::repeat_n(0.0, OUTPUT_PREFILL));
        let failure: Failure = Arc::new(Mutex::new(None));
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (rate_tx, rate_rx) = mpsc::channel::<u32>();
        let thread_failure = failure.clone();
        let spawned = std::thread::Builder::new().name("hush-monitor-out".into()).spawn(move || {
            match play(cons, thread_failure.clone()) {
                Ok((stream, rate)) => {
                    let _ = rate_tx.send(rate);
                    // Returns when the Monitor is dropped.
                    let _ = stop_rx.recv();
                    drop(stream);
                }
                Err(e) => {
                    log::warn!("monitor output could not open: {e:#}");
                    fail(&thread_failure, e.to_string());
                }
            }
        });
        if let Err(e) = spawned {
            fail(&failure, e.to_string());
        }
        Self { prod, rate: rate_rx, resampler: None, buf: Vec::with_capacity(4_096), failure, _stop: stop_tx }
    }

    /// Queue one processed block (48 kHz mono). Dropped until the device is open.
    pub fn push(&mut self, block: &[f32]) {
        if self.resampler.is_none() {
            let Ok(rate) = self.rate.try_recv() else { return };
            self.resampler = Some(Resampler::new(ENGINE_RATE, rate));
        }
        let Some(r) = self.resampler.as_mut() else { return };
        self.buf.clear();
        r.process(block, &mut self.buf);
        self.prod.push_slice(&self.buf);
    }

    /// The device could not be opened or went away, e.g. headphones unplugged.
    pub fn failed(&self) -> bool {
        self.failure.lock().map(|f| f.is_some()).unwrap_or(true)
    }
}
