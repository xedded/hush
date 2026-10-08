//! Decides who is speaking from a stream of embeddings, adds voices it has not
//! heard before, and decides whether the current speaker may pass.

use super::embedder::{cosine, mean};
use super::library::{Library, Policy};

/// Score needed to call a voice by name.
const MATCH: f32 = 0.5;
/// Lower bar for the user's own voice, which was enrolled through the same microphone.
const ME_MATCH: f32 = 0.4;
/// The user's voice also wins when another voice scores at most this much higher.
const ME_BIAS: f32 = 0.05;
/// Score above which a match is used to refine the stored print.
const CONFIDENT: f32 = 0.65;
/// Below this against everyone, the speech may be a new voice.
const NEW_VOICE_BELOW: f32 = 0.35;
/// Unmatched embeddings (about 0.5 s each) needed before a new voice is created.
const NEW_VOICE_EMBEDDINGS: usize = 4;
/// Every pair of unmatched embeddings must agree this well. (Checking against
/// their mean is too weak: two different voices both sit 0.7+ from their mean.)
const NEW_VOICE_CONSISTENCY: f32 = 0.5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Who {
    Me,
    Voice(String),
    /// Speech that is not (yet) recognised.
    Unknown,
}

#[derive(Debug, PartialEq)]
pub struct Observation {
    pub who: Who,
    pub score: f32,
    /// Id of a voice created by this observation.
    pub created: Option<String>,
}

#[derive(Default)]
pub struct Tracker {
    pending: Vec<Vec<f32>>,
}

impl Tracker {
    /// A new turn started after silence: unmatched speech from before belongs to someone else.
    pub fn new_turn(&mut self) {
        self.pending.clear();
    }

    pub fn observe(&mut self, lib: &mut Library, emb: &[f32], today: &str) -> Observation {
        let me_score = lib.me.as_ref().map_or(f32::MIN, |p| cosine(p, emb));
        let best_voice = lib
            .voices
            .iter()
            .map(|v| (v.id.clone(), cosine(&v.print, emb)))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        let voice_score = best_voice.as_ref().map_or(f32::MIN, |b| b.1);

        // Muting the user by mistake is far worse than letting someone through,
        // so the user's own voice wins at a lower score and on near-ties.
        if me_score >= ME_MATCH && me_score + ME_BIAS >= voice_score {
            self.pending.clear();
            return Observation { who: Who::Me, score: me_score, created: None };
        }
        if let Some((id, score)) = best_voice.filter(|b| b.1 >= MATCH) {
            self.pending.clear();
            if score >= CONFIDENT {
                lib.reinforce(&id, emb, today);
            } else {
                lib.touch(&id, today);
            }
            return Observation { who: Who::Voice(id), score, created: None };
        }

        let score = me_score.max(voice_score).max(0.0);
        if score >= NEW_VOICE_BELOW {
            // Ambiguous: close to someone but not close enough. Do not learn from it.
            return Observation { who: Who::Unknown, score, created: None };
        }

        self.pending.push(emb.to_vec());
        if self.pending.len() >= NEW_VOICE_EMBEDDINGS {
            let consistent = self
                .pending
                .iter()
                .enumerate()
                .all(|(i, a)| self.pending[i + 1..].iter().all(|b| cosine(a, b) >= NEW_VOICE_CONSISTENCY));
            if consistent {
                let centre = mean(&self.pending).expect("pending is not empty");
                let id = lib.add(centre, self.pending.len() as u32, today);
                self.pending.clear();
                return Observation { who: Who::Voice(id.clone()), score: 1.0, created: Some(id) };
            }
            // Mixed speakers in the buffer: keep only the most recent ones.
            self.pending.remove(0);
        }
        Observation { who: Who::Unknown, score, created: None }
    }
}

