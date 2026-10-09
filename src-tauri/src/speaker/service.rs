//! Speaker recognition service. Runs on its own thread: the audio engine sends
//! it 10 ms hops of denoised 16 kHz audio, it works out who is speaking and
//! publishes one bit back, whether the current speaker may pass. It also owns
//! the meeting's voice list, enrollment of the user's own voice and saving the
//! library.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};

use super::embedder::{self, Embedder, WINDOW_SAMPLES};
use super::library::{Library, Policy, Store};
use super::tally::Tally;
use super::tracker::{self, Tracker, Who};
use crate::audio::params::{Mode, Params};

const RATE: usize = 16_000;
/// Speech needed before the first identification of a turn.
const FIRST_ID_SAMPLES: usize = RATE / 2;
/// Re-identify this often while someone keeps talking.
const EMBED_EVERY: usize = RATE / 2;
/// Silence that ends a turn.
const TURN_GAP: usize = RATE * 6 / 10;
/// How long after the user spoke an unidentified turn gets the benefit of the doubt.
const ME_RECENT: Duration = Duration::from_secs(60);
/// Voices drop off the meeting list after this long without speaking.
const SESSION_EXPIRY: Duration = Duration::from_secs(15 * 60);
/// Learned changes to voiceprints are written at most this often.
const SAVE_EVERY: Duration = Duration::from_secs(10);
/// Speech needed to enroll the user's own voice.
pub const ENROLL_SECONDS: f32 = 30.0;
/// Enrollment windows must agree with their mean at least this well on average.
const ENROLL_CONSISTENCY: f32 = 0.6;
const QUEUE_HOPS: usize = 300;

/// One 10 ms hop from the engine.
pub struct Hop {
    pub samples: Vec<f32>,
    pub speech: bool,
}

/// The engine's side of the connection.
#[derive(Clone)]
pub struct Link {
    tx: SyncSender<Hop>,
    allow: Arc<AtomicBool>,
}

impl Link {
    pub fn send(&self, hop: Hop) {
        // If the service falls behind, drop audio rather than block the engine.
        let _ = self.tx.try_send(hop);
    }
    pub fn allow(&self) -> bool {
        self.allow.load(Ordering::Relaxed)
    }
}

/// A voice heard recently: drives "Pratar nu" and the activity lane.
struct SessionVoice {
    id: String,
    last: Instant,
    score: f32,
}

#[derive(Default)]
struct Live {
    speaking: Option<String>,
}

struct Enrollment {
    speech: Vec<f32>,
}

pub struct Shared {
    lib: Mutex<Library>,
    store: Store,
    session: Mutex<Vec<SessionVoice>>,
    live: Mutex<Live>,
    enrollment: Mutex<Option<Enrollment>>,
    enroll_result: Mutex<Option<EnrollResult>>,
    error: Mutex<Option<String>>,
    /// Bumped whenever the voices view changes, so the UI only reloads then.
    version: AtomicU64,
    dirty: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrollResult {
    pub ok: bool,
    pub message: String,
}

/// One voice in the library. Its single choice, heard or muted, applies at
/// once and is remembered for the next meeting.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceView {
    pub id: String,
    pub name: String,
    pub named: bool,
    pub heard: bool,
    pub last_heard: String,
    /// Heard in the last 15 minutes.
    pub recent: bool,
    /// Latest match score while recent.
    pub score: Option<f32>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrollView {
    pub seconds: f32,
    pub needed: f32,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoicesView {
    pub enrolled: bool,
    pub enrollment: Option<EnrollView>,
    pub unknown: Policy,
    /// Recently heard voices first, then by when they were last heard.
    pub voices: Vec<VoiceView>,
    pub error: Option<String>,
}

/// Today's date (UTC) as YYYY-MM-DD, without a date library.
pub fn today() -> String {
    civil_date(day_number())
}

fn day_number() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400) as i64
}

/// The last `n` dates, oldest first, ending today.
pub fn recent_days(n: usize) -> Vec<String> {
    let today = day_number();
    (0..n as i64).rev().map(|back| civil_date(today - back)).collect()
}

/// Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
fn civil_date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Average 48 kHz audio down to 16 kHz (box filter; enough for voiceprints).
pub fn downsample_48k(samples: &[f32]) -> Vec<f32> {
    samples.chunks_exact(3).map(|c| (c[0] + c[1] + c[2]) / 3.0).collect()
}

