//! Pure Calendar event mapping and deterministic projection diffing.
use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};

use crate::api::planning::ScheduledTaskBody;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesiredEvent {
    pub external_id: String,
    pub task_id: String,
    pub summary: String,
    pub start_at: String,
    pub end_at: String,
    pub color_id: Option<String>,
    pub transparent: bool,
    pub reminders_minutes: Vec<i64>,
}

/// Google client-supplied event ids use lowercase a-v and 0-9, 5–1024 characters.
/// Task hex tails satisfy that alphabet. Removing `task_` is reversible by
/// prepending it again; no random id or persisted mapping is needed.
pub fn external_id(task_id: &str) -> Option<String> {
    let tail = task_id.strip_prefix("task_")?;
    ((5..=1024).contains(&tail.len())
        && tail.bytes().all(|c| matches!(c, b'a'..=b'v' | b'0'..=b'9')))
    .then(|| tail.to_owned())
}

/// Captured ids must validate on their own; deriving a fallback would duplicate the meeting.
pub fn external_id_for(task_id: &str, captured_event_id: Option<&str>) -> Option<String> {
    match captured_event_id {
        Some(origin) => external_id(&format!("task_{origin}")),
        None => external_id(task_id),
    }
}

pub fn desired_events(
    steps: &[ScheduledTaskBody],
    reminders_by_objective: &BTreeMap<String, Vec<i64>>,
    objective_of_task: &BTreeMap<String, String>,
    captured_origins: &BTreeMap<String, String>,
) -> Vec<DesiredEvent> {
    let mut events: Vec<_> = steps
        .iter()
        .filter_map(|step| {
            Some(DesiredEvent {
                external_id: external_id_for(&step.task_id, captured_origins.get(&step.task_id).map(String::as_str))?,
                task_id: step.task_id.clone(),
                summary: step.summary.clone(),
                start_at: step.start_at.clone(),
                end_at: step.end_at.clone(),
                color_id: step.gcal_color_id.clone(),
                transparent: !step.occupies_capacity,
                reminders_minutes: objective_of_task
                    .get(&step.task_id)
                    .and_then(|id| reminders_by_objective.get(id))
                    .cloned()
                    .unwrap_or_default(),
            })
        })
        .collect();
    events.sort_by(|a, b| (&a.start_at, &a.external_id).cmp(&(&b.start_at, &b.external_id)));
    events
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "event", rename_all = "snake_case")]
pub enum CalendarOperation {
    Create(DesiredEvent),
    Update(DesiredEvent),
    Delete {
        external_id: String,
        summary: String,
    },
}

