//! User-authored one-off Tasks, admitted through the ordinary store boundary.
use axum::http::StatusCode;
use serde_json::{json, Map, Value};
use ubu_core::{core::Task, AuthoritySource, ObjectType, UbuId, VersionRef};
use ubu_store::{models::object_record::NewObjectRecord, queries};

use crate::{
    errors::{AppError, Result},
    state::AppState,
};

const EDITABLE_FIELDS: &[&str] = &[
    "title",
    "description",
    "duration_estimate",
    "allowed_time_range",
    "static_window",
    "due_at",
    "tags",
    "category_tag",
    "occupies_capacity",
    "preconditions",
    "effects",
    "correlation_groups",
    "blocked_by",
    "objective_id",
];

fn editable_fields(fields: &Value) -> Result<&Map<String, Value>> {
    let fields = fields
        .as_object()
        .ok_or_else(|| AppError::BadRequest("Task fields must be a JSON object".into()))?;
    for key in fields.keys() {
        if !EDITABLE_FIELDS.contains(&key.as_str()) {
            return Err(AppError::bad_request_diagnostic(
                "unsupported_capture_field",
                format!("field `{key}` is not editable"),
            ));
        }
    }
    Ok(fields)
}

fn validate(payload: &Value) -> Result<()> {
    if payload
        .get("title")
        .is_none_or(|title| title.is_null() || title.as_str() == Some(""))
    {
        return Err(AppError::bad_request_diagnostic(
            "missing_title",
            "a nonempty title is required",
        ));
    }
    let task: Task =
        serde_json::from_value(payload.clone()).map_err(|e| AppError::BadRequest(e.to_string()))?;
    task.validate()
        .map_err(|e| AppError::BadRequest(e.to_string()))
}

pub async fn capture(state: &AppState, fields: Value) -> Result<(String, i64)> {
    let fields = editable_fields(&fields)?;
    let id = UbuId::new(ObjectType::Task);
    let now = state.planning_now();
    let mut payload = json!({"id":id,"status":"active","provenance":{"created_at":now,"authority_source":"user"}});
    payload.as_object_mut().unwrap().extend(fields.clone());
    validate(&payload)?;
    let envelope = state.envelope_for(
        [(id.clone(), VersionRef::Absent)].into_iter().collect(),
        AuthoritySource::User,
        now,
    )?;
    let record = queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        NewObjectRecord {
            id: id.to_string(),
            object_type: ObjectType::Task.as_str().into(),
            version: 1,
            status: "active".into(),
            compartment_label: "user-capture".into(),
            payload,
            created_at: now.to_string(),
            updated_at: now.to_string(),
        },
    )
    .await?;
    Ok((record.id, record.version))
}

pub async fn edit(
    state: &AppState,
    id: &str,
    expected_version: i64,
    fields: Value,
) -> Result<(String, i64)> {
    let unknown = || AppError::Diagnostic {
        status: StatusCode::NOT_FOUND,
        code: "unknown_task".into(),
        message: format!("Task `{id}` does not exist"),
    };
    let task_id = UbuId::parse(id).map_err(|_| unknown())?;
    task_id
        .require_object_type(ObjectType::Task)
        .map_err(|_| unknown())?;
    let current = queries::get_current_state(state.inner().store.pool(), id)
        .await?
        .ok_or_else(unknown)?;
    if current.object_type != ObjectType::Task.as_str() {
        return Err(unknown());
    }
    let mut payload: Value = serde_json::from_str(&current.payload_json)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    if let Some(occurrence) = payload.get("occurrence") {
        let routine = occurrence["routine_objective_id"]
            .as_str()
            .unwrap_or("unknown");
        return Err(AppError::bad_request_diagnostic(
            "routine_occurrence_not_editable",
            format!("Task `{id}` is a routine occurrence; edit routine `{routine}` instead"),
        ));
    }
    if expected_version != current.version {
        let message = format!(
            "Task `{id}` expected version {expected_version}, current version {}",
            current.version
        );
        return Err(AppError::Diagnostics {
            status: StatusCode::CONFLICT,
            summary: message.clone(),
            items: vec![("version_conflict".into(), message)],
        });
    }
    let fields = editable_fields(&fields)?;
    let object = payload
        .as_object_mut()
        .ok_or_else(|| AppError::Internal("stored Task is not an object".into()))?;
    for (key, value) in fields {
        if value.is_null() {
            object.remove(key);
        } else {
            object.insert(key.clone(), value.clone());
        }
    }
    validate(&payload)?;
    let observed = u64::try_from(current.version).map_err(|e| AppError::Internal(e.to_string()))?;
    let version = current.version.checked_add(1).ok_or_else(|| {
        AppError::Store(ubu_store::StoreError::ObjectVersionExhausted {
            object_id: id.into(),
            version: observed,
        })
    })?;
    let now = state.planning_now();
    let envelope = state.envelope_for(
        [(task_id, VersionRef::Version(observed))]
            .into_iter()
            .collect(),
        AuthoritySource::User,
        now,
    )?;
    let record = queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        NewObjectRecord {
            id: id.into(),
            object_type: ObjectType::Task.as_str().into(),
            version,
            status: current.status,
            compartment_label: current.compartment_label,
            payload,
            created_at: current.created_at,
            updated_at: now.to_string(),
        },
    )
    .await?;
    Ok((record.id, record.version))
}
