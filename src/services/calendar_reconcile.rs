//! Pure comparison of Calendar observations with UbU's recorded ownership.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::calendar_projection::DesiredEvent;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CalendarConflict {
    pub external_id: String,
    pub conflict_type: String,
    pub summary: String,
    pub message: String,
}

/// Only the applied record establishes ownership. A known Task id is evidence
/// for an unrecorded event, never permission to adopt it.
pub fn classify(
    applied: &[DesiredEvent],
    observed: &[DesiredEvent],
    known_external_ids: &BTreeSet<String>,
) -> Vec<CalendarConflict> {
    let applied: BTreeMap<_, _> = applied
        .iter()
        .map(|event| (&event.external_id, event))
        .collect();
    let observed: BTreeMap<_, _> = observed
        .iter()
        .map(|event| (&event.external_id, event))
        .collect();
    let mut conflicts = Vec::new();
    for (id, event) in &applied {
        match observed.get(id) {
            None => conflicts.push(conflict(
                event,
                "missing",
                "UbU applied this event and the calendar no longer has it",
            )),
            Some(current) if current != event => conflicts.push(conflict(
                event,
                "drifted",
                "the calendar's copy of this event differs from what UbU applied",
            )),
            Some(_) => {}
        }
    }
    for (id, event) in &observed {
        if applied.contains_key(id) {
            continue;
        }
        if known_external_ids.contains(*id) {
            conflicts.push(conflict(event, "unrecorded", "this event matches a known Task but UbU has no applied record; it will not be adopted"));
        } else {
            conflicts.push(conflict(
                event,
                "foreign",
                "this event was not created by UbU and will not be touched",
            ));
        }
    }
    conflicts.sort_by(|a, b| {
        (&a.conflict_type, &a.external_id).cmp(&(&b.conflict_type, &b.external_id))
    });
    conflicts
}

fn conflict(event: &DesiredEvent, kind: &str, message: &str) -> CalendarConflict {
    CalendarConflict {
        external_id: event.external_id.clone(),
        conflict_type: kind.into(),
        summary: event.summary.clone(),
        message: message.into(),
    }
}

