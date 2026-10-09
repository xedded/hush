//! The statistics page: talk time, share and loudness per voice, from the
//! counters kept in the voice library.

use super::library::{Library, Stats};
use super::service::recent_days;

/// Days shown in each voice's activity chart.
const CHART_DAYS: usize = 14;

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatRow {
    /// "me" or a voice id.
    pub id: String,
    pub name: String,
    pub me: bool,
    pub seconds: f64,
    /// Share of all recognised talk time, 0..=1.
    pub share: f64,
    pub mean_db: Option<f32>,
    pub peak_db: Option<f32>,
    pub turns: u32,
    /// Average length of a turn, in seconds.
    pub turn_seconds: Option<f64>,
    pub days_heard: usize,
    pub first_heard: Option<String>,
    pub last_heard: Option<String>,
    /// Seconds per day for the last CHART_DAYS days, oldest first.
    pub chart: Vec<f32>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsView {
    pub enrolled: bool,
    pub total_seconds: f64,
    /// Earliest day any voice was counted.
    pub since: Option<String>,
    /// Dates of the chart columns, oldest first.
    pub days: Vec<String>,
    /// Voices with any counted speech, most talk time first.
    pub rows: Vec<StatRow>,
}

fn row(id: &str, name: &str, me: bool, s: &Stats, total: f64, days: &[String]) -> StatRow {
    StatRow {
        id: id.to_string(),
        name: name.to_string(),
        me,
        seconds: s.seconds,
        share: if total > 0.0 { s.seconds / total } else { 0.0 },
        mean_db: s.mean_db(),
        peak_db: s.peak_db,
        turns: s.turns,
        turn_seconds: (s.turns > 0).then(|| s.seconds / s.turns as f64),
        days_heard: s.days.len(),
        first_heard: s.first_heard.clone(),
        last_heard: s.days.keys().next_back().cloned(),
        chart: days.iter().map(|d| s.days.get(d).copied().unwrap_or(0.0)).collect(),
    }
}

pub fn view(lib: &Library) -> StatsView {
    let days = recent_days(CHART_DAYS);
    let total = lib.me_stats.seconds + lib.voices.iter().map(|v| v.stats.seconds).sum::<f64>();
    let mut rows: Vec<StatRow> = std::iter::once(row("me", "Du", true, &lib.me_stats, total, &days))
        .chain(lib.voices.iter().map(|v| row(&v.id, &v.name, false, &v.stats, total, &days)))
        .filter(|r| r.seconds > 0.0)
        .collect();
    rows.sort_by(|a, b| b.seconds.total_cmp(&a.seconds));
    let since = rows.iter().filter_map(|r| r.first_heard.clone()).min();
    StatsView { enrolled: lib.me.is_some(), total_seconds: total, since, days, rows }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_ranked_by_talk_time_with_shares() {
        let mut lib = Library::default();
        let today = recent_days(1).pop().unwrap();
        let quiet = lib.add(vec![1.0], 1, &today);
        let talker = lib.add(vec![1.0], 1, &today);
        lib.me_stats.add(60.0, 60.0 * 0.001, Some(-20.0), &today);
        lib.stats_mut(&talker).unwrap().add(180.0, 180.0 * 0.01, Some(-10.0), &today);
        lib.stats_mut(&talker).unwrap().turns = 6;
        let v = view(&lib);
        assert_eq!(v.rows.len(), 2, "voices never heard are left out: {quiet}");
        assert_eq!(v.rows[0].id, talker);
        assert!((v.rows[0].share - 0.75).abs() < 1e-9);
        assert_eq!(v.rows[0].turn_seconds, Some(30.0));
        assert!((v.rows[0].mean_db.unwrap() + 20.0).abs() < 0.01);
        assert_eq!(v.rows[1].name, "Du");
        assert_eq!(v.rows[1].chart.last().copied(), Some(60.0));
        assert_eq!(v.days.len(), CHART_DAYS);
        assert_eq!(v.since.as_deref(), Some(today.as_str()));
    }

    #[test]
    fn old_days_fall_out_of_the_counters() {
        let mut s = Stats::default();
        for d in 1..=120 {
            s.add(1.0, 0.0, None, &format!("2026-{:02}-{:02}", 1 + (d - 1) / 28, 1 + (d - 1) % 28));
        }
        assert_eq!(s.days.len(), 90);
        assert_eq!(s.seconds, 120.0, "totals keep everything");
    }
}
