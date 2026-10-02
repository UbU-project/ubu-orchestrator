//! Capture only foreign Calendar events through ordinary Task admission.
//!
//! A colour decides the placement. An event with no colour is work for UbU to
//! schedule: a Dynamic Task of the event's length, at no fixed time. An event
//! with any colour is a commitment at its own time: a Static Task, whose
//! category is the colour's. This is the inverse of export, where a Static Task
//! carries its category colour and a Dynamic one carries none, so a round trip
//! closes. See docs/CALENDAR_CAPTURE.md.
//!
//! UbU owns an event whose id can round-trip through `external_id`. Any other
//! event, such as an instance of a recurring one, is captured as occupied time:
//! a Static Task with a minted handle that UbU never writes back to, never
//! exports and never records as applied. UbU cannot move such an event, so it
//! is Static whatever its colour. See docs/CALENDAR_OCCUPANCY.md.
use super::{
    calendar_apply::{self, StoredCalendarResult, CALENDAR_PROJECTION_RESULT_SCHEMA_VERSION},
    calendar_client::{CalendarApi, CalendarExportMode},
    calendar_google::GoogleCalendarApi,
    calendar_projection::{external_id_for, DesiredEvent},
    calendar_range::CalendarTimeRange,
    calendar_reconcile,
    calendar_reconciliation_service::known_external_ids,
    calendar_sources,
};
use crate::{
    api::{
        calendar_capture::{CalendarCaptureResponse, CALENDAR_CAPTURE_SCHEMA_VERSION},
        planning::DiagnosticBody,
    },
    errors::{AppError, Result},
    state::AppState,
};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use ubu_core::{
    core::Task, projection::ProjectionResultStatus, AuthoritySource, ObjectType, UbuId, VersionRef,
};
use ubu_store::{models::object_record::NewObjectRecord, queries};

