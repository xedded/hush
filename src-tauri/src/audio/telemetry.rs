//! Levels and load figures collected by the processing thread and read by the UI.
//! The processing thread is not the real-time device callback, so a short
//! mutex here is fine.

use std::sync::Mutex;
use std::time::Duration;

/// Hops (10 ms each) folded into one scope bar: about 33 bars per second.
const HOPS_PER_BAR: usize = 3;
/// Bars kept when the UI is not reading, e.g. while minimized to the tray.
const MAX_PENDING_BARS: usize = 400;
pub const SILENCE_DB: f32 = -90.0;

#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// New scope bars since the last snapshot, peak level in dBFS.
    pub scope_in: Vec<f32>,
    pub scope_out: Vec<f32>,
    pub peak_in_db: f32,
    pub peak_out_db: f32,
    /// Share of total CPU used by the processing thread, in percent.
    pub cpu_pct: f32,
    pub latency_ms: f32,
}

#[derive(Default)]
struct Inner {
    scope_in: Vec<f32>,
    scope_out: Vec<f32>,
    bar_in: f32,
    bar_out: f32,
    hops_in_bar: usize,
    peak_in: f32,
    peak_out: f32,
    busy: Duration,
    audio: Duration,
    latency_ms: f32,
}

#[derive(Default)]
pub struct Telemetry {
    inner: Mutex<Inner>,
    /// Gate detector levels while the voice gate is being calibrated.
    pub levels: super::calibrate::Recorder,
}

pub fn lin_to_db(v: f32) -> f32 {
    if v <= 1e-6 {
        SILENCE_DB
    } else {
        (20.0 * v.log10()).max(SILENCE_DB)
    }
}

fn peak(block: &[f32]) -> f32 {
    block.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

impl Telemetry {
    pub fn set_latency_ms(&self, ms: f32) {
        if let Ok(mut t) = self.inner.lock() {
            t.latency_ms = ms;
        }
    }

    /// Record one processed hop: the raw input, the output, and how long processing took.
    pub fn record_hop(&self, input: &[f32], output: &[f32], busy: Duration, audio: Duration) {
        let Ok(mut t) = self.inner.lock() else { return };
        let (pi, po) = (peak(input), peak(output));
        t.bar_in = t.bar_in.max(pi);
        t.bar_out = t.bar_out.max(po);
        t.peak_in = t.peak_in.max(pi);
        t.peak_out = t.peak_out.max(po);
        t.busy += busy;
        t.audio += audio;
        t.hops_in_bar += 1;
        if t.hops_in_bar >= HOPS_PER_BAR {
            let (bi, bo) = (lin_to_db(t.bar_in), lin_to_db(t.bar_out));
            t.scope_in.push(bi);
            t.scope_out.push(bo);
            if t.scope_in.len() > MAX_PENDING_BARS {
                let excess = t.scope_in.len() - MAX_PENDING_BARS;
                t.scope_in.drain(..excess);
                t.scope_out.drain(..excess);
            }
            t.bar_in = 0.0;
            t.bar_out = 0.0;
            t.hops_in_bar = 0;
        }
    }

    /// Drain pending bars and reset peak and load accumulators.
    pub fn take(&self) -> Snapshot {
        let Ok(mut t) = self.inner.lock() else { return Snapshot::default() };
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f32;
        let cpu_pct = if t.audio.is_zero() {
            0.0
        } else {
            t.busy.as_secs_f32() / t.audio.as_secs_f32() * 100.0 / cores
        };
        let snap = Snapshot {
            scope_in: std::mem::take(&mut t.scope_in),
            scope_out: std::mem::take(&mut t.scope_out),
            peak_in_db: lin_to_db(t.peak_in),
            peak_out_db: lin_to_db(t.peak_out),
            cpu_pct,
            latency_ms: t.latency_ms,
        };
        t.peak_in = 0.0;
        t.peak_out = 0.0;
        t.busy = Duration::ZERO;
        t.audio = Duration::ZERO;
        snap
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOP: Duration = Duration::from_millis(10);

    #[test]
    fn db_conversion_floors_silence() {
        assert_eq!(lin_to_db(0.0), SILENCE_DB);
        assert!((lin_to_db(1.0)).abs() < 1e-6);
        assert!((lin_to_db(0.1) + 20.0).abs() < 1e-4);
    }

    #[test]
    fn bars_are_emitted_every_few_hops_and_drained() {
        let t = Telemetry::default();
        for _ in 0..HOPS_PER_BAR * 2 {
            t.record_hop(&[0.5, -1.0], &[0.1], Duration::from_millis(1), HOP);
        }
        let s = t.take();
        assert_eq!(s.scope_in.len(), 2);
        assert!(s.scope_in[0].abs() < 1e-6);
        assert!((s.peak_out_db + 20.0).abs() < 1e-4);
        assert!(t.take().scope_in.is_empty());
    }

    #[test]
    fn pending_bars_are_capped() {
        let t = Telemetry::default();
        for _ in 0..(MAX_PENDING_BARS + 50) * HOPS_PER_BAR {
            t.record_hop(&[0.1], &[0.1], Duration::ZERO, HOP);
        }
        assert_eq!(t.take().scope_in.len(), MAX_PENDING_BARS);
    }
}
