//! Pure, transient duration evidence from explicitly recorded routine execution.
use std::collections::BTreeMap;

use crate::api::planning::DurationEstimateBody;

/// Fewer than five completed observations cannot support a trustworthy spread.
pub const MINIMUM_OBSERVATIONS: usize = 5;
/// Older observations no longer describe how the routine goes now.
pub const OBSERVATION_WINDOW_SECONDS: i64 = 90 * 24 * 60 * 60;
/// Exclude forgotten timers without trimming away a fixed fraction of the genuine tail.
pub const OUTLIER_MULTIPLE: u64 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observation {
    pub started_at: i64,
    pub completed_at: i64,
}
impl Observation {
    pub fn seconds(&self) -> i64 {
        self.completed_at.saturating_sub(self.started_at)
    }
    fn usable(&self) -> bool {
        self.completed_at > self.started_at
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Untrusted {
    TooFewObservations(usize),
}

#[derive(Debug, PartialEq, Eq)]
pub struct DerivedDuration {
    pub estimate: DurationEstimateBody,
    pub observations: usize,
    pub min_seconds: u64,
    pub mode_seconds: u64,
    pub p95_seconds: u64,
}

/// Nearest-rank percentile of a nonempty, sorted list of positive durations.
pub fn percentile(sorted: &[i64], fraction: f64) -> u64 {
    assert!(!sorted.is_empty());
    let rank = (sorted.len() as f64 * fraction).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)] as u64
}

pub fn derive(observations: &[Observation]) -> Result<DerivedDuration, Untrusted> {
    let mut seconds: Vec<_> = observations
        .iter()
        .filter(|o| o.usable())
        .map(Observation::seconds)
        .collect();
    if seconds.len() < MINIMUM_OBSERVATIONS {
        return Err(Untrusted::TooFewObservations(seconds.len()));
    }
    seconds.sort_unstable();
    let ceiling = percentile(&seconds, 0.5).saturating_mul(OUTLIER_MULTIPLE);
    seconds.retain(|&seconds| seconds as u64 <= ceiling);
    if seconds.len() < MINIMUM_OBSERVATIONS {
        return Err(Untrusted::TooFewObservations(seconds.len()));
    }
    let min_seconds = seconds[0] as u64;
    let mut mode_seconds = percentile(&seconds, 0.5);
    let mut p95_seconds = percentile(&seconds, 0.95);
    let estimate = if min_seconds == p95_seconds {
        DurationEstimateBody::Fixed {
            seconds: min_seconds,
        }
    } else {
        // Tied order statistics need one-second nudges for the schema's strict ordering.
        mode_seconds = mode_seconds.max(min_seconds + 1);
        p95_seconds = p95_seconds.max(mode_seconds + 1);
        DurationEstimateBody::ShiftedLognormalP95 {
            min_seconds,
            mode_seconds,
            p95_seconds,
        }
    };
    Ok(DerivedDuration {
        estimate,
        observations: seconds.len(),
        min_seconds,
        mode_seconds,
        p95_seconds,
    })
}

/// Starts replace an open start; only a following completion closes an observation.
pub fn pair(mut events: Vec<(i64, bool)>) -> Vec<Observation> {
    events.sort_unstable_by_key(|&(time, start)| (time, std::cmp::Reverse(start)));
    let mut open = None;
    let mut observations = Vec::new();
    for (time, start) in events {
        if start {
            open = Some(time);
        } else if let Some(started_at) = open.take() {
            observations.push(Observation {
                started_at,
                completed_at: time,
            });
        }
    }
    observations
}