/// Whether the current speaker passes.
///
/// * The user's own voice always passes.
/// * "Bara min röst" (`only_me`) mutes every other recognised voice.
/// * Otherwise a recognised voice follows its switch for this meeting.
/// * Speech not recognised yet follows the unknown-voice policy. `benefit_of_doubt`
///   lets it through while a turn is still unidentified and the user spoke recently,
///   so the start of the user's own sentences is not cut.
pub fn allowed(who: &Who, only_me: bool, voice_on: impl Fn(&str) -> bool, unknown: Policy, benefit_of_doubt: bool) -> bool {
    match who {
        Who::Me => true,
        Who::Voice(id) => !only_me && voice_on(id),
        Who::Unknown => benefit_of_doubt || unknown == Policy::Pass,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODAY: &str = "2026-10-08";

    /// Unit vector along `axis` with a small, deterministic wobble.
    fn voice(axis: usize, wobble: f32) -> Vec<f32> {
        let mut v = vec![0.0f32; 8];
        v[axis] = 1.0;
        v[(axis + 1) % 8] = wobble;
        super::super::embedder::normalize(&mut v);
        v
    }

    #[test]
    fn enrolled_user_is_recognised() {
        let mut lib = Library { me: Some(voice(0, 0.0)), ..Library::default() };
        let mut t = Tracker::default();
        let o = t.observe(&mut lib, &voice(0, 0.2), TODAY);
        assert_eq!(o.who, Who::Me);
        assert!(lib.voices.is_empty());
    }

    #[test]
    fn a_new_voice_is_created_after_consistent_speech_and_then_recognised() {
        let mut lib = Library { me: Some(voice(0, 0.0)), ..Library::default() };
        let mut t = Tracker::default();
        for i in 0..NEW_VOICE_EMBEDDINGS - 1 {
            let o = t.observe(&mut lib, &voice(3, 0.1 * i as f32), TODAY);
            assert_eq!(o.who, Who::Unknown);
        }
        let o = t.observe(&mut lib, &voice(3, 0.05), TODAY);
        let id = o.created.expect("voice created");
        assert_eq!(o.who, Who::Voice(id.clone()));
        assert_eq!(lib.voices.len(), 1);
        assert_eq!(t.observe(&mut lib, &voice(3, 0.15), TODAY).who, Who::Voice(id));
    }

    #[test]
    fn alternating_strangers_do_not_create_a_merged_voice() {
        let mut lib = Library::default();
        let mut t = Tracker::default();
        for i in 0..10 {
            t.observe(&mut lib, &voice(if i % 2 == 0 { 2 } else { 5 }, 0.0), TODAY);
        }
        assert!(lib.voices.is_empty());
    }

    #[test]
    fn new_turn_discards_unmatched_speech() {
        let mut lib = Library::default();
        let mut t = Tracker::default();
        for _ in 0..NEW_VOICE_EMBEDDINGS - 1 {
            t.observe(&mut lib, &voice(4, 0.0), TODAY);
        }
        t.new_turn();
        t.observe(&mut lib, &voice(4, 0.0), TODAY);
        assert!(lib.voices.is_empty());
    }

    #[test]
    fn deleted_voice_is_treated_as_new_again() {
        let mut lib = Library::default();
        let mut t = Tracker::default();
        let mut created = None;
        for _ in 0..NEW_VOICE_EMBEDDINGS {
            created = t.observe(&mut lib, &voice(6, 0.0), TODAY).created.or(created);
        }
        assert!(lib.delete(&created.unwrap()));
        assert_eq!(t.observe(&mut lib, &voice(6, 0.0), TODAY).who, Who::Unknown);
    }

    #[test]
    fn policy_rules() {
        let on = |_: &str| true;
        let off = |_: &str| false;
        let v = Who::Voice("v2".into());
        assert!(allowed(&Who::Me, true, off, Policy::Mute, false));
        assert!(allowed(&v, false, on, Policy::Mute, false));
        assert!(!allowed(&v, false, off, Policy::Pass, false));
        assert!(!allowed(&v, true, on, Policy::Pass, false), "only-me mutes everyone else");
        assert!(!allowed(&Who::Unknown, false, on, Policy::Mute, false));
        assert!(allowed(&Who::Unknown, false, on, Policy::Mute, true), "user's own sentence start passes");
        assert!(allowed(&Who::Unknown, true, on, Policy::Pass, false));
    }
}

#[cfg(test)]
mod recording_tests {
    use super::super::embedder::tests::read_fixture;
    use super::super::embedder::{mean, Embedder};
    use super::*;

    #[test]
    fn recognises_the_user_and_learns_a_stranger_from_recordings() {
        let e = Embedder::new().unwrap();
        let windows = |name: &str| e.embed_all(&read_fixture(name), 8_000).unwrap();
        let mut lib = Library { me: mean(&windows("david_a.wav")), ..Library::default() };
        let mut t = Tracker::default();

        let created: Vec<String> =
            windows("zira_a.wav").iter().filter_map(|w| t.observe(&mut lib, w, "2026-10-08").created).collect();
        assert_eq!(created.len(), 1, "exactly one new voice for the stranger");

        t.new_turn();
        let user: Vec<Who> = windows("david_b.wav").iter().map(|w| t.observe(&mut lib, w, "2026-10-08").who).collect();
        assert!(user.iter().all(|w| *w == Who::Me), "user misidentified: {user:?}");

        t.new_turn();
        let again: Vec<Who> = windows("zira_a.wav").iter().map(|w| t.observe(&mut lib, w, "2026-10-08").who).collect();
        // The recording ends in silence, which the app never feeds to the tracker:
        // allow an unrecognised tail, but never a wrong identity.
        let stranger = Who::Voice(created[0].clone());
        assert!(again.iter().all(|w| *w == stranger || *w == Who::Unknown), "confused: {again:?}");
        assert!(again.iter().filter(|w| **w == stranger).count() >= again.len() - 1, "not recognised: {again:?}");
    }
}
