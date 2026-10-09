//! The audio engine: microphone -> DeepFilterNet -> voice gate -> speaker gate
//! -> voice filter -> virtual microphone (and, on request, your own headphones).
//!
//! Three threads are involved. The cpal input callback downmixes to mono and
//! pushes into a lock-free ring. A dedicated processing thread (which also owns
//! both streams) resamples to 48 kHz, runs the model in 10 ms hops and pushes
//! into a second ring. The cpal output callback drains that ring into VB-Cable.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{Device, ErrorKind, FromSample, SampleFormat, SizedSample, Stream, StreamConfig};
use df::tract::{DfParams, DfTract, RuntimeParams};
use ndarray::{ArrayView2, ArrayViewMut2};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};

use super::devices;
use super::fx::{self, VoiceFx};
use super::gate::Gate;
use super::monitor::Monitor;
use super::params::Params;
use super::resample::Resampler;
use super::telemetry::Telemetry;
use crate::speaker::service::{downsample_48k, Hop, Link};

pub const ENGINE_RATE: u32 = 48_000;
/// Silence queued ahead of the output to absorb scheduling jitter.
pub(super) const OUTPUT_PREFILL: usize = 960;
/// Beyond this much queued output (clock drift between devices) we drop back to the prefill level.
const OUTPUT_MAX_QUEUE: usize = 4_800;
const RING_SECONDS: usize = 1;
const IDLE_WAIT: Duration = Duration::from_millis(2);
/// Typical WASAPI shared-mode period on each side.
const DEVICE_PERIOD_MS: f32 = 10.0;
/// Gain for a speaker who should not pass (-40 dB), and how fast it fades in or out.
const SPEAKER_FLOOR: f32 = 0.01;
const SPEAKER_FADE_MS: f32 = 20.0;

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineInfo {
    pub input_id: String,
    pub input_name: String,
    pub input_rate: u32,
    pub bluetooth_quality: bool,
}

pub struct Engine {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    failure: Arc<Mutex<Option<String>>>,
    info: EngineInfo,
}

impl Engine {
    /// Start the engine on the given input (or the default microphone).
    /// Blocks until the model is loaded and both streams run, or fails.
    pub fn start(
        input_id: Option<String>,
        params: Arc<Params>,
        telemetry: Arc<Telemetry>,
        speaker: Option<Link>,
    ) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let failure = Arc::new(Mutex::new(None));
        let (ready_tx, ready_rx) = mpsc::channel::<Result<EngineInfo>>();

        let worker = {
            let (stop, failure) = (stop.clone(), failure.clone());
            std::thread::Builder::new()
                .name("hush-audio".into())
                .spawn(move || run(input_id, params, telemetry, speaker, stop, failure, ready_tx))
                .context("could not spawn audio thread")?
        };

