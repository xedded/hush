//! Counts talk time and loudness for whoever is speaking, for the statistics
//! page. Speech is collected per turn and credited to a voice once it is
//! recognised, so the half second before identification counts too. Speech
//! that is never recognised is not counted.

use std::time::{Duration, Instant};

use super::library::Library;

/// Loudness is measured over windows this long (samples at 16 kHz): long
/// enough that a single click does not count as the loudest moment.
const PEAK_WINDOW: usize = 3_200;
/// A long monologue is credited this often, so the page stays current.
const FLUSH_EVERY: Duration = Duration::from_secs(2);

pub struct Tally {
    rate: f64,
    owner: Option<String>,
    seconds: f64,
    energy: f64,
    peak_db: Option<f32>,
    window_sq: f64,
    window_n: usize,
    new_turn: bool,
    last_flush: Instant,
}

impl Tally {
    pub fn new(rate: usize) -> Self {
        Self {
            rate: rate as f64,
            owner: None,
            seconds: 0.0,
            energy: 0.0,
            peak_db: None,
            window_sq: 0.0,
            window_n: 0,
            new_turn: false,
            last_flush: Instant::now(),
        }
    }

    fn clear(&mut self) {
        self.seconds = 0.0;
        self.energy = 0.0;
        self.peak_db = None;
    }

    /// One hop of speech (16 kHz).
    pub fn hop(&mut self, samples: &[f32]) {
        let sq: f64 = samples.iter().map(|&v| (v as f64) * (v as f64)).sum();
        self.seconds += samples.len() as f64 / self.rate;
        self.energy += sq / self.rate;
        self.window_sq += sq;
        self.window_n += samples.len();
        if self.window_n >= PEAK_WINDOW {
            let db = (10.0 * (self.window_sq / self.window_n as f64).max(1e-12).log10()) as f32;
            self.peak_db = Some(self.peak_db.map_or(db, |p| p.max(db)));
            self.window_sq = 0.0;
            self.window_n = 0;
        }
    }

    /// Who is speaking now: "me", a voice id, or None while not recognised.
    pub fn speaker(&mut self, who: Option<&str>, lib: &mut Library, today: &str) {
        if who == self.owner.as_deref() {
            return;
        }
        match (&self.owner, who) {
            // First identification in the turn: the speech so far is theirs.
            (None, Some(id)) => {
                self.owner = Some(id.to_string());
                self.new_turn = true;
            }
            (Some(_), next) => {
                self.flush(lib, today);
                self.clear();
                self.owner = next.map(str::to_string);
                self.new_turn = next.is_some();
            }
            (None, None) => {}
        }
    }

    /// Silence ended the turn.
    pub fn end_turn(&mut self, lib: &mut Library, today: &str) {
        self.flush(lib, today);
        self.clear();
        self.owner = None;
        self.window_sq = 0.0;
        self.window_n = 0;
    }

    pub fn flush_if_due(&mut self, lib: &mut Library, today: &str) {
        if self.owner.is_some() && self.last_flush.elapsed() >= FLUSH_EVERY {
            self.flush(lib, today);
        }
    }

    fn flush(&mut self, lib: &mut Library, today: &str) {
        self.last_flush = Instant::now();
        let Some(owner) = self.owner.as_deref() else { return };
        if self.seconds <= 0.0 && !self.new_turn {
            return;
        }
        if let Some(stats) = lib.stats_mut(owner) {
            stats.add(self.seconds, self.energy, self.peak_db, today);
            if self.new_turn {
                stats.turns += 1;
            }
            lib.changes += 1;
        }
        self.new_turn = false;
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: &str = "2026-10-09";

    fn speech(seconds: f32, amp: f32) -> Vec<f32> {
        (0..(16_000.0 * seconds) as usize).map(|i| amp * (i as f32 * 0.07).sin()).collect()
    }

    fn feed(t: &mut Tally, x: &[f32]) {
        x.chunks(160).for_each(|h| t.hop(h));
    }

    #[test]
    fn speech_before_identification_is_credited_to_the_speaker() {
        let mut lib = Library::default();
        let mut t = Tally::new(16_000);
        feed(&mut t, &speech(0.5, 0.1));
        t.speaker(Some("me"), &mut lib, DAY);
        feed(&mut t, &speech(1.5, 0.1));
        t.end_turn(&mut lib, DAY);
        let s = &lib.me_stats;
        assert!((s.seconds - 2.0).abs() < 0.02, "{}", s.seconds);
        assert_eq!(s.turns, 1);
        assert!((s.days[DAY] - 2.0).abs() < 0.02);
        // A sine of amplitude 0.1 has a mean square of 0.005, about -23 dBFS.
        assert!((s.mean_db().unwrap() + 23.0).abs() < 0.5);
        assert!((s.peak_db.unwrap() + 23.0).abs() < 0.5);
    }

    #[test]
    fn unrecognised_speech_is_not_counted() {
        let mut lib = Library::default();
        let mut t = Tally::new(16_000);
        feed(&mut t, &speech(1.0, 0.1));
        t.end_turn(&mut lib, DAY);
        assert_eq!(lib.me_stats.seconds, 0.0);
        assert_eq!(lib.changes, 0);
    }

    #[test]
    fn a_change_of_speaker_splits_the_time() {
        let mut lib = Library::default();
        let id = lib.add(vec![1.0], 1, DAY);
        let mut t = Tally::new(16_000);
        t.speaker(Some("me"), &mut lib, DAY);
        feed(&mut t, &speech(1.0, 0.1));
        t.speaker(Some(&id), &mut lib, DAY);
        feed(&mut t, &speech(3.0, 0.3));
        t.end_turn(&mut lib, DAY);
        assert!((lib.me_stats.seconds - 1.0).abs() < 0.02);
        let other = &lib.voice(&id).unwrap().stats;
        assert!((other.seconds - 3.0).abs() < 0.02);
        assert_eq!((lib.me_stats.turns, other.turns), (1, 1));
        assert!(other.mean_db().unwrap() > lib.me_stats.mean_db().unwrap() + 8.0);
    }
}