/// Rewrite only the applied belief: retain previously owned ids that still
/// exist, using their observed values. Corrections come from the ordinary
/// desired-versus-applied diff, not repair operations here. Observed foreign and
/// unrecorded events are neither adopted nor mutated. Output is sorted by id.
pub fn repair(applied: &[DesiredEvent], observed: &[DesiredEvent]) -> Vec<DesiredEvent> {
    let owned: BTreeSet<_> = applied.iter().map(|event| &event.external_id).collect();
    observed
        .iter()
        .filter(|event| owned.contains(&event.external_id))
        .map(|event| (&event.external_id, event.clone()))
        .collect::<BTreeMap<_, _>>()
        .into_values()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::calendar_projection::{diff, CalendarOperation};

    fn event(id: &str, title: &str) -> DesiredEvent {
        DesiredEvent {
            external_id: id.into(),
            task_id: format!("task_{id}"),
            summary: title.into(),
            start_at: "2026-09-25T09:00:00Z".into(),
            end_at: "2026-09-25T09:30:00Z".into(),
            color_id: Some("5".into()),
            transparent: false,
            reminders_minutes: vec![10, 0],
        }
    }

    #[test]
    fn wiped_calendar_produces_only_missing_sorted_by_id() {
        let applied = [event("bbbbb", "Standup"), event("aaaaa", "Breakfast")];
        let conflicts = classify(&applied, &[], &BTreeSet::new());
        assert_eq!(
            conflicts
                .iter()
                .map(|c| (c.conflict_type.as_str(), c.external_id.as_str()))
                .collect::<Vec<_>>(),
            vec![("missing", "aaaaa"), ("missing", "bbbbb")]
        );
        assert_eq!(conflicts[0].summary, "Breakfast");
        assert_eq!(
            conflicts[0].message,
            "UbU applied this event and the calendar no longer has it"
        );
    }

    #[test]
    fn each_changed_field_is_one_drift_and_repair_uses_observed_values() {
        let original = event("aaaaa", "Breakfast");
        let mut variants = Vec::new();
        let mut changed = original.clone();
        changed.summary = "Moved breakfast".into();
        variants.push(changed);
        let mut changed = original.clone();
        changed.start_at = "2026-09-25T09:10:00Z".into();
        variants.push(changed);
        let mut changed = original.clone();
        changed.end_at = "2026-09-25T09:45:00Z".into();
        variants.push(changed);
        let mut changed = original.clone();
        changed.color_id = None;
        variants.push(changed);
        let mut changed = original.clone();
        changed.transparent = true;
        variants.push(changed);
        let mut changed = original.clone();
        changed.reminders_minutes.clear();
        variants.push(changed);
        let mut changed = original.clone();
        changed.task_id = "task_other".into();
        variants.push(changed);
        for changed in variants {
            let conflicts = classify(
                std::slice::from_ref(&original),
                std::slice::from_ref(&changed),
                &BTreeSet::new(),
            );
            assert_eq!(conflicts.len(), 1);
            assert_eq!(conflicts[0].conflict_type, "drifted");
            let repaired = repair(
                std::slice::from_ref(&original),
                std::slice::from_ref(&changed),
            );
            assert_eq!(repaired, vec![changed]);
            assert_eq!(
                diff(std::slice::from_ref(&original), &repaired),
                vec![CalendarOperation::Update(original.clone())]
            );
        }
    }

    #[test]
    fn foreign_event_survives_repair_without_adoption_or_operations() {
        let owned = event("aaaaa", "Breakfast");
        let foreign = event("ddddd", "Dentist"); // Same alphabet as a Task-derived id.
        let observed = vec![foreign.clone(), owned.clone()];
        let before = observed.clone();
        let conflicts = classify(std::slice::from_ref(&owned), &observed, &BTreeSet::new());
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].conflict_type, "foreign");
        let repaired = repair(std::slice::from_ref(&owned), &observed);
        assert_eq!(repaired, vec![owned.clone()]);
        assert!(diff(&[owned], &repaired).is_empty());
        assert_eq!(observed, before);
        assert!(observed.contains(&foreign));
    }

    #[test]
    fn known_task_ids_distinguish_unrecorded_from_foreign_without_claiming_either() {
        let owned = event("aaaaa", "Breakfast");
        let known = event("bbbbb", "Unrecorded Task");
        let foreign = event("ccccc", "Dentist");
        let known_ids = BTreeSet::from([known.external_id.clone()]);
        let observed = [known.clone(), foreign.clone()];
        let conflicts = classify(std::slice::from_ref(&owned), &observed, &known_ids);
        assert_eq!(
            conflicts
                .iter()
                .map(|c| c.conflict_type.as_str())
                .collect::<Vec<_>>(),
            vec!["foreign", "missing", "unrecorded"]
        );
        assert_eq!(
            conflicts,
            classify(std::slice::from_ref(&owned), &[foreign, known], &known_ids)
        );
        assert!(repair(&[owned], &observed).is_empty());
    }

    #[test]
    fn unchanged_owned_events_have_no_conflicts_even_without_known_tasks() {
        let applied = [event("bbbbb", "Standup"), event("aaaaa", "Breakfast")];
        let observed = [applied[1].clone(), applied[0].clone()];
        assert!(classify(&applied, &observed, &BTreeSet::new()).is_empty());
        assert!(classify(&[], &[], &BTreeSet::new()).is_empty());
        assert_eq!(repair(&applied, &observed), observed);
    }

    #[test]
    fn repair_of_wiped_calendar_is_empty_and_normal_diff_recreates_every_event() {
        let applied = [event("aaaaa", "Breakfast"), event("bbbbb", "Standup")];
        let repaired = repair(&applied, &[]);
        assert!(repaired.is_empty());
        assert_eq!(
            diff(&applied, &repaired),
            applied
                .into_iter()
                .map(CalendarOperation::Create)
                .collect::<Vec<_>>()
        );
    }
}
