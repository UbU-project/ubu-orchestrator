//! Capture only foreign Calendar commitments through ordinary Task admission.
use super::{
    calendar_apply::{self, StoredCalendarResult, CALENDAR_PROJECTION_RESULT_SCHEMA_VERSION},
    calendar_client::{CalendarApi, CalendarExportMode, RecordingCalendarApi},
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

#[derive(Debug, Clone, PartialEq)]
pub struct CapturedTask {
    /// None for a new source: allocation belongs to admission, keeping planning pure.
    pub task_id: Option<String>,
    pub origin_event_id: String,
    pub title: String,
    pub start_at: String,
    pub end_at: String,
    pub occupies_capacity: bool,
    pub category_tag: Option<String>,
}

pub fn plan_capture(
    foreign: &[DesiredEvent],
    palette_inverse: &BTreeMap<String, Option<String>>,
    existing_by_source: &BTreeMap<String, String>,
) -> (Vec<CapturedTask>, Vec<DiagnosticBody>) {
    let mut tasks = Vec::new();
    let mut diagnostics = Vec::new();
    let mut seen = BTreeSet::new();
    for event in foreign {
        if !seen.insert(&event.external_id) {
            continue;
        }
        let window = CalendarTimeRange::parse(&event.start_at, &event.end_at);
        if external_id_for("", Some(&event.external_id)).is_none()
            || window.is_err()
            || event.summary.trim().is_empty()
        {
            diagnostics.push(DiagnosticBody {
                code: "capture_event_invalid".into(),
                message: "Calendar event has an unusable id, title or concrete time span; skipped"
                    .into(),
            });
            continue;
        }
        let window = window.unwrap();
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
        let category_tag = match event
            .color_id
            .as_ref()
            .and_then(|color| palette_inverse.get(color))
        {
            Some(Some(category)) => Some(category.clone()),
            Some(None) => {
                diagnostics.push(DiagnosticBody { code: "capture_colour_ambiguous".into(), message: format!("Calendar event `{}` has a colour shared by multiple categories; no category assigned", event.external_id) });
                None
            }
            None => None,
        };
        tasks.push(CapturedTask {
            task_id: existing_by_source.get(&event.external_id).cloned(),
            origin_event_id: event.external_id.clone(),
            title: event.summary.clone(),
            start_at,
            end_at,
            occupies_capacity: !event.transparent,
            category_tag,
        });
    }
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
        state
            .calendar_api()
            .unwrap_or_else(|| Arc::new(RecordingCalendarApi::with_events(applied.clone())))
    };
    let mut observed = client
        .list_events(&observation_range)
        .await
        .map_err(AppError::Upstream)?;
    normalize_observed(&mut observed, &applied);
    observed.retain(|event| range.overlaps(event) || applied.iter().any(|old| old.external_id == event.external_id));
    let mut diagnostics = client.take_diagnostics().await;
    let interaction = super::calendar_interaction::apply(state, &observed, &mut applied).await?;
    diagnostics.extend(interaction.diagnostics);
    let conflicts =
        calendar_reconcile::classify(&applied, &observed, &known_external_ids(pool).await?);
    let foreign_ids: BTreeSet<_> = conflicts
        .iter()
        .filter(|c| c.conflict_type == "foreign")
        .map(|c| c.external_id.as_str())
        .collect();
    let foreign: Vec<_> = observed
        .iter()
        .filter(|e| foreign_ids.contains(e.external_id.as_str()))
        .cloned()
        .collect();
    let existing = calendar_sources::by_source(pool).await?;
    let by_source = existing
        .iter()
        .map(|(source, (row, _))| (source.clone(), row.id.clone()))
        .collect();
    let (tasks, plan_diagnostics) = plan_capture(
        &foreign,
        &state.inner().category_palette.inverse(),
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
        skipped: invalid + diagnostics.len(),
        diagnostics: Vec::new(),
    };
    diagnostics.extend(plan_diagnostics);
    // An unchanged owned source is a reuse, never a second admission. Owned drift
    // remains reconciliation/P1B-33 work, even if its source belongs to a Task.
    for event in observed
        .iter()
        .filter(|event| !foreign_ids.contains(event.external_id.as_str()))
    {
        if interaction.changed_external_ids.contains(&event.external_id) { continue; }
        if existing
            .get(&event.external_id)
            .is_some_and(|(row, _)| row.status == "active")
            && applied.iter().any(|old| old == event)
        {
            response.updated += 1;
        } else {
            response.skipped += 1;
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
        payload["static_window"] = json!({"start":task.start_at,"end":task.end_at});
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
        if previous.is_none_or(|(_, old)| old != &payload) {
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
        if previous.is_some() {
            response.updated += 1;
        } else {
            response.captured += 1;
        }
        let mut origin = foreign
            .iter()
            .find(|event| event.external_id == task.origin_event_id)
            .unwrap()
            .clone();
        origin.task_id = id.to_string();
        origin.start_at = task.start_at;
        origin.end_at = task.end_at;
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