/// Repeat short speech until it fills one model window.
fn tile(speech: &[f32]) -> Vec<f32> {
    speech.iter().copied().cycle().take(WINDOW_SAMPLES).collect()
}

pub struct Speakers {
    shared: Arc<Shared>,
    link: Link,
}

impl Speakers {
    pub fn start(store: Store, params: Arc<Params>) -> Self {
        let (lib, error) = match store.load() {
            Ok(lib) => (lib, None),
            Err(e) => {
                log::error!("voice library: {e:#}");
                (Library::default(), Some("Röstbiblioteket kunde inte läsas. Nya röster sparas i ett nytt bibliotek.".to_string()))
            }
        };
        let shared = Arc::new(Shared {
            lib: Mutex::new(lib),
            store,
            session: Mutex::new(Vec::new()),
            live: Mutex::new(Live::default()),
            enrollment: Mutex::new(None),
            enroll_result: Mutex::new(None),
            error: Mutex::new(error),
            version: AtomicU64::new(1),
            dirty: AtomicBool::new(false),
        });
        let (tx, rx) = mpsc::sync_channel(QUEUE_HOPS);
        let allow = Arc::new(AtomicBool::new(true));
        let link = Link { tx, allow: allow.clone() };
        let worker_shared = shared.clone();
        std::thread::Builder::new()
            .name("hush-speaker".into())
            .spawn(move || Worker::new(worker_shared, params, allow).run(rx))
            .expect("spawn speaker thread");
        Self { shared, link }
    }

    pub fn link(&self) -> Link {
        self.link.clone()
    }

    pub fn version(&self) -> u64 {
        self.shared.version.load(Ordering::Relaxed)
    }

    pub fn speaking(&self) -> Option<String> {
        lock(&self.shared.live).speaking.clone()
    }

    pub fn take_enroll_result(&self) -> Option<EnrollResult> {
        lock(&self.shared.enroll_result).take()
    }

    fn changed(&self) {
        self.shared.version.fetch_add(1, Ordering::Relaxed);
    }

    /// Persist now; used after every user edit so deletions really are gone from disk.
    fn save_now(&self) -> Result<()> {
        let lib = lock(&self.shared.lib).clone();
        self.shared.dirty.store(false, Ordering::Relaxed);
        self.shared.store.save(&lib).map_err(|e| {
            log::error!("saving voice library: {e:#}");
            anyhow!("Röstbiblioteket kunde inte sparas.")
        })
    }

    pub fn view(&self) -> VoicesView {
        let lib = lock(&self.shared.lib);
        let session = lock(&self.shared.session);
        let recent = |id: &str| session.iter().find(|s| s.id == id);
        let mut voices: Vec<VoiceView> = lib
            .voices
            .iter()
            .map(|v| VoiceView {
                id: v.id.clone(),
                name: v.name.clone(),
                named: v.named,
                heard: v.default == Policy::Pass,
                last_heard: v.last_heard.clone(),
                recent: recent(&v.id).is_some(),
                score: recent(&v.id).map(|s| s.score),
            })
            .collect();
        let since = |id: &str| recent(id).map(|s| s.last.elapsed());
        voices.sort_by(|a, b| match (since(&a.id), since(&b.id)) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => b.last_heard.cmp(&a.last_heard).then_with(|| a.name.cmp(&b.name)),
        });
        VoicesView {
            enrolled: lib.me.is_some(),
            enrollment: lock(&self.shared.enrollment).as_ref().map(|e| EnrollView {
                seconds: e.speech.len() as f32 / RATE as f32,
                needed: ENROLL_SECONDS,
            }),
            unknown: lib.unknown_policy(),
            voices,
            error: lock(&self.shared.error).clone(),
        }
    }

    pub fn stats(&self) -> super::stats::StatsView {
        super::stats::view(&lock(&self.shared.lib))
    }

    pub fn reset_stats(&self) -> Result<()> {
        lock(&self.shared.lib).reset_stats();
        self.save_now()
    }

    pub fn rename(&self, id: &str, name: &str) -> Result<()> {
        lock(&self.shared.lib).rename(id, name)?;
        self.changed();
        self.save_now()
    }

    pub fn set_default(&self, id: &str, policy: Policy) -> Result<()> {
        lock(&self.shared.lib).set_default(id, policy)?;
        self.changed();
        self.save_now()
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        if !lock(&self.shared.lib).delete(id) {
            return Err(anyhow!("Rösten finns inte längre."));
        }
        lock(&self.shared.session).retain(|s| s.id != id);
        self.changed();
        self.save_now()
    }

    pub fn set_unknown(&self, policy: Policy) -> Result<()> {
        lock(&self.shared.lib).unknown = Some(policy);
        self.changed();
        self.save_now()
    }

    pub fn start_enrollment(&self) {
        *lock(&self.shared.enrollment) = Some(Enrollment { speech: Vec::with_capacity(RATE * 31) });
        *lock(&self.shared.enroll_result) = None;
        self.changed();
    }

    pub fn cancel_enrollment(&self) {
        *lock(&self.shared.enrollment) = None;
        self.changed();
    }
}

