//! Voice gate calibration: measure the room while the user is quiet, then
//! while they speak, and place the threshold just below their voice. Distant
//! voices and room noise then stay under it.

use std::sync::Mutex;

use super::params::{GATE_MAX_DBFS, GATE_MIN_DBFS};

/// How far below the quieter part of the user's speech the gate opens.
const BELOW_SPEECH_DB: f32 = 8.0;
/// The gate closes 6 dB under its threshold; keep that above the room.
const ABOVE_ROOM_DB: f32 = 7.0;
/// Speech must stand this far above the room to calibrate at all.
const MIN_GAP_DB: f32 = 10.0;
/// A speech hop counts as speech when this far above the room.
const SPEECH_OVER_ROOM_DB: f32 = 3.0;
/// At least this share of the speaking phase must be speech.
const MIN_SPEECH_SHARE: f32 = 0.25;

/// Levels the gate's detector saw, one per 10 ms hop, while a measurement runs.
#[derive(Default)]
pub struct Recorder {
    levels: Mutex<Option<Vec<f32>>>,
}

impl Recorder {
    pub fn start(&self) {
        if let Ok(mut l) = self.levels.lock() {
            *l = Some(Vec::with_capacity(1_000));
        }
    }

    pub fn stop(&self) -> Vec<f32> {
        self.levels.lock().ok().and_then(|mut l| l.take()).unwrap_or_default()
    }

    /// Called by the processing thread for every hop.
    pub fn record(&self, level_db: f32) {
        if let Ok(mut l) = self.levels.lock() {
            if let Some(v) = l.as_mut() {
                v.push(level_db);
            }
        }
    }
}

fn percentile(values: &[f32], p: f32) -> Option<f32> {
    let mut v: Vec<f32> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f32::total_cmp);
    let i = ((v.len() - 1) as f32 * p).round() as usize;
    Some(v[i])
}

/// The threshold for a quiet phase and a speaking phase, or why it cannot be set.
pub fn suggest(quiet: &[f32], speech: &[f32]) -> Result<f32, &'static str> {
    let room = percentile(quiet, 0.95).ok_or("Hush hörde ingenting. Kontrollera att mikrofonen är på.")?;
    let spoken: Vec<f32> = speech.iter().copied().filter(|&l| l > room + SPEECH_OVER_ROOM_DB).collect();
    if (spoken.len() as f32) < speech.len() as f32 * MIN_SPEECH_SHARE {
        return Err("Hush hörde nästan inget tal. Prata som vanligt medan mätaren går.");
    }
    // The quieter part of the speech, so soft syllables still open the gate.
    let voice = percentile(&spoken, 0.3).expect("not empty");
    if voice - room < MIN_GAP_DB {
        return Err("Rösten hördes knappt över bakgrundsljudet. Prata närmare mikrofonen eller välj en annan mikrofon.");
    }
    let threshold = (voice - BELOW_SPEECH_DB).max(room + ABOVE_ROOM_DB);
    Ok(threshold.round().clamp(GATE_MIN_DBFS, GATE_MAX_DBFS))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn around(level: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| level + ((i * 7) % 11) as f32 * 0.5 - 2.5).collect()
    }

    #[test]
    fn threshold_sits_between_room_and_voice() {
        let quiet = around(-62.0, 300);
        let mut speech = around(-30.0, 280);
        speech.extend(around(-62.0, 120)); // pauses between words
        let t = suggest(&quiet, &speech).unwrap();
        println!("threshold {t}");
        assert!(t > -55.0 && t < -32.0, "{t}");
    }

    #[test]
    fn noisy_room_keeps_the_gate_above_the_noise() {
        let t = suggest(&around(-40.0, 300), &around(-22.0, 300)).unwrap();
        assert!(t >= -33.0, "gate would chatter on the room: {t}");
    }

    #[test]
    fn refuses_when_the_voice_is_not_heard() {
        assert!(suggest(&around(-50.0, 300), &around(-50.0, 300)).is_err());
        assert!(suggest(&around(-50.0, 300), &around(-44.0, 300)).is_err());
        assert!(suggest(&[], &around(-30.0, 300)).is_err());
    }

    #[test]
    fn result_stays_inside_the_slider() {
        assert_eq!(suggest(&around(-95.0, 300), &around(-75.0, 300)).unwrap(), GATE_MIN_DBFS);
    }

    #[test]
    fn recorder_collects_only_while_running() {
        let r = Recorder::default();
        r.record(-40.0);
        r.start();
        r.record(-30.0);
        r.record(-31.0);
        assert_eq!(r.stop(), vec![-30.0, -31.0]);
        r.record(-20.0);
        assert!(r.stop().is_empty());
    }
}
