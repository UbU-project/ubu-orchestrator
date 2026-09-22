use crate::{
    errors::{AppError, Result},
    services::routine_service::{live, window},
    state::AppState,
};
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use ubu_store::models::object_record::ObjectRecord;
use utoipa::ToSchema;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RoutineSummaryResponse {
    pub schema_version: String,
    pub routines: Vec<RoutineSummary>,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RoutineSummary {
    pub objective_id: String,
    pub title: String,
    pub done: usize,
    pub skipped: usize,
    pub missed: usize,
    pub pending: usize,
    pub current_streak: usize,
    pub last_occurrence: Option<LastRoutineOccurrence>,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LastRoutineOccurrence {
    pub local_date: String,
    pub outcome: RoutineOutcome,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoutineOutcome {
    Done,
    Skipped,
    Missed,
    Pending,
}

/// Read-only: expiration is reflected in the summary without changing Tasks or Logs.
#[utoipa::path(get, path="/routines", responses((status=200, body=RoutineSummaryResponse)))]
pub async fn summaries(State(state): State<AppState>) -> Result<Json<RoutineSummaryResponse>> {
    let pool = state.inner().store.pool();
    let internal = |e: sqlx::Error| AppError::Internal(e.to_string());
    let rows=sqlx::query_as::<_,ObjectRecord>("SELECT * FROM objects WHERE object_type='Objective' AND json_extract(payload_json,'$.recurrence') IS NOT NULL").fetch_all(pool).await.map_err(internal)?;
    let mut routines = BTreeMap::new();
    for row in rows {
        let payload: Value = serde_json::from_str(&row.payload_json)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        if !live(&row, &payload) {
            continue;
        }
        routines.insert(
            row.id.clone(),
            RoutineSummary {
                objective_id: row.id,
                title: payload["title"].as_str().unwrap_or("").into(),
                done: 0,
                skipped: 0,
                missed: 0,
                pending: 0,
                current_streak: 0,
                last_occurrence: None,
            },
        );
    }
    let ids: Vec<_> = routines.keys().collect();
    let rows=sqlx::query_as::<_,ObjectRecord>("SELECT * FROM objects WHERE object_type='Task' AND json_extract(payload_json,'$.occurrence.routine_objective_id') IN (SELECT value FROM json_each(?))").bind(json!(ids).to_string()).fetch_all(pool).await.map_err(internal)?;
    let now = state.planning_now().inner().unix_timestamp();
    let mut history = BTreeMap::<String, Vec<(String, u64, RoutineOutcome)>>::new();
    for row in rows {
        let payload: Value = serde_json::from_str(&row.payload_json)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let Some((start, end)) = window(&payload) else {
            continue;
        };
        let outcome = match row.status.as_str() {
            "completed" => RoutineOutcome::Done,
            "moot" if payload["moot_reason_code"] == "user_declared_moot" => {
                RoutineOutcome::Skipped
            }
            "failed" => RoutineOutcome::Missed,
            "active" if i128::from(end) <= i128::from(now) => RoutineOutcome::Missed,
            "active" => RoutineOutcome::Pending,
            _ => continue,
        };
        let id = payload["occurrence"]["routine_objective_id"]
            .as_str()
            .unwrap_or("");
        let Some(summary) = routines.get_mut(id) else {
            continue;
        };
        match outcome {
            RoutineOutcome::Done => summary.done += 1,
            RoutineOutcome::Skipped => summary.skipped += 1,
            RoutineOutcome::Missed => summary.missed += 1,
            RoutineOutcome::Pending => summary.pending += 1,
        }
        history.entry(id.into()).or_default().push((
            payload["occurrence"]["local_date"]
                .as_str()
                .unwrap_or("")
                .into(),
            start,
            outcome,
        ));
    }
    for (id, mut entries) in history {
        entries.sort_by(|a, b| (&b.0, b.1).cmp(&(&a.0, a.1)));
        let terminal: Vec<_> = entries
            .into_iter()
            .filter(|e| e.2 != RoutineOutcome::Pending)
            .collect();
        let summary = routines.get_mut(&id).unwrap();
        summary.current_streak = terminal
            .iter()
            .take_while(|e| e.2 == RoutineOutcome::Done)
            .count();
        summary.last_occurrence = terminal.first().map(|e| LastRoutineOccurrence {
            local_date: e.0.clone(),
            outcome: e.2,
        });
    }
    let mut routines: Vec<_> = routines.into_values().collect();
    routines.sort_by(|a, b| (&a.title, &a.objective_id).cmp(&(&b.title, &b.objective_id)));
    Ok(Json(RoutineSummaryResponse {
        schema_version: "routine-summary/1".into(),
        routines,
    }))
}
