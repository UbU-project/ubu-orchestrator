//! Pure, deterministic recurrence expansion. Local ambiguity chooses the earlier instant;
//! a gap suppresses that date. Materialization decides which expanded dates to keep.
use crate::api::planning::DiagnosticBody;
use chrono::{Datelike, LocalResult, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
};
use ubu_core::{
    core::{
        routine_occurrence_key, RecurrenceRule, RecurrenceSchedule, RoutineInstanceTemplate,
        Weekday,
    },
    UbuId,
};

#[derive(Debug, Clone)]
pub struct RoutineDefinition {
    pub objective_id: UbuId,
    pub schedule: RecurrenceSchedule,
    pub template: RoutineInstanceTemplate,
}
#[derive(Debug, Clone)]
pub struct Occurrence {
    pub objective_id: UbuId,
    pub local_date: String,
    pub key: String,
    pub template: RoutineInstanceTemplate,
    /// Static interval or planned allowed range, in UTC seconds.
    pub start: u64,
    pub end: u64,
    pub nominal_end: u64,
    pub after: Vec<(UbuId, i64)>,
}
#[derive(Debug, Default)]
pub struct Instantiation {
    pub occurrences: Vec<Occurrence>,
    pub diagnostics: Vec<DiagnosticBody>,
    pub dates: BTreeMap<String, (String, String)>,
}
pub(crate) fn diagnostic(code: &str, message: impl Into<String>) -> DiagnosticBody {
    DiagnosticBody {
        code: code.into(),
        message: message.into(),
    }
}
fn matches(schedule: &RecurrenceSchedule, date: NaiveDate) -> bool {
    let d = date.to_string();
    if schedule.enabled_from.as_ref().is_some_and(|s| &d < s)
        || schedule.enabled_until.as_ref().is_some_and(|s| &d > s)
        || schedule.exdates.contains(&d)
    {
        return false;
    }
    let weekday = match date.weekday() {
        chrono::Weekday::Mon => Weekday::Mon,
        chrono::Weekday::Tue => Weekday::Tue,
        chrono::Weekday::Wed => Weekday::Wed,
        chrono::Weekday::Thu => Weekday::Thu,
        chrono::Weekday::Fri => Weekday::Fri,
        chrono::Weekday::Sat => Weekday::Sat,
        chrono::Weekday::Sun => Weekday::Sun,
    };
    let first_workday = || {
        let mut first = date.with_day(1).unwrap();
        while first.weekday().number_from_monday() > 5 {
            first = first.succ_opt().unwrap();
        }
        date == first
    };
    match &schedule.rule {
        RecurrenceRule::Daily => true,
        RecurrenceRule::Weekly { weekdays } => weekdays.contains(&weekday),
        RecurrenceRule::MonthlyDay { days } => days.contains(&(date.day() as u8)),
        RecurrenceRule::FirstWorkdayOfMonth => first_workday(),
        RecurrenceRule::FirstWorkdayOfQuarter => {
            [1, 4, 7, 10].contains(&date.month()) && first_workday()
        }
    }
}
fn local(
    tz: Tz,
    date: NaiveDate,
    time: &str,
    id: &UbuId,
    report: bool,
    diagnostics: &mut Vec<DiagnosticBody>,
) -> Option<u64> {
    let time = NaiveTime::parse_from_str(time, "%H:%M:%S").ok()?;
    let instant = match tz.from_local_datetime(&date.and_time(time)) {
        LocalResult::Single(t) => t,
        LocalResult::Ambiguous(a, b) => {
            if report {
                diagnostics.push(diagnostic(
                    "routine_occurrence_ambiguous_local_time",
                    format!("Routine `{id}` on {date} uses the earlier instant for {time} in {tz}"),
                ));
            }
            a.min(b)
        }
        LocalResult::None => {
            if report {
                diagnostics.push(diagnostic(
                    "routine_occurrence_nonexistent_local_time",
                    format!("Routine `{id}` on {date} has nonexistent local time {time} in {tz}"),
                ));
            }
            return None;
        }
    };
    u64::try_from(instant.timestamp()).ok()
}
pub fn instantiate(defs: &[RoutineDefinition], start: u64, end: u64) -> Instantiation {
    let mut out = Instantiation::default();
    let (Some(start), Some(end)) = (
        i64::try_from(start)
            .ok()
            .and_then(|s| chrono::DateTime::<Utc>::from_timestamp(s, 0)),
        i64::try_from(end)
            .ok()
            .and_then(|s| chrono::DateTime::<Utc>::from_timestamp(s, 0)),
    ) else {
        return out;
    };
    let definitions: BTreeMap<_, _> = defs.iter().map(|d| (d.objective_id.clone(), d)).collect();
    let mut remaining: BTreeMap<_, BTreeSet<_>> = definitions
        .iter()
        .map(|(id, d)| {
            (
                id.clone(),
                d.template
                    .after
                    .iter()
                    .filter(|a| definitions.contains_key(&a.objective_id))
                    .map(|a| a.objective_id.clone())
                    .collect(),
            )
        })
        .collect();
    let mut order = Vec::new();
    loop {
        let next = remaining
            .iter()
            .find(|(_, parents)| parents.is_empty())
            .map(|(id, _)| id.clone());
        let Some(next) = next else {
            break;
        };
        remaining.remove(&next);
        for parents in remaining.values_mut() {
            parents.remove(&next);
        }
        order.push(next);
    }
    let cyclic: BTreeSet<_> = remaining.keys().cloned().collect();
    if !cyclic.is_empty() {
        out.diagnostics.push(diagnostic(
            "routine_after_cycle",
            format!(
                "Routine after cycle or downstream dependency: {}",
                cyclic
                    .iter()
                    .map(|id| format!("`{id}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }
    order.extend(cyclic.iter().cloned());
    let mut nominal_ends = BTreeMap::new();
    for id in order {
        let d = definitions[&id];
        let Ok(tz) = Tz::from_str(&d.schedule.timezone) else {
            out.diagnostics.push(diagnostic(
                "routine_timezone_unknown",
                format!(
                    "Routine `{id}` has unknown timezone `{}`",
                    d.schedule.timezone
                ),
            ));
            continue;
        };
        let first = start.with_timezone(&tz).date_naive();
        let last = end.with_timezone(&tz).date_naive();
        let from = first.pred_opt().unwrap_or(first);
        let until = last.succ_opt().unwrap_or(last);
        out.dates
            .insert(id.to_string(), (from.to_string(), until.to_string()));
        let mut date = from;
        loop {
            if matches(&d.schedule, date) {
                let report = date >= first && date <= last;
                let mut expand = || -> Option<Occurrence> {
                    let nominal = local(
                        tz,
                        date,
                        &d.template.nominal_start,
                        &id,
                        report,
                        &mut out.diagnostics,
                    )?;
                    let range = if let Some(range) = &d.template.allowed_local_range {
                        Some((
                            local(tz, date, &range.earliest, &id, report, &mut out.diagnostics)?,
                            local(tz, date, &range.latest, &id, report, &mut out.diagnostics)?,
                        ))
                    } else {
                        None
                    };
                    let mut after = BTreeMap::<UbuId, i64>::new();
                    let mut floor = 0;
                    if !cyclic.contains(&id) {
                        for reference in &d.template.after {
                            if let Some(&end) =
                                nominal_ends.get(&(reference.objective_id.clone(), date))
                            {
                                floor = floor
                                    .max(u64::saturating_add(end, reference.minimum_seconds as u64));
                                after
                                    .entry(reference.objective_id.clone())
                                    .and_modify(|n| *n = (*n).max(reference.minimum_seconds))
                                    .or_insert(reference.minimum_seconds);
                            } else if report {
                                out.diagnostics.push(diagnostic("routine_after_unmatched", format!("Routine `{id}` on {date} has no same-date predecessor `{}`",reference.objective_id)));
                            }
                        }
                    }
                    let duration = d.template.duration_estimate.scalar_seconds();
                    let (start, end, nominal_end) = match range {
                        Some((earliest, latest)) => {
                            let earliest = earliest.max(floor);
                            if earliest.saturating_add(duration) > latest {
                                if report {
                                    out.diagnostics.push(diagnostic("routine_after_infeasible", format!("Routine `{id}` on {date} cannot fit its lowered allowed range")));
                                }
                                return None;
                            }
                            (
                                earliest,
                                latest,
                                nominal.max(earliest).saturating_add(duration),
                            )
                        }
                        None => {
                            let start = nominal.max(floor);
                            (
                                start,
                                start.saturating_add(duration),
                                start.saturating_add(duration),
                            )
                        }
                    };
                    Some(Occurrence {
                        objective_id: id.clone(),
                        local_date: date.to_string(),
                        key: routine_occurrence_key(
                            &id,
                            d.schedule.schedule_version,
                            &date.to_string(),
                            &d.template.nominal_start,
                            d.template.placement,
                            d.template.template_version,
                        ),
                        template: d.template.clone(),
                        start,
                        end,
                        nominal_end,
                        after: after.into_iter().collect(),
                    })
                };
                if let Some(occurrence) = expand() {
                    nominal_ends.insert((id.clone(), date), occurrence.nominal_end);
                    out.occurrences.push(occurrence);
                }
            }
            if date == until {
                break;
            }
            let Some(next) = date.succ_opt() else {
                break;
            };
            date = next;
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlapSide {
    pub objective_id: UbuId,
    pub window: (String, String),
    pub local_date: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineOverlap {
    pub first: OverlapSide,
    pub second: OverlapSide,
    pub dates: usize,
    pub self_overlap: bool,
    pub dst_only: bool,
}
fn local_instant(seconds: u64, zone: Tz) -> Option<chrono::NaiveDateTime> {
    i64::try_from(seconds)
        .ok()
        .and_then(|s| chrono::DateTime::<Utc>::from_timestamp(s, 0))
        .map(|t| t.with_timezone(&zone).naive_local())
}
fn overlap_side(occurrence: &Occurrence, zone: Tz) -> OverlapSide {
    let clock = |seconds| {
        local_instant(seconds, zone)
            .map(|t| t.format("%H:%M:%S").to_string())
            .unwrap_or_else(|| "out-of-range".into())
    };
    OverlapSide {
        objective_id: occurrence.objective_id.clone(),
        window: (clock(occurrence.start), clock(occurrence.end)),
        local_date: occurrence.local_date.clone(),
    }
}
fn only_dst(first: &Occurrence, second: &Occurrence, first_zone: Tz, second_zone: Tz) -> bool {
    if first_zone != second_zone {
        return false;
    }
    // Use actual, already-lowered starts, with scalar duration on the local clock.
    let nominal = |o: &Occurrence, zone| {
        let start = local_instant(o.start, zone)?;
        let duration = i64::try_from(o.template.duration_estimate.scalar_seconds())
            .ok()
            .and_then(chrono::TimeDelta::try_seconds)?;
        Some((start, start.checked_add_signed(duration)?))
    };
    match (nominal(first, first_zone), nominal(second, second_zone)) {
        (Some((a, b)), Some((c, d))) => b <= c || d <= a,
        _ => false, // Unrepresentable local spans cannot earn the DST exemption.
    }
}

/// Check capacity Static occurrences starting in the span. Retain one date per
/// unordered Objective pair rather than a potentially year-sized date set.
pub fn static_overlaps(
    defs: &[RoutineDefinition],
    start: u64,
    end: u64,
) -> (Vec<RoutineOverlap>, Vec<DiagnosticBody>) {
    let expanded = instantiate(defs, start, end);
    let zones: BTreeMap<_, _> = defs
        .iter()
        .filter_map(|d| {
            d.schedule
                .timezone
                .parse::<Tz>()
                .ok()
                .map(|zone| (d.objective_id.clone(), zone))
        })
        .collect();
    let mut occurrences: Vec<_> = expanded
        .occurrences
        .iter()
        .filter(|o| {
            o.start >= start
                && o.start < end
                && o.template.placement == ubu_core::core::RoutinePlacement::Static
                && o.template.occupies_capacity
        })
        .collect();
    occurrences.sort_by(|a, b| (a.start, a.end, &a.key).cmp(&(b.start, b.end, &b.key)));
    let mut open: Vec<&Occurrence> = Vec::new();
    let mut pairs = BTreeMap::<(UbuId, UbuId), (RoutineOverlap, String)>::new();
    for current in occurrences {
        open.retain(|other| other.end > current.start);
        for &earlier in &open {
            let key = if earlier.objective_id <= current.objective_id {
                (earlier.objective_id.clone(), current.objective_id.clone())
            } else {
                (current.objective_id.clone(), earlier.objective_id.clone())
            };
            let (pair, last_date) = pairs.entry(key).or_insert_with(|| {
                (
                    RoutineOverlap {
                        first: overlap_side(earlier, zones[&earlier.objective_id]),
                        second: overlap_side(current, zones[&current.objective_id]),
                        dates: 0,
                        self_overlap: earlier.objective_id == current.objective_id,
                        dst_only: true,
                    },
                    String::new(),
                )
            });
            // Long multi-day occurrences can revisit older open instances; never
            // count their dates again after a newer date has already been seen.
            if earlier.local_date > *last_date {
                pair.dates += 1;
                last_date.clone_from(&earlier.local_date);
            }
            if pair.dst_only {
                pair.dst_only = only_dst(
                    earlier,
                    current,
                    zones[&earlier.objective_id],
                    zones[&current.objective_id],
                );
            }
        }
        open.push(current);
    }
    let mut overlaps: Vec<_> = pairs.into_values().map(|(pair, _)| pair).collect();
    overlaps.sort_by(|a, b| {
        (
            &a.first.local_date,
            &a.first.objective_id,
            &a.second.objective_id,
        )
            .cmp(&(
                &b.first.local_date,
                &b.first.objective_id,
                &b.second.objective_id,
            ))
    });
    (overlaps, expanded.diagnostics)
}