struct Worker {
    shared: Arc<Shared>,
    params: Arc<Params>,
    allow: Arc<AtomicBool>,
    embedder: Option<Embedder>,
    tracker: Tracker,
    turn: Vec<f32>,
    since_embed: usize,
    silence: usize,
    in_turn: bool,
    turn_ids: usize,
    current: Who,
    last_me: Option<Instant>,
    last_save: Instant,
    last_expiry: Instant,
    last_enroll_tick: f32,
    tally: Tally,
    saved_changes: u64,
}

impl Worker {
    fn new(shared: Arc<Shared>, params: Arc<Params>, allow: Arc<AtomicBool>) -> Self {
        Self {
            shared,
            params,
            allow,
            embedder: None,
            tracker: Tracker::default(),
            turn: Vec::with_capacity(WINDOW_SAMPLES),
            since_embed: 0,
            silence: 0,
            in_turn: false,
            turn_ids: 0,
            current: Who::Unknown,
            last_me: None,
            last_save: Instant::now(),
            last_expiry: Instant::now(),
            last_enroll_tick: 0.0,
            tally: Tally::new(RATE),
            saved_changes: 0,
        }
    }

    fn run(mut self, rx: Receiver<Hop>) {
        match Embedder::new() {
            Ok(e) => self.embedder = Some(e),
            Err(e) => {
                log::error!("speaker model: {e:#}");
                *lock(&self.shared.error) = Some("Röstigenkänningen kunde inte starta.".into());
            }
        }
        loop {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(hop) => self.hop(hop),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.housekeeping();
        }
    }

    fn bump(&self) {
        self.shared.version.fetch_add(1, Ordering::Relaxed);
    }

    fn hop(&mut self, hop: Hop) {
        if self.enroll(&hop) {
            self.allow.store(true, Ordering::Relaxed);
            return;
        }
        let enrolled = lock(&self.shared.lib).me.is_some();
        if !enrolled || self.embedder.is_none() || !self.params.processing() {
            // Nothing to recognise against: everyone passes.
            self.allow.store(true, Ordering::Relaxed);
            lock(&self.shared.live).speaking = None;
            return;
        }

        if !hop.speech {
            self.silence += hop.samples.len();
            if self.silence >= TURN_GAP && self.in_turn {
                self.in_turn = false;
                self.tally.end_turn(&mut lock(&self.shared.lib), &today());
                lock(&self.shared.live).speaking = None;
            }
            return;
        }
        if !self.in_turn {
            self.in_turn = true;
            self.turn.clear();
            self.since_embed = 0;
            self.turn_ids = 0;
            self.current = Who::Unknown;
            self.tracker.new_turn();
        }
        self.silence = 0;
        self.tally.hop(&hop.samples);
        self.turn.extend_from_slice(&hop.samples);
        if self.turn.len() > WINDOW_SAMPLES {
            let excess = self.turn.len() - WINDOW_SAMPLES;
            self.turn.drain(..excess);
        }
        self.since_embed += hop.samples.len();

        let due = if self.turn_ids == 0 { FIRST_ID_SAMPLES } else { EMBED_EVERY };
        if self.turn.len() >= FIRST_ID_SAMPLES && self.since_embed >= due {
            self.since_embed = 0;
            self.identify();
        }
        self.decide();
    }

