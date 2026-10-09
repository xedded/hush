//! The voice library: the user's own voiceprint plus every voice Hush has
//! heard. Voices are never removed automatically, only when the user deletes
//! them. Stored encrypted (AES-256-GCM) with the key in the OS keychain, so
//! the file is useless if copied off the machine.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{anyhow, Context, Result};
use base64::Engine as _;

use super::embedder;

const KEYCHAIN_SERVICE: &str = "se.segerstad.hush";
const KEYCHAIN_ACCOUNT: &str = "voice-library-key";
const NONCE_LEN: usize = 12;
pub const MAX_NAME_LEN: usize = 40;
/// Running-mean weight cap: older speech keeps counting, new speech still adapts the print.
const MAX_PRINT_WEIGHT: u32 = 60;
/// The user's own print adapts far more slowly: muting the user by mistake is
/// the worst failure, so one odd day must not move it much.
const MAX_ME_WEIGHT: u32 = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Policy {
    Pass,
    Mute,
}

/// Days of talk time kept per voice for the statistics page.
const STATS_DAYS: usize = 90;

/// How much and how loudly a voice has been heard. Counted only while the
/// voice is recognised, so speech before identification is not included.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Stats {
    /// Seconds of recognised speech.
    pub seconds: f64,
    /// Sum of mean-square level times seconds, for the average level.
    pub energy: f64,
    /// Loudest 200 ms, in dBFS.
    pub peak_db: Option<f32>,
    /// Times the voice started speaking (turns).
    pub turns: u32,
    /// Seconds per local date (YYYY-MM-DD), the last STATS_DAYS days with speech.
    pub days: BTreeMap<String, f32>,
    pub first_heard: Option<String>,
}

impl Stats {
    pub fn add(&mut self, seconds: f64, energy: f64, peak_db: Option<f32>, today: &str) {
        self.seconds += seconds;
        self.energy += energy;
        if let Some(p) = peak_db {
            self.peak_db = Some(self.peak_db.map_or(p, |q| q.max(p)));
        }
        *self.days.entry(today.to_string()).or_default() += seconds as f32;
        while self.days.len() > STATS_DAYS {
            let oldest = self.days.keys().next().cloned().expect("not empty");
            self.days.remove(&oldest);
        }
        self.first_heard.get_or_insert_with(|| today.to_string());
    }