/// Both ends must lie in the inclusive recent window; future events are excluded.
pub fn by_group(rows: Vec<(String, i64, bool)>, now: i64) -> BTreeMap<String, Vec<Observation>> {
    let mut groups = BTreeMap::<String, Vec<(i64, bool)>>::new();
    let since = now.saturating_sub(OBSERVATION_WINDOW_SECONDS);
    for (group, time, start) in rows {
        if time >= since && time <= now {
            groups.entry(group).or_default().push((time, start));
        }
    }
    groups
        .into_iter()
        .map(|(group, events)| (group, pair(events)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn observations(seconds: &[i64]) -> Vec<Observation> {
        seconds
            .iter()
            .map(|&completed_at| Observation {
                started_at: 0,
                completed_at,
            })
            .collect()
    }
    #[test]
    fn too_few_including_unusable_intervals() {
        for values in [
            vec![],
            vec![1, 2, 3, 4],
            vec![1, 2, 3, 4, 0, -1],
            vec![60, 60, 60, 60, 6000],
        ] {
            let expected = if values.is_empty() { 0 } else { 4 };
            assert_eq!(
                derive(&observations(&values)),
                Err(Untrusted::TooFewObservations(expected))
            );
        }
    }
    #[test]
    fn spread_uses_nearest_rank_and_strict_schema_ordering() {
        let model = derive(&observations(&[900, 960, 1020, 1200, 1500, 1800])).unwrap();
        assert_eq!(model.observations, 6);
        assert_eq!(
            model.estimate,
            DurationEstimateBody::ShiftedLognormalP95 {
                min_seconds: 900,
                mode_seconds: 1020,
                p95_seconds: 1800
            }
        );
        assert_eq!(percentile(&[1, 2, 3, 4, 5, 6], 0.5), 3);
        assert_eq!(percentile(&(1..=20).collect::<Vec<_>>(), 0.95), 19);
        for values in [[1, 1, 1, 1, 2], [1, 2, 2, 2, 2]] {
            let model = derive(&observations(&values)).unwrap();
            assert!(
                model.min_seconds < model.mode_seconds && model.mode_seconds < model.p95_seconds
            );
        }
    }
    #[test]
    fn forgotten_timer_is_removed_but_genuine_long_day_remains() {
        let model = derive(&observations(&[600, 660, 720, 780, 840, 4800, 86400])).unwrap();
        assert_eq!(model.observations, 6);
        assert_eq!(
            (model.min_seconds, model.mode_seconds, model.p95_seconds),
            (600, 720, 4800)
        );
        // Exactly eight times the pre-filter median is retained.
        assert_eq!(
            derive(&observations(&[600, 600, 600, 600, 600, 4800]))
                .unwrap()
                .p95_seconds,
            4800
        );
    }
    #[test]
    fn constant_work_stays_fixed() {
        let model = derive(&observations(&[600; 6])).unwrap();
        assert_eq!(model.estimate, DurationEstimateBody::Fixed { seconds: 600 });
        assert_eq!(
            (model.min_seconds, model.mode_seconds, model.p95_seconds),
            (600, 600, 600)
        );
    }
    #[test]
    fn pairing_orders_ties_replaces_starts_and_skips_unmatched_events() {
        assert_eq!(
            pair(vec![
                (40, true),
                (30, false),
                (20, true),
                (10, true),
                (0, false),
                (31, false)
            ]),
            vec![Observation {
                started_at: 20,
                completed_at: 30
            }]
        );
        let tied = pair(vec![(10, false), (10, true)]);
        assert_eq!(
            tied,
            vec![Observation {
                started_at: 10,
                completed_at: 10
            }]
        );
        assert!(!tied[0].usable());
    }
    #[test]
    fn grouping_excludes_old_and_future_events() {
        let now = OBSERVATION_WINDOW_SECONDS + 100;
        let rows = vec![
            ("b", 100, true),
            ("b", 160, false),
            ("a", 99, true),
            ("a", 170, false),
            ("b", now - 60, true),
            ("b", now, false),
            ("c", now + 1, true),
            ("c", now + 61, false),
        ];
        let rows: Vec<_> = rows
            .into_iter()
            .map(|(g, t, s)| (g.to_owned(), t, s))
            .collect();
        let result = by_group(rows.clone(), now);
        let mut reversed = rows;
        reversed.reverse();
        assert_eq!(result, by_group(reversed, now));
        assert_eq!(
            result.keys().map(String::as_str).collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert!(result["a"].is_empty());
        assert_eq!(
            result["b"],
            vec![
                Observation {
                    started_at: 100,
                    completed_at: 160
                },
                Observation {
                    started_at: now - 60,
                    completed_at: now
                }
            ]
        );
    }
}