    fn identify(&mut self) {
        let Some(embedder) = &self.embedder else { return };
        let window = if self.turn.len() >= WINDOW_SAMPLES { self.turn.clone() } else { tile(&self.turn) };
        let emb = match embedder.embed(&window) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("embedding failed: {e:#}");
                return;
            }
        };
        let day = today();
        let obs = {
            let mut lib = lock(&self.shared.lib);
            let obs = self.tracker.observe(&mut lib, &emb, &day);
            let who = match &obs.who {
                Who::Me => Some("me"),
                Who::Voice(id) => Some(id.as_str()),
                Who::Unknown => None,
            };
            self.tally.speaker(who, &mut lib, &day);
            obs
        };
        self.turn_ids += 1;
        self.current = obs.who.clone();

        match &obs.who {
            Who::Me => self.last_me = Some(Instant::now()),
            Who::Voice(id) => self.note_voice(id, obs.score),
            Who::Unknown => {}
        }
        if let Some(id) = &obs.created {
            log::info!("new voice {id}");
            self.bump();
        }
    }

    /// Mark a recognised voice as heard just now.
    fn note_voice(&self, id: &str, score: f32) {
        let mut session = lock(&self.shared.session);
        if let Some(s) = session.iter_mut().find(|s| s.id == id) {
            s.last = Instant::now();
            s.score = score;
            return;
        }
        session.push(SessionVoice { id: id.to_string(), last: Instant::now(), score });
        drop(session);
        self.bump();
    }

    fn decide(&self) {
        let only_me = self.params.mode() == Mode::Me;
        let me_recent = self.last_me.is_some_and(|t| t.elapsed() < ME_RECENT);
        let lib = lock(&self.shared.lib);
        let heard = |id: &str| lib.voice(id).is_some_and(|v| v.default == Policy::Pass);
        let pass = tracker::allowed(&self.current, only_me, heard, lib.unknown_policy(), me_recent && self.turn_ids == 0);
        drop(lib);
        self.allow.store(pass, Ordering::Relaxed);
        lock(&self.shared.live).speaking = Some(match &self.current {
            Who::Me => "me".to_string(),
            Who::Voice(id) => id.clone(),
            Who::Unknown => "unknown".to_string(),
        });
    }

    /// Returns true while enrolling (the hop was consumed).
    fn enroll(&mut self, hop: &Hop) -> bool {
        let mut guard = lock(&self.shared.enrollment);
        let Some(enrollment) = guard.as_mut() else { return false };
        if hop.speech {
            enrollment.speech.extend_from_slice(&hop.samples);
        }
        let seconds = enrollment.speech.len() as f32 / RATE as f32;
        if seconds - self.last_enroll_tick >= 0.5 {
            self.last_enroll_tick = seconds;
            self.bump();
        }
        if seconds < ENROLL_SECONDS {
            return true;
        }
        let speech = std::mem::take(&mut enrollment.speech);
        *guard = None;
        drop(guard);
        self.last_enroll_tick = 0.0;
        let result = self.finish_enrollment(&speech);
        *lock(&self.shared.enroll_result) = Some(result);
        self.bump();
        true
    }

    fn finish_enrollment(&mut self, speech: &[f32]) -> EnrollResult {
        let fail = |message: &str| EnrollResult { ok: false, message: message.into() };
        let Some(embedder) = &self.embedder else { return fail("Röstigenkänningen är inte igång.") };
        let windows = match embedder.embed_all(speech, RATE * 3 / 4) {
            Ok(w) if !w.is_empty() => w,
            _ => return fail("Inspelningen kunde inte analyseras. Försök igen."),
        };
        let Some(print) = embedder::mean(&windows) else { return fail("Inspelningen var för kort.") };
        let consistency = windows.iter().map(|w| embedder::cosine(w, &print)).sum::<f32>() / windows.len() as f32;
        if consistency < ENROLL_CONSISTENCY {
            return fail("Det hördes flera röster eller mycket bakgrundsljud. Försök igen i ett tystare rum.");
        }
        log::info!("voice profile recorded: {} windows, consistency {consistency:.2}", windows.len());
        lock(&self.shared.lib).set_me(print, windows.len() as u32);
        self.last_me = Some(Instant::now());
        let lib = lock(&self.shared.lib).clone();
        match self.shared.store.save(&lib) {
            Ok(()) => EnrollResult { ok: true, message: "Din röstprofil är sparad.".into() },
            Err(e) => {
                log::error!("saving voice library: {e:#}");
                fail("Röstprofilen kunde inte sparas.")
            }
        }
    }

    fn housekeeping(&mut self) {
        {
            let mut lib = lock(&self.shared.lib);
            self.tally.flush_if_due(&mut lib, &today());
            // Prints, new voices and statistics all count as changes worth saving.
            if lib.changes != self.saved_changes {
                self.saved_changes = lib.changes;
                self.shared.dirty.store(true, Ordering::Relaxed);
            }
        }
        if self.last_expiry.elapsed() >= Duration::from_secs(30) {
            self.last_expiry = Instant::now();
            let mut session = lock(&self.shared.session);
            let before = session.len();
            session.retain(|s| s.last.elapsed() < SESSION_EXPIRY);
            if session.len() != before {
                drop(session);
                self.bump();
            }
        }
        if self.shared.dirty.load(Ordering::Relaxed) && self.last_save.elapsed() >= SAVE_EVERY {
            self.last_save = Instant::now();
            self.shared.dirty.store(false, Ordering::Relaxed);
            let lib = lock(&self.shared.lib).clone();
            if let Err(e) = self.shared.store.save(&lib) {
                log::error!("saving voice library: {e:#}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(20_369), "2025-10-08");
        assert_eq!(civil_date(11_016), "2000-02-29");
    }

    #[test]
    fn downsample_averages_triples() {
        assert_eq!(downsample_48k(&[0.0, 1.5, 3.0, 1.0, 1.0, 1.0]), vec![1.5, 1.0]);
    }

    #[test]
    fn tile_fills_one_window() {
        let t = tile(&[1.0, 2.0]);
        assert_eq!(t.len(), WINDOW_SAMPLES);
        assert_eq!(&t[..4], &[1.0, 2.0, 1.0, 2.0]);
    }
}

#[cfg(test)]
mod service_tests {
    use super::super::embedder::tests::read_fixture;
    use super::super::library::KeySource;
    use super::*;

    struct FixedKey;
    impl KeySource for FixedKey {
        fn key(&self) -> Result<[u8; 32]> {
            Ok([3u8; 32])
        }
    }

    fn feed(link: &Link, audio: &[f32]) {
        for chunk in audio.chunks(160) {
            // Blocking send: the test wants every hop processed, unlike the engine.
            link.tx.send(Hop { samples: chunk.to_vec(), speech: true }).unwrap();
        }
    }

    fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
        let start = Instant::now();
        while !ok() {
            assert!(start.elapsed() < Duration::from_secs(60), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn enroll_then_mute_a_stranger_and_pass_the_user() {
        let dir = std::env::temp_dir().join(format!("hush-service-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let params = Arc::new(Params::new(true, Mode::Noise, 72.0, -42.0));
        let speakers = Speakers::start(Store::new(&dir, Box::new(FixedKey)), params);
        let link = speakers.link();
        let (david_a, david_b, zira) = (read_fixture("david_a.wav"), read_fixture("david_b.wav"), read_fixture("zira_a.wav"));

        speakers.start_enrollment();
        let user: Vec<f32> = david_a.iter().chain(&david_b).copied().collect();
        while speakers.view().enrollment.is_some() {
            feed(&link, &user);
        }
        wait_for("enrollment result", || speakers.view().enrolled);
        assert!(speakers.take_enroll_result().unwrap().ok);

        feed(&link, &zira);
        wait_for("stranger to be learned", || speakers.view().voices.len() == 1);
        let stranger = speakers.view().voices[0].clone();
        assert!(stranger.recent, "a voice just learned counts as heard now");
        assert!(!stranger.heard, "unknown policy is mute, so the new voice starts muted");
        assert!(!link.allow(), "stranger must not pass");

        // One switch: letting the voice through applies at once and is saved.
        speakers.set_default(&stranger.id, Policy::Pass).unwrap();
        feed(&link, &zira);
        wait_for("stranger to pass once heard is on", || link.allow());
        assert!(Store::new(&dir, Box::new(FixedKey)).load().unwrap().voices[0].default == Policy::Pass);

        feed(&link, &david_b);
        wait_for("user to pass again", || link.allow());
        assert_eq!(speakers.speaking().as_deref(), Some("me"));

        // The library survives a restart, encrypted on disk.
        let reloaded = Store::new(&dir, Box::new(FixedKey)).load().unwrap();
        assert!(reloaded.me.is_some());
        let _ = std::fs::remove_dir_all(dir);
    }
}