        match ready_rx.recv() {
            Ok(Ok(info)) => Ok(Self { stop, worker: Some(worker), failure, info }),
            Ok(Err(e)) => {
                let _ = worker.join();
                Err(e)
            }
            Err(_) => {
                let _ = worker.join();
                Err(anyhow!("audio thread exited during startup"))
            }
        }
    }

    pub fn info(&self) -> &EngineInfo {
        &self.info
    }

    /// Set when a stream died, typically because the device was unplugged.
    pub fn failure(&self) -> Option<String> {
        self.failure.lock().ok().and_then(|f| f.clone())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

struct Running {
    _input: Stream,
    _output: Stream,
    in_cons: HeapCons<f32>,
    out_prod: HeapProd<f32>,
    input_rate: u32,
    output_rate: u32,
    df: DfTract,
}

fn run(
    input_id: Option<String>,
    params: Arc<Params>,
    telemetry: Arc<Telemetry>,
    speaker: Option<Link>,
    stop: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<String>>>,
    ready: mpsc::Sender<Result<EngineInfo>>,
) {
    let setup = setup(input_id.as_deref(), &params, &failure);
    let (running, info) = match setup {
        Ok(v) => v,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let latency = DEVICE_PERIOD_MS * 2.0
        + (running.df.hop_size * (1 + running.df.lookahead) + OUTPUT_PREFILL) as f32 * 1000.0 / ENGINE_RATE as f32;
    telemetry.set_latency_ms(latency);
    let _ = ready.send(Ok(info));
    let ctx = Shared { params: &params, telemetry: &telemetry, speaker: speaker.as_ref(), base_latency_ms: latency };
    process_loop(running, ctx, &stop, &failure);
}

fn setup(
    input_id: Option<&str>,
    params: &Params,
    failure: &Arc<Mutex<Option<String>>>,
) -> Result<(Running, EngineInfo)> {
    let input = devices::find_input(input_id).ok_or_else(|| anyhow!("Ingen mikrofon hittades."))?;
    let output = devices::find_virtual_sink()
        .ok_or_else(|| anyhow!(devices::MISSING_SINK_MESSAGE))?;

    let rp = RuntimeParams::default_with_ch(1).with_atten_lim(params.attenuation_db());
    let df = DfTract::new(DfParams::default(), &rp).map_err(|e| anyhow!("Brusmodellen kunde inte laddas: {e}"))?;
    if df.sr != ENGINE_RATE as usize {
        return Err(anyhow!("unexpected model sample rate {}", df.sr));
    }

    let in_cfg = input.default_input_config().context("Mikrofonen kunde inte öppnas.")?;
    let input_rate = in_cfg.sample_rate();
    let in_channels = in_cfg.channels() as usize;
    let (in_prod, in_cons) = HeapRb::<f32>::new(input_rate as usize * RING_SECONDS).split();
    let input_stream = build_input(&input, in_cfg.sample_format(), in_cfg.config(), in_channels, in_prod, failure.clone())?;

    // Run the output at the device's own rate: Windows would convert for us, macOS will not.
    let (out_channels, output_rate) = output
        .default_output_config()
        .map(|c| (c.channels(), c.sample_rate()))
        .unwrap_or((2, ENGINE_RATE));
    let out_cfg = StreamConfig {
        channels: out_channels,
        sample_rate: output_rate,
        buffer_size: cpal::BufferSize::Default,
    };
    let (mut out_prod, out_cons) = HeapRb::<f32>::new(output_rate as usize * RING_SECONDS).split();
    out_prod.push_iter(std::iter::repeat_n(0.0, OUTPUT_PREFILL));
    let output_stream = build_output(&output, out_cfg, out_cons, failure.clone())?;

    input_stream.play().context("Mikrofonen kunde inte startas.")?;
    output_stream.play().context("Den virtuella mikrofonen kunde inte startas.")?;

    let input_id = input.id().map(|id| id.to_string()).unwrap_or_default();
    let info = EngineInfo {
        input_id: input_id.clone(),
        input_name: devices::input_display_name(&input),
        input_rate,
        bluetooth_quality: input_rate <= 16_000 || devices::is_bluetooth(&input, &input_id),
    };
    let running = Running { _input: input_stream, _output: output_stream, in_cons, out_prod, input_rate, output_rate, df };
    Ok((running, info))
}

/// Glitches and route notices leave the stream running. Treating them as fatal
/// made the engine restart every few seconds on Bluetooth headsets, which report
/// an xrun when they switch to the hands-free profile.
fn is_fatal(kind: ErrorKind) -> bool {
    !matches!(kind, ErrorKind::Xrun | ErrorKind::RealtimeDenied | ErrorKind::DeviceChanged)
}

fn report(failure: &Arc<Mutex<Option<String>>>, err: &cpal::Error, msg: &str) {
    if !is_fatal(err.kind()) {
        log::debug!("ignored stream notice: {err}");
        return;
    }
    if let Ok(mut f) = failure.lock() {
        f.get_or_insert_with(|| format!("{msg} ({err})."));
    }
}

fn build_input(
    dev: &Device,
    format: SampleFormat,
    cfg: StreamConfig,
    channels: usize,
    prod: HeapProd<f32>,
    failure: Arc<Mutex<Option<String>>>,
) -> Result<Stream> {
    match format {
        SampleFormat::F32 => input_stream::<f32>(dev, cfg, channels, prod, failure),
        SampleFormat::I16 => input_stream::<i16>(dev, cfg, channels, prod, failure),
        SampleFormat::I32 => input_stream::<i32>(dev, cfg, channels, prod, failure),
        SampleFormat::U16 => input_stream::<u16>(dev, cfg, channels, prod, failure),
        other => Err(anyhow!("Mikrofonens ljudformat stöds inte ({other}).")),
    }
}

fn input_stream<T>(
    dev: &Device,
    cfg: StreamConfig,
    channels: usize,
    mut prod: HeapProd<f32>,
    failure: Arc<Mutex<Option<String>>>,
) -> Result<Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let scale = 1.0 / channels.max(1) as f32;
    dev.build_input_stream::<T, _, _>(
        cfg,
        move |data: &[T], _| {
            let mono = data
                .chunks_exact(channels.max(1))
                .map(|frame| frame.iter().map(|&s| f32::from_sample_(s)).sum::<f32>() * scale);
            // If the processing thread falls behind, newest samples are dropped.
            prod.push_iter(mono);
        },
        move |e| report(&failure, &e, "Mikrofonen kopplades bort"),
        None,
    )
    .context("Mikrofonen kunde inte öppnas.")
}

pub(super) fn build_output(
    dev: &Device,
    cfg: StreamConfig,
    mut cons: HeapCons<f32>,
    failure: Arc<Mutex<Option<String>>>,
) -> Result<Stream> {
    let channels = cfg.channels as usize;
    dev.build_output_stream::<f32, _, _>(
        cfg,
        move |data: &mut [f32], _| {
            let queued = cons.occupied_len();
            if queued > OUTPUT_MAX_QUEUE {
                cons.skip(queued - OUTPUT_PREFILL);
            }
            for frame in data.chunks_exact_mut(channels) {
                let s = cons.try_pop().unwrap_or(0.0);
                frame.fill(s);
            }
        },
        move |e| report(&failure, &e, "Den virtuella mikrofonen slutade svara"),
        None,
    )
    .context("Den virtuella mikrofonen kunde inte öppnas.")
}

/// What the processing thread reads besides its own streams.
struct Shared<'a> {
    params: &'a Params,
    telemetry: &'a Telemetry,
    speaker: Option<&'a Link>,
    /// Latency without a voice filter.
    base_latency_ms: f32,
}