/// Compare event sets keyed by external id; creates, updates, then deletes,
/// each in external-id order. An unchanged event produces no operation.
pub fn diff(desired: &[DesiredEvent], existing: &[DesiredEvent]) -> Vec<CalendarOperation> {
    let desired: BTreeMap<_, _> = desired.iter().map(|e| (&e.external_id, e)).collect();
    let existing: BTreeMap<_, _> = existing.iter().map(|e| (&e.external_id, e)).collect();
    let mut creates = Vec::new();
    let mut updates = Vec::new();
    let mut deletes = Vec::new();
    for (id, event) in &desired {
        match existing.get(id) {
            None => creates.push(CalendarOperation::Create((*event).clone())),
            Some(old) if old != event => updates.push(CalendarOperation::Update((*event).clone())),
            Some(_) => {}
        }
    }
    for (id, event) in &existing {
        if !desired.contains_key(id) {
            deletes.push(CalendarOperation::Delete {
                external_id: (*id).clone(),
                summary: event.summary.clone(),
            });
        }
    }
    creates.extend(updates);
    creates.extend(deletes);
    creates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(id: &str) -> ScheduledTaskBody {
        ScheduledTaskBody {
            index: 0,
            task_id: format!("task_{id}"),
            summary: "Synthetic event".into(),
            start: 1,
            end: 2,
            start_at: "2026-09-24T09:00:00Z".into(),
            end_at: "2026-09-24T09:30:00Z".into(),
            depends_on: vec![],
            static_anchor: false,
            placement_authority: "planner".into(),
            occupies_capacity: true,
            category_tag: None,
            gcal_color_id: None,
        }
    }
    fn event(id: &str) -> DesiredEvent {
        desired_events(&[step(id)], &BTreeMap::new(), &BTreeMap::new(), &BTreeMap::new())
            .pop()
            .unwrap()
    }

    #[test]
    fn captured_origins_override_derived_ids_and_never_fall_back() {
        assert_eq!(external_id_for("task_aaaaa", None).as_deref(), Some("aaaaa"));
        assert_eq!(external_id_for("task_aaaaa", Some("bbbbb")).as_deref(), Some("bbbbb"));
        for invalid in ["bad!", "abc", "UPPER", "abcwx", "", &"a".repeat(1025)] {
            assert!(external_id_for("task_aaaaa", Some(invalid)).is_none());
            let origins = BTreeMap::from([("task_aaaaa".into(), invalid.into())]);
            assert!(desired_events(&[step("aaaaa")], &BTreeMap::new(), &BTreeMap::new(), &origins).is_empty());
        }
        let origins = BTreeMap::from([("task_aaaaa".into(), "bbbbb".into())]);
        assert_eq!(desired_events(&[step("aaaaa")], &BTreeMap::new(), &BTreeMap::new(), &origins)[0].external_id, "bbbbb");
    }

    #[test]
    fn external_ids_validate_the_prefix_alphabet_and_length() {
        for tail in ["0123456789abcdef", "abcuv", &"a".repeat(1024)] {
            let task = format!("task_{tail}");
            assert_eq!(external_id(&task).as_deref(), Some(tail));
            assert_eq!(format!("task_{}", external_id(&task).unwrap()), task);
        }
        for id in [
            "task_1234",
            "task_",
            "obj_abcde",
            "task_abcwx",
            "task_ABCDE",
            "task_ab_cd",
            "task_éabcd",
            &format!("task_{}", "a".repeat(1025)),
        ] {
            assert_eq!(external_id(id), None, "{id}");
        }
        assert!(desired_events(&[step("bad!")], &BTreeMap::new(), &BTreeMap::new(), &BTreeMap::new()).is_empty());
    }

    #[test]
    fn transparency_color_and_event_order_follow_the_steps() {
        let mut transparent = step("bbbbb");
        transparent.occupies_capacity = false;
        transparent.gcal_color_id = Some("9".into());
        let opaque = step("aaaaa");
        let mut early = step("ccccc");
        early.start_at = "2026-09-24T08:00:00Z".into();
        let events = desired_events(
            &[transparent, opaque, early],
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        );
        assert_eq!(
            events
                .iter()
                .map(|e| e.external_id.as_str())
                .collect::<Vec<_>>(),
            vec!["ccccc", "aaaaa", "bbbbb"]
        );
        assert!(!events[1].transparent);
        assert_eq!(events[1].color_id, None);
        assert!(events[2].transparent);
        assert_eq!(events[2].color_id.as_deref(), Some("9"));
    }

    #[test]
    fn only_routine_occurrences_receive_objective_reminders() {
        let reminders = BTreeMap::from([("obj_routine".into(), vec![10, 0])]);
        let objectives = BTreeMap::from([
            ("task_aaaaa".into(), "obj_routine".into()),
            ("task_ccccc".into(), "obj_absent".into()),
        ]);
        let events = desired_events(
            &[step("aaaaa"), step("bbbbb"), step("ccccc")],
            &reminders,
            &objectives,
            &BTreeMap::new(),
        );
        assert_eq!(events[0].reminders_minutes, vec![10, 0]);
        assert!(events[1].reminders_minutes.is_empty());
        assert!(events[2].reminders_minutes.is_empty());
    }

    #[test]
    fn unchanged_sets_have_no_operations_and_first_run_is_sorted_creates() {
        let events = vec![event("bbbbb"), event("aaaaa")];
        assert!(diff(&events, &[events[1].clone(), events[0].clone()]).is_empty());
        assert!(diff(&[], &[]).is_empty());
        assert_eq!(
            diff(&events, &[]),
            vec![
                CalendarOperation::Create(events[1].clone()),
                CalendarOperation::Create(events[0].clone())
            ]
        );
    }

    #[test]
    fn changing_an_event_field_produces_exactly_one_update() {
        let original = event("aaaaa");
        let mut changed = original.clone();
        changed.start_at = "2026-09-24T08:30:00Z".into();
        assert_eq!(
            diff(&[changed.clone()], &[original]),
            vec![CalendarOperation::Update(changed)]
        );
    }

    #[test]
    fn removed_events_follow_sorted_creates_and_updates() {
        let existing = vec![event("bbbbb"), event("aaaaa"), event("ccccc")];
        let mut a = existing[1].clone();
        a.summary = "Changed A".into();
        let mut b = existing[0].clone();
        b.reminders_minutes = vec![0];
        let d = event("ddddd");
        let e = event("eeeee");
        assert_eq!(
            diff(&[b.clone(), e.clone(), a.clone(), d.clone()], &existing),
            vec![
                CalendarOperation::Create(d),
                CalendarOperation::Create(e),
                CalendarOperation::Update(a),
                CalendarOperation::Update(b),
                CalendarOperation::Delete {
                    external_id: "ccccc".into(),
                    summary: existing[2].summary.clone()
                }
            ]
        );
        let deleted = diff(&[], &existing);
        assert_eq!(
            deleted,
            vec!["aaaaa", "bbbbb", "ccccc"]
                .into_iter()
                .map(|id| CalendarOperation::Delete {
                    external_id: id.into(),
                    summary: "Synthetic event".into()
                })
                .collect::<Vec<_>>()
        );
    }
}
