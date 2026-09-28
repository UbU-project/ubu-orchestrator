//! Read-only Task views. Nothing here writes, and occurrences stay visible.
use crate::{
    api::task::TASK_READ_SCHEMA_VERSION,
    api::task::{TaskListResponse, TaskPlacement, TaskReadResponse, TaskSummary},
    errors::{AppError, Result},
    state::AppState,
};
use axum::http::StatusCode;
use serde_json::Value;
use std::collections::BTreeMap;
use ubu_core::{
    core::{ContainerStatus, TaskStatus},
    ObjectType, UbuId,
};
use ubu_store::{models::object_record::ObjectRecord, queries};

fn internal(error: impl std::fmt::Display) -> AppError {
    AppError::Internal(error.to_string())
}
fn is_occurrence(payload: &Value) -> bool {
    payload.get("occurrence").is_some_and(|v| !v.is_null())
}
fn text(payload: &Value, field: &str) -> Option<String> {
    payload[field].as_str().map(str::to_owned)
}

pub async fn get(state: &AppState, id: &str) -> Result<TaskReadResponse> {
    let unknown = || AppError::Diagnostic {
        status: StatusCode::NOT_FOUND,
        code: "unknown_task".into(),
        message: format!("Task `{id}` does not exist"),
    };
    UbuId::parse(id)
        .and_then(|task_id| task_id.require_object_type(ObjectType::Task))
        .map_err(|_| unknown())?;
    let row = queries::get_current_state(state.inner().store.pool(), id)
        .await?
        .filter(|row| row.object_type == ObjectType::Task.as_str())
        .ok_or_else(unknown)?;
    let payload: Value = serde_json::from_str(&row.payload_json).map_err(internal)?;
    Ok(TaskReadResponse {
        schema_version: TASK_READ_SCHEMA_VERSION.into(),
        task_id: row.id,
        version: row.version,
        status: row.status,
        is_routine_occurrence: is_occurrence(&payload),
        payload,
    })
}

/// One status per call, so the order within a response is creation time then id.
pub async fn list(state: &AppState, status: Option<&str>) -> Result<TaskListResponse> {
    let status = status.unwrap_or("active");
    let status = serde_json::from_value::<TaskStatus>(Value::String(status.into()))
        .map_err(|_| {
            AppError::bad_request_diagnostic(
                "unknown_task_status",
                format!("`{status}` is not one of active, completed, failed, moot"),
            )
        })?
        .as_str();
    let mut containers = BTreeMap::new();
    for (row, container) in super::decomposition::containers(state).await? {
        if container.status == ContainerStatus::Active {
            for item in &container.items {
                containers.insert(item.object_ref.id.to_string(), row.id.clone());
            }
        }
    }
    let rows = sqlx::query_as::<_, ObjectRecord>(
        "SELECT * FROM objects WHERE object_type='Task' AND status=? ORDER BY status, created_at, id",
    )
    .bind(status)
    .fetch_all(state.inner().store.pool())
    .await
    .map_err(internal)?;
    let mut tasks = Vec::new();
    for row in rows {
        let payload: Value = serde_json::from_str(&row.payload_json).map_err(internal)?;
        tasks.push(TaskSummary {
            title: text(&payload, "title").unwrap_or_default(),
            placement: if payload.get("static_window").is_some_and(|v| !v.is_null()) {
                TaskPlacement::Static
            } else {
                TaskPlacement::Planned
            },
            duration_estimate: payload.get("duration_estimate").cloned(),
            due_at: text(&payload, "due_at"),
            objective_id: text(&payload, "objective_id"),
            category_tag: text(&payload, "category_tag"),
            is_routine_occurrence: is_occurrence(&payload),
            container_id: containers.get(&row.id).cloned(),
            task_id: row.id,
            status: row.status,
            version: row.version,
        });
    }
    Ok(TaskListResponse {
        schema_version: TASK_READ_SCHEMA_VERSION.into(),
        status: status.into(),
        tasks,
    })
}