/// Open or close the "listen to yourself" stream to match the setting.
/// A failure switches the setting off so the UI shows the real state.
fn sync_monitor(monitor: &mut Option<Monitor>, params: &Params) {
    if monitor.as_ref().is_some_and(Monitor::failed) {
        log::warn!("monitor output stopped");
        *monitor = None;
        params.set_monitor(false);
    }
    match (params.monitor(), monitor.is_some()) {
        (true, false) => *monitor = Some(Monitor::open()),
        (false, true) => *monitor = None,
        _ => {}
    }
}

fn process_loop(mut r: Running, ctx: Shared, stop: &AtomicBool, failure: &Arc<Mutex<Option<String>>>) {
    let Shared { params, telemetry, speaker, base_latency_ms } = ctx;
    let mut fx = VoiceFx::new(ENGINE_RATE);
    let mut fx_was_active = false;
    let mut monitor: Option<Monitor> = None;
    let hop = r.df.hop_size;
    let mut speaker_gain = 1.0f32;
    let speaker_coef = (-1.0 / (SPEAKER_FADE_MS / 1000.0 * ENGINE_RATE as f32)).exp();
    let hop_duration = Duration::from_secs_f64(hop as f64 / ENGINE_RATE as f64);
    let mut resampler = Resampler::new(r.input_rate, ENGINE_RATE);
    let mut out_resampler = Resampler::new(ENGINE_RATE, r.output_rate);
    let mut out_buf: Vec<f32> = Vec::with_capacity(hop * 2);
    let mut gate = Gate::new(ENGINE_RATE);
    let mut raw = vec![0.0f32; r.input_rate as usize / 50];
    let mut pending: Vec<f32> = Vec::with_capacity(hop * 8);
    let mut enhanced = vec![0.0f32; hop];
    let mut atten_db = params.attenuation_db();

    while !stop.load(Ordering::Relaxed) {
        if failure.lock().map(|f| f.is_some()).unwrap_or(false) {
            break;
        }
        sync_monitor(&mut monitor, params);
        let n = r.in_cons.pop_slice(&mut raw);
        if n == 0 {
            std::thread::sleep(IDLE_WAIT);
            continue;
        }
        resampler.process(&raw[..n], &mut pending);

        let mut offset = 0;
        while pending.len() - offset >= hop {
            let frame = &pending[offset..offset + hop];
            offset += hop;
            let started = Instant::now();

            if params.processing() {
                let wanted = params.attenuation_db();
                if wanted != atten_db {
                    atten_db = wanted;
                    r.df.set_atten_lim(atten_db);
                }
                let noisy = ArrayView2::from_shape((1, hop), frame).expect("hop shape");
                let enh = ArrayViewMut2::from_shape((1, hop), &mut enhanced[..]).expect("hop shape");
                if let Err(e) = r.df.process(noisy, enh) {
                    log::warn!("denoise failed on a hop: {e}");
                    enhanced.copy_from_slice(frame);
                }
                gate.process(&mut enhanced, params.gate_dbfs());
                if let Some(link) = speaker {
                    link.send(Hop { samples: downsample_48k(&enhanced), speech: gate.is_open() });
                    let target = if link.allow() { 1.0 } else { SPEAKER_FLOOR };
                    for s in enhanced.iter_mut() {
                        speaker_gain = target + speaker_coef * (speaker_gain - target);
                        *s *= speaker_gain;
                    }
                }
            } else {
                enhanced.copy_from_slice(frame);
            }
            // The filter follows the speaker gate, so it only ever changes your own voice.
            let mut voice = params.fx();
            voice.enabled &= params.active();
            fx.process(&mut enhanced, &voice);
            if fx.active() != fx_was_active {
                fx_was_active = fx.active();
                let extra = if fx_was_active { fx::latency_ms(ENGINE_RATE) } else { 0.0 };
                telemetry.set_latency_ms(base_latency_ms + extra);
            }
            if params.muted() {
                enhanced.fill(0.0);
            }

            telemetry.record_hop(frame, &enhanced, started.elapsed(), hop_duration);
            if let Some(m) = monitor.as_mut() {
                m.push(&enhanced);
            }
            out_buf.clear();
            out_resampler.process(&enhanced, &mut out_buf);
            r.out_prod.push_slice(&out_buf);
        }
        pending.drain(..offset);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic white noise without pulling in a rand dependency.
    fn noise(n: usize, amp: f32) -> Vec<f32> {
        let mut x: u32 = 0x1234_5678;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x as f32 / u32::MAX as f32 * 2.0 - 1.0) * amp
            })
            .collect()
    }

    fn energy(x: &[f32]) -> f32 {
        x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32
    }

    /// Manual soak test on real hardware: `HUSH_TEST_MIC=Elite cargo test --lib engine_survives -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn engine_survives_on_named_microphone() {
        let wanted = std::env::var("HUSH_TEST_MIC").expect("set HUSH_TEST_MIC to part of a device name");
        let mic = devices::list_inputs().into_iter().find(|i| i.name.contains(&wanted)).expect("microphone not found");
        let params = Arc::new(Params::new(true, crate::audio::params::Mode::Noise, 72.0, -42.0));
        params.set_fx(fx::FxSettings { enabled: true, pitch: -5.0, formant: -20.0, style: fx::Style::Natural });
        let telemetry = Arc::new(Telemetry::default());
        let engine = Engine::start(Some(mic.id), params, telemetry.clone(), None).expect("engine starts");
        println!("running on {} (bluetooth: {})", engine.info().input_name, engine.info().bluetooth_quality);
        for second in 1..=15 {
            std::thread::sleep(Duration::from_secs(1));
            assert_eq!(engine.failure(), None, "engine failed after {second} s");
            let t = telemetry.take();
            println!("{second:>2} s: cpu {:.1} %, latency {:.0} ms", t.cpu_pct, t.latency_ms);
        }
    }

    #[test]
    fn only_real_failures_stop_the_engine() {
        assert!(!is_fatal(ErrorKind::Xrun));
        assert!(!is_fatal(ErrorKind::DeviceChanged));
        assert!(is_fatal(ErrorKind::DeviceNotAvailable));
        assert!(is_fatal(ErrorKind::StreamInvalidated));
    }

    #[test]
    fn model_suppresses_stationary_noise_in_real_time() {
        let rp = RuntimeParams::default_with_ch(1).with_atten_lim(40.0);
        let mut df = DfTract::new(DfParams::default(), &rp).expect("model loads");
        let hop = df.hop_size;
        let input = noise(ENGINE_RATE as usize * 3, 0.03);
        let mut output = vec![0.0f32; input.len()];
        let started = Instant::now();
        for (i, o) in input.chunks_exact(hop).zip(output.chunks_exact_mut(hop)) {
            let noisy = ArrayView2::from_shape((1, hop), i).unwrap();
            let enh = ArrayViewMut2::from_shape((1, hop), o).unwrap();
            df.process(noisy, enh).unwrap();
        }
        let per_hop = started.elapsed() / (input.len() / hop) as u32;
        // Judge the last second, after the model has adapted.
        let tail = input.len() - ENGINE_RATE as usize;
        let reduction_db = 10.0 * (energy(&input[tail..]) / energy(&output[tail..]).max(1e-12)).log10();
        println!("noise reduction {reduction_db:.1} dB, {per_hop:?} per 10 ms hop");
        assert!(reduction_db > 15.0, "only {reduction_db:.1} dB");
        // Real time is 10 ms per hop; leave headroom for slow shared CI machines.
        assert!(per_hop < Duration::from_millis(8), "too slow: {per_hop:?}");
    }
}