    /// Average speech level in dBFS.
    pub fn mean_db(&self) -> Option<f32> {
        (self.seconds > 0.5).then(|| (10.0 * (self.energy / self.seconds).max(1e-12).log10()) as f32)
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Voice {
    pub id: String,
    pub name: String,
    /// False until the user gives the voice a name ("Röst 3").
    pub named: bool,
    pub default: Policy,
    pub print: Vec<f32>,
    /// How many embeddings the print averages, capped at MAX_PRINT_WEIGHT.
    pub weight: u32,
    /// Local date, YYYY-MM-DD.
    pub last_heard: String,
    #[serde(default)]
    pub stats: Stats,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Library {
    pub me: Option<Vec<f32>>,
    /// The print from the recording. Adaptation never strays far from it.
    pub me_anchor: Option<Vec<f32>>,
    /// How many embeddings the user's print averages.
    pub me_weight: u32,
    pub voices: Vec<Voice>,
    /// Counter for "Röst N" names; never reused, so a deleted number does not come back.
    pub next_number: u32,
    pub unknown: Option<Policy>,
    pub me_stats: Stats,
    /// Bumped on every change worth saving; not stored.
    #[serde(skip)]
    pub changes: u64,
}

impl Library {
    pub fn unknown_policy(&self) -> Policy {
        self.unknown.unwrap_or(Policy::Mute)
    }

    pub fn voice(&self, id: &str) -> Option<&Voice> {
        self.voices.iter().find(|v| v.id == id)
    }

    fn voice_mut(&mut self, id: &str) -> Option<&mut Voice> {
        self.voices.iter_mut().find(|v| v.id == id)
    }

    /// Add a newly heard voice and return its id.
    pub fn add(&mut self, print: Vec<f32>, weight: u32, today: &str) -> String {
        self.next_number = self.next_number.max(1) + 1;
        let number = self.next_number;
        let id = format!("v{number}-{}", today.replace('-', ""));
        self.voices.push(Voice {
            id: id.clone(),
            name: format!("Röst {number}"),
            named: false,
            default: self.unknown_policy(),
            print,
            weight: weight.min(MAX_PRINT_WEIGHT),
            last_heard: today.to_string(),
            stats: Stats::default(),
        });
        self.changes += 1;
        id
    }

    /// A freshly recorded profile replaces the old one, adaptation included.
    pub fn set_me(&mut self, print: Vec<f32>, weight: u32) {
        self.me_anchor = Some(print.clone());
        self.me = Some(print);
        self.me_weight = weight.clamp(1, MAX_ME_WEIGHT);
    }

    /// Fold a confident embedding of the user into their print, so the profile
    /// follows new headsets and rooms. Profiles from before adaptation existed
    /// take their current print as the anchor.
    pub fn reinforce_me(&mut self, embedding: &[f32]) {
        let Some(me) = self.me.as_mut() else { return };
        if self.me_anchor.is_none() {
            self.me_anchor = Some(me.clone());
            self.me_weight = MAX_ME_WEIGHT / 4;
        }
        let w = self.me_weight.max(1) as f32;
        for (p, e) in me.iter_mut().zip(embedding) {
            *p = (*p * w + e) / (w + 1.0);
        }
        embedder::normalize(me);
        self.me_weight = (self.me_weight + 1).min(MAX_ME_WEIGHT);
        self.changes += 1;
    }

    /// Fold a confident new embedding into a voice's print.
    pub fn reinforce(&mut self, id: &str, embedding: &[f32], today: &str) {
        let Some(v) = self.voice_mut(id) else { return };
        let w = v.weight.max(1) as f32;
        for (p, e) in v.print.iter_mut().zip(embedding) {
            *p = (*p * w + e) / (w + 1.0);
        }
        embedder::normalize(&mut v.print);
        v.weight = (v.weight + 1).min(MAX_PRINT_WEIGHT);
        v.last_heard = today.to_string();
        self.changes += 1;
    }

    pub fn touch(&mut self, id: &str, today: &str) {
        if let Some(v) = self.voice_mut(id) {
            v.last_heard = today.to_string();
        }
    }

    /// Stats for "me" or a voice id.
    pub fn stats_mut(&mut self, who: &str) -> Option<&mut Stats> {
        if who == "me" {
            return Some(&mut self.me_stats);
        }
        self.voice_mut(who).map(|v| &mut v.stats)
    }

    /// Start the statistics over, for everyone.
    pub fn reset_stats(&mut self) {
        self.me_stats = Stats::default();
        self.voices.iter_mut().for_each(|v| v.stats = Stats::default());
        self.changes += 1;
    }

    pub fn rename(&mut self, id: &str, name: &str) -> Result<()> {
        let name: String = name.trim().chars().take(MAX_NAME_LEN).collect();
        if name.is_empty() {
            return Err(anyhow!("Namnet får inte vara tomt."));
        }
        let v = self.voice_mut(id).ok_or_else(|| anyhow!("Rösten finns inte längre."))?;
        v.name = name;
        v.named = true;
        Ok(())
    }

    pub fn set_default(&mut self, id: &str, policy: Policy) -> Result<()> {
        self.voice_mut(id).ok_or_else(|| anyhow!("Rösten finns inte längre."))?.default = policy;
        Ok(())
    }

    pub fn delete(&mut self, id: &str) -> bool {
        let before = self.voices.len();
        self.voices.retain(|v| v.id != id);
        self.voices.len() != before
    }
}

/// Where the 256-bit file key lives. Tests use an in-memory key.
pub trait KeySource: Send + Sync {
    fn key(&self) -> Result<[u8; 32]>;
}

pub struct Keychain;

impl KeySource for Keychain {
    fn key(&self) -> Result<[u8; 32]> {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).context("keychain")?;
        let b64 = base64::engine::general_purpose::STANDARD;
        match entry.get_password() {
            Ok(text) => {
                let bytes = b64.decode(text.trim()).context("stored key is not base64")?;
                bytes.try_into().map_err(|_| anyhow!("stored key has the wrong length"))
            }
            Err(keyring::Error::NoEntry) => {
                let key = Aes256Gcm::generate_key(OsRng);
                entry.set_password(&b64.encode(key)).context("could not store key in keychain")?;
                Ok(key.into())
            }
            Err(e) => Err(anyhow!("keychain: {e}")),
        }
    }
}

pub struct Store {
    path: PathBuf,
    keys: Box<dyn KeySource>,
}

impl Store {
    pub fn new(dir: &Path, keys: Box<dyn KeySource>) -> Self {
        Self { path: dir.join("voices.bin"), keys }
    }

    pub fn load(&self) -> Result<Library> {
        let data = match std::fs::read(&self.path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Library::default()),
            Err(e) => return Err(e).context("could not read voice library"),
        };
        if data.len() <= NONCE_LEN {
            return Err(anyhow!("voice library file is truncated"));
        }
        let key = self.keys.key()?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        let (nonce, body) = data.split_at(NONCE_LEN);
        let plain = cipher
            .decrypt(Nonce::from_slice(nonce), body)
            .map_err(|_| anyhow!("voice library could not be decrypted"))?;
        serde_json::from_slice(&plain).context("voice library is corrupt")
    }

    pub fn save(&self, lib: &Library) -> Result<()> {
        let key = self.keys.key()?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let plain = serde_json::to_vec(lib)?;
        let body = cipher.encrypt(&nonce, plain.as_slice()).map_err(|_| anyhow!("encryption failed"))?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("bin.tmp");
        let mut out = nonce.to_vec();
        out.extend_from_slice(&body);
        std::fs::write(&tmp, out)?;
        std::fs::rename(&tmp, &self.path).context("could not save voice library")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedKey;
    impl KeySource for FixedKey {
        fn key(&self) -> Result<[u8; 32]> {
            Ok([7u8; 32])
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hush-voices-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn new_voices_get_increasing_numbers_that_are_never_reused() {
        let mut lib = Library::default();
        let a = lib.add(vec![1.0, 0.0], 3, "2026-10-08");
        let b = lib.add(vec![0.0, 1.0], 3, "2026-10-08");
        assert_eq!(lib.voice(&a).unwrap().name, "Röst 2");
        assert!(lib.delete(&b));
        let c = lib.add(vec![0.0, 1.0], 3, "2026-10-08");
        assert_eq!(lib.voice(&c).unwrap().name, "Röst 4");
        assert!(!lib.voice(&c).unwrap().named);
    }

    #[test]
    fn rename_validates_and_marks_named() {
        let mut lib = Library::default();
        let id = lib.add(vec![1.0], 1, "2026-10-08");
        assert!(lib.rename(&id, "   ").is_err());
        lib.rename(&id, &"x".repeat(80)).unwrap();
        assert_eq!(lib.voice(&id).unwrap().name.chars().count(), MAX_NAME_LEN);
        assert!(lib.voice(&id).unwrap().named);
        assert!(lib.rename("missing", "Sara").is_err());
    }

    #[test]
    fn new_voice_inherits_unknown_policy() {
        let mut lib = Library { unknown: Some(Policy::Pass), ..Library::default() };
        let id = lib.add(vec![1.0], 1, "2026-10-08");
        assert_eq!(lib.voice(&id).unwrap().default, Policy::Pass);
    }

    #[test]
    fn reinforce_moves_print_towards_new_speech() {
        let mut lib = Library::default();
        let id = lib.add(vec![1.0, 0.0], 1, "2026-10-01");
        lib.reinforce(&id, &[0.0, 1.0], "2026-10-08");
        let v = lib.voice(&id).unwrap();
        assert!(v.print[1] > 0.5 && v.print[0] > 0.5);
        assert_eq!(v.last_heard, "2026-10-08");
    }

    #[test]
    fn store_round_trips_encrypted() {
        let dir = temp_dir("roundtrip");
        let store = Store::new(&dir, Box::new(FixedKey));
        assert_eq!(store.load().unwrap(), Library::default());
        let mut lib = Library { me: Some(vec![0.6, 0.8]), ..Library::default() };
        lib.add(vec![1.0, 0.0], 2, "2026-10-08");
        store.save(&lib).unwrap();
        let raw = std::fs::read(dir.join("voices.bin")).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("Röst"), "file must not be plain text");
        // The change counter lives only in memory.
        assert_eq!(store.load().unwrap(), Library { changes: 0, ..lib });
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tampered_file_is_rejected() {
        let dir = temp_dir("tamper");
        let store = Store::new(&dir, Box::new(FixedKey));
        store.save(&Library::default()).unwrap();
        let path = dir.join("voices.bin");
        let mut raw = std::fs::read(&path).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 1;
        std::fs::write(&path, raw).unwrap();
        assert!(store.load().is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