/// What the event's colour decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapturedPlacement {
    /// A coloured event, or one UbU cannot own: pinned to the event's own window.
    Static,
    /// An uncoloured event: the planner decides when. Only the event's length is
    /// kept, as a fixed duration; its start time is discarded.
    Dynamic { seconds: u64 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct CapturedTask {
    /// None for a new source: allocation belongs to admission, keeping planning pure.
    pub task_id: Option<String>,
    pub origin_event_id: String,
    pub title: String,
    /// The event's own window. A Static Task is pinned to it. For a Dynamic Task
    /// it is only what the applied record remembers of the event.
    pub start_at: String,
    pub end_at: String,
    pub placement: CapturedPlacement,
    pub occupies_capacity: bool,
    pub category_tag: Option<String>,
}

/// One ownership rule and one operator-facing reason for capture and reconciliation.
pub fn not_ownable_diagnostic(external_id: &str) -> Option<DiagnosticBody> {
    external_id_for("", Some(external_id)).is_none().then(|| DiagnosticBody {
        code: "capture_event_not_ownable".into(),
        message: format!("Calendar event `{external_id}` cannot be captured: its id cannot be a UbU Task handle, so UbU cannot own it"),
    })
}

/// How many unowned events one capture names before it counts the rest.
pub const MAX_OCCUPANCY_NAMED: usize = 3;

/// One diagnostic for every event of a capture that UbU cannot own, in the shape
/// `suggest_tags::skipped_occurrences` uses: the first few are named and the rest
/// are counted. A week of a daily commitment is one line, not seven. Ids only:
/// an event's title is never echoed.
pub fn occupancy_diagnostic(ids: &[String]) -> Option<DiagnosticBody> {
    let message = match ids {
        [] => return None,
        [id] => format!("Calendar event `{id}` cannot be owned by UbU, so its time is recorded as an occupied window that UbU will never write back to or export"),
        _ => {
            let named: Vec<_> = ids.iter().take(MAX_OCCUPANCY_NAMED).map(|id| format!("`{id}`")).collect();
            let rest = match ids.len() - named.len() {
                0 => String::new(),
                more => format!(" and {more} more"),
            };
            format!(
                "{} Calendar events cannot be owned by UbU, so the time of each is recorded as an occupied window that UbU will never write back to or export: {}{rest}",
                ids.len(),
                named.join(", ")
            )
        }
    };
    Some(DiagnosticBody { code: "capture_occupancy_only".into(), message })
}

pub fn plan_capture(
    foreign: &[DesiredEvent],
    palette_inverse: &BTreeMap<String, Option<String>>,
    existing_by_source: &BTreeMap<String, String>,
) -> (Vec<CapturedTask>, Vec<DiagnosticBody>) {
    let mut tasks = Vec::new();
    let mut diagnostics = Vec::new();
    let mut seen = BTreeSet::new();
    let mut unowned = Vec::new();
    for event in foreign {
        if !seen.insert(&event.external_id) {
            continue;
        }
        let window = CalendarTimeRange::parse(&event.start_at, &event.end_at);
        // An id UbU cannot own, such as a recurring instance, is still occupied
        // time. Record the occupancy without claiming the event: the Task gets a
        // minted handle, the Google id lives in provenance so a repeat capture
        // updates rather than duplicates, and `external_id_for` keeps it out of
        // every desired export set because its origin cannot round-trip.
        let ownable = not_ownable_diagnostic(&event.external_id).is_none();
        if window.is_err() || event.summary.trim().is_empty() {
            diagnostics.push(DiagnosticBody {
                code: "capture_event_invalid".into(),
                message: "Calendar event has an unusable title or concrete time span; skipped"
                    .into(),
            });
            continue;
        }
        let window = window.unwrap();
        // Whole planning seconds, as the canonical window below has them.
        let span_seconds = u64::try_from(window.end.inner().unix_timestamp() - window.start.inner().unix_timestamp()).unwrap_or(0);
        // Planning coordinates are whole UTC seconds; canonicalize both sides of the round trip.
        let times = [window.start, window.end].map(|timestamp| {
            u64::try_from(timestamp.inner().unix_timestamp())
                .ok()
                .and_then(|seconds| crate::planning_time::timestamp_at(seconds).ok())
        });
        let [Some(start_at), Some(end_at)] = times else {
            diagnostics.push(DiagnosticBody {
                code: "capture_event_invalid".into(),
                message: "Calendar event is outside the supported planning time range; skipped"
                    .into(),
            });
            continue;
        };
        if start_at >= end_at {
            diagnostics.push(DiagnosticBody {
                code: "capture_event_invalid".into(),
                message: "Calendar event must span at least one planning second; skipped".into(),
            });
            continue;
        }
        // The colour decides the placement, and only its absence makes work
        // Dynamic. A colour mapped to no category, or to several, is still a
        // colour: the event is a commitment whose category is merely unknown.
        let (placement, category_tag) = match &event.color_id {
            None if ownable => {
                diagnostics.push(DiagnosticBody { code: "capture_colour_absent".into(), message: format!("Calendar event `{}` has no colour, so it is taken as work for UbU to schedule: a Dynamic Task of the event's length, at no fixed time", event.external_id) });
                (CapturedPlacement::Dynamic { seconds: span_seconds }, None)
            }
            None => {
                diagnostics.push(DiagnosticBody { code: "capture_colour_absent".into(), message: format!("Calendar event `{}` has no colour, but UbU cannot own it and so cannot move it: it stays a commitment at its own time, with no category", event.external_id) });
                (CapturedPlacement::Static, None)
            }
            Some(color) => {
                let category = match palette_inverse.get(color) {
                    Some(Some(category)) => Some(category.clone()),
                    Some(None) => {
                        diagnostics.push(DiagnosticBody { code: "capture_colour_ambiguous".into(), message: format!("Calendar event `{}` has a colour shared by multiple categories; no category assigned", event.external_id) });
                        None
                    }
                    None => {
                        diagnostics.push(DiagnosticBody { code: "capture_colour_unmapped".into(), message: format!("Calendar event `{}` has unmapped colour `{color}`; no category assigned; map that colour in Settings to assign a category", event.external_id) });
                        None
                    }
                };
                (CapturedPlacement::Static, category)
            }
        };
        if !ownable {
            unowned.push(event.external_id.clone());
        }
        // A Dynamic Task that occupied nothing would be scheduled into a void, so
        // it always occupies capacity. A Free commitment that does not block is a
        // real distinction, and a Static capture keeps it.
        let occupies_capacity = match placement {
            CapturedPlacement::Static => !event.transparent,
            CapturedPlacement::Dynamic { .. } => true,
        };
        tasks.push(CapturedTask {
            task_id: existing_by_source.get(&event.external_id).cloned(),
            origin_event_id: event.external_id.clone(),
            title: event.summary.clone(),
            start_at,
            end_at,
            placement,
            occupies_capacity,
            category_tag,
        });
    }
    // After the per-event colour diagnostics, and once for the whole capture.
    diagnostics.extend(occupancy_diagnostic(&unowned));
    (tasks, diagnostics)
}

/// Wire event ids cannot encode a captured Task id. Restore identity from ownership,
/// and compare times in the same UTC coordinates as the planner.
pub fn normalize_observed(observed: &mut [DesiredEvent], applied: &[DesiredEvent]) {
    for event in observed {
        if let Some(old) = applied
            .iter()
            .find(|old| old.external_id == event.external_id)
        {
            event.task_id = old.task_id.clone();
        }
        for timestamp in [&mut event.start_at, &mut event.end_at] {
            if let Ok(parsed) = ubu_core::UbuTimestamp::parse(timestamp.as_str()) {
                if let Ok(seconds) = u64::try_from(parsed.inner().unix_timestamp()) {
                    if let Ok(canonical) = crate::planning_time::timestamp_at(seconds) {
                        *timestamp = canonical;
                    }
                }
            }
        }
    }
}

fn internal(error: impl std::fmt::Display) -> AppError {
    AppError::Internal(error.to_string())
}

pub async fn capture(
    state: &AppState,
    mode: CalendarExportMode,
) -> Result<CalendarCaptureResponse> {
    mode.ensure_available(state)?;
    let _guard = state.inner().calendar_projection_lock.lock().await;
    let pool = state.inner().store.pool();
    let mut applied = calendar_apply::last_applied_events(pool).await?;
    let range = CalendarTimeRange::planning(state).await?;
    // One read includes recent owned events for completion/reopen. Foreign
    // capture still uses the original planning horizon, never this lookback.
    let mut observation_range = range.clone();
    let lookback = crate::planning_time::timestamp_at(u64::try_from(state.planning_now().inner().unix_timestamp()).map_err(internal)?.saturating_sub(86_400))?;
    observation_range.start = observation_range.start.min(ubu_core::UbuTimestamp::parse(&lookback).map_err(internal)?);
    let client: Arc<dyn CalendarApi> = if mode == CalendarExportMode::Live {
        Arc::new(GoogleCalendarApi::new(&state.inner().config).map_err(internal)?)
    } else {
        state.mock_calendar_api(&applied)
    };
    let mut observed = client
        .list_events(&observation_range)
        .await
        .map_err(AppError::Upstream)?;
    normalize_observed(&mut observed, &applied);
    observed.retain(|event| range.overlaps(event) || applied.iter().any(|old| old.external_id == event.external_id));
    let mut diagnostics = client.take_diagnostics().await;
    let wire_skipped = diagnostics.len();
    // An event that gained or lost its colour since UbU last saw it. Read here,
    // before a gesture accepts the whole observation into the applied record.
    let colour_flipped: BTreeSet<String> = observed
        .iter()
        .filter(|event| applied.iter().any(|old| old.external_id == event.external_id && old.color_id.is_some() != event.color_id.is_some()))
        .map(|event| event.external_id.clone())
        .collect();
    let interaction = super::calendar_interaction::apply(state, &observed, &mut applied).await?;
    diagnostics.extend(interaction.diagnostics);
    let conflicts =
        calendar_reconcile::classify(&applied, &observed, &known_external_ids(pool).await?);
    let foreign_ids: BTreeSet<_> = conflicts
        .iter()
        .filter(|c| c.conflict_type == "foreign")
        .map(|c| c.external_id.as_str())
        .collect();
    let existing = calendar_sources::by_source(pool).await?;
    // A captured Task follows its event's colour for as long as the event exists.
    // Such an event is in the applied record, so it is not foreign and is not
    // captured again. But when it gains or loses its colour the same rule decides
    // its placement afresh: the Task becomes Static or Dynamic, and is not left as
    // drift. Only a Task that capture made is read this way. An event UbU exported
    // for a Task of its own is never offered here: its colour is a gesture, and
    // means done.
    let replaced: BTreeSet<&str> = colour_flipped
        .iter()
        .filter(|id| !foreign_ids.contains(id.as_str()))
        .filter(|id| existing.get(*id).is_some_and(|(row, _)| row.status == "active"))
        .map(String::as_str)
        .collect();
    let foreign: Vec<_> = observed
        .iter()
        .filter(|e| foreign_ids.contains(e.external_id.as_str()) || replaced.contains(e.external_id.as_str()))
        .cloned()
        .collect();
    let by_source = existing
        .iter()
        .map(|(source, (row, _))| (source.clone(), row.id.clone()))
        .collect();
    let (tasks, plan_diagnostics) = plan_capture(
        &foreign,
        &crate::category_palette::CategoryPalette::from_pool(pool).await?.inverse(),
        &by_source,
    );
    let invalid = foreign
        .iter()
        .map(|e| &e.external_id)
        .collect::<BTreeSet<_>>()
        .len()
        - tasks.len();
    let mut response = CalendarCaptureResponse {
        schema_version: CALENDAR_CAPTURE_SCHEMA_VERSION.into(),
        captured: 0,
        updated: interaction.changed_external_ids.len(),
        moved: interaction.moved,
        resized: interaction.resized,
        unchanged: 0,
        skipped: invalid + wire_skipped,
        diagnostics: Vec::new(),
    };
    diagnostics.extend(plan_diagnostics);
    // Successful gestures changed the Task; otherwise report owned matches and
    // unresolved drift separately from entries that could not be captured.
    for event in observed.iter().filter(|event| !foreign_ids.contains(event.external_id.as_str())) {
        if interaction.changed_external_ids.contains(&event.external_id) || replaced.contains(event.external_id.as_str()) { continue; }
        match applied.iter().find(|old| old.external_id == event.external_id) {
            Some(old) if old == event => response.unchanged += 1,
            Some(old) => diagnostics.push(DiagnosticBody {
                code: "capture_owned_drift".into(),
                message: format!("Owned Task `{}` event `{}` differs from the applied record; no Task update was made", old.task_id, event.external_id),
            }),
            None => {
                response.skipped += 1;
                diagnostics.push(DiagnosticBody {
                    code: "capture_unrecorded_event".into(),
                    message: format!("Event `{}` matches known Task evidence without applied ownership; not captured", event.external_id),
                });
            }
        }
    }
    let now = state.planning_now();
    let mut recorded = !interaction.changed_external_ids.is_empty();
    for task in tasks {
        let previous = existing.get(&task.origin_event_id);
        if previous.is_some_and(|(row, _)| row.status != "active") {
            response.skipped += 1;
            continue;
        }
        let id = match &task.task_id {
            Some(id) => UbuId::parse(id).map_err(internal)?,
            None => UbuId::new(ObjectType::Task),
        };
        let mut payload = previous.map(|(_,payload)| payload.clone()).unwrap_or_else(|| json!({
            "id":id, "status":"active", "provenance":{"created_at":now,"authority_source":"user","source":{"source_kind":"google_calendar","source_id":task.origin_event_id}}
        }));
        payload["title"] = task.title.clone().into();
        // One scheduling form, never both. A Task that changes placement loses
        // the other form's field rather than keeping a stale one.
        let fields = payload.as_object_mut().unwrap();
        match task.placement {
            CapturedPlacement::Static => {
                if fields.get("static_window").is_none_or(|window| window.is_null()) {
                    fields.remove("duration_estimate");
                }
                fields.insert("static_window".into(), json!({"start":task.start_at,"end":task.end_at}));
            }
            CapturedPlacement::Dynamic { seconds } => {
                fields.remove("static_window");
                fields.insert("duration_estimate".into(), json!({"type":"fixed","seconds":seconds}));
            }
        }
        payload["occupies_capacity"] = task.occupies_capacity.into();
        if let Some(category) = &task.category_tag {
            payload["category_tag"] = category.clone().into();
            // Core requires category_tag to be an exact member of tags.
            let mut tags = payload["tags"].as_array().cloned().unwrap_or_default();
            if !tags.iter().any(|tag| tag.as_str() == Some(category)) {
                tags.push(category.clone().into());
            }
            payload["tags"] = tags.into();
        } else {
            payload.as_object_mut().unwrap().remove("category_tag");
        }
        let typed: Task = serde_json::from_value(payload.clone()).map_err(internal)?;
        typed.validate().map_err(internal)?;
        let changed = previous.is_none_or(|(_, old)| old != &payload);
        if changed {
            let (version, observed_version, created_at, compartment_label) =
                if let Some((row, _)) = previous {
                    let version = row
                        .version
                        .checked_add(1)
                        .ok_or_else(|| internal("Calendar Task version exhausted"))?;
                    (
                        version,
                        VersionRef::Version(u64::try_from(row.version).map_err(internal)?),
                        row.created_at.clone(),
                        row.compartment_label.clone(),
                    )
                } else {
                    (
                        1,
                        VersionRef::Absent,
                        now.to_string(),
                        "user-capture".into(),
                    )
                };
            let envelope = state.envelope_for(
                [(id.clone(), observed_version)].into_iter().collect(),
                AuthoritySource::User,
                now,
            )?;
            queries::admit_object(
                pool,
                &envelope,
                NewObjectRecord {
                    id: id.to_string(),
                    object_type: ObjectType::Task.as_str().into(),
                    version,
                    status: "active".into(),
                    compartment_label,
                    payload,
                    created_at,
                    updated_at: now.to_string(),
                },
            )
            .await?;
        }
        if previous.is_none() {
            response.captured += 1;
        } else if changed {
            // A gesture on the same event was counted as its update already.
            if !interaction.changed_external_ids.contains(&task.origin_event_id) {
                response.updated += 1;
            }
        } else {
            response.unchanged += 1;
        }
        // Only an event UbU can own enters the applied record. An unowned one stays
        // foreign to reconciliation and is read afresh by every capture: its
        // occupancy is a Task, never a claim that UbU applied the event.
        if not_ownable_diagnostic(&task.origin_event_id).is_some() {
            continue;
        }
        let mut origin = foreign
            .iter()
            .find(|event| event.external_id == task.origin_event_id)
            .unwrap()
            .clone();
        origin.task_id = id.to_string();
        origin.start_at = task.start_at;
        origin.end_at = task.end_at;
        // An event already in the applied record is replaced, not listed twice.
        applied.retain(|old| old.external_id != origin.external_id);
        applied.push(origin);
        recorded = true;
    }
    if recorded {
        applied.sort_by(|a, b| a.external_id.cmp(&b.external_id));
        calendar_apply::persist_result(
            pool,
            &StoredCalendarResult {
                schema_version: CALENDAR_PROJECTION_RESULT_SCHEMA_VERSION.into(),
                preview_id: UbuId::new(ObjectType::ProjectionPreview).to_string(),
                status: ProjectionResultStatus::Applied,
                applied_events: applied,
                operation_results: Vec::new(),
                diagnostics: Vec::new(),
            },
            now,
        )
        .await?;
    }
    response.diagnostics = diagnostics;
    Ok(response)
}
