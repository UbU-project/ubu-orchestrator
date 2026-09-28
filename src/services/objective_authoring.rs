//! Native Objective authoring. A routine is an evergreen Objective (UBU-D0286),
//! so it is written here and nowhere else. No `provenance.source` is ever set:
//! the importer reconciles only `source_kind = 'quick_ubu'` rows.
use crate::{
    errors::{AppError, Result},
    state::AppState,
};
use axum::http::StatusCode;
use serde_json::{json, Map, Value};
use ubu_core::{core::Objective, AuthoritySource, ObjectType, UbuId, VersionRef};
use ubu_store::{
    models::object_record::{NewObjectRecord, ObjectRecord},
    queries,
};

const EDITABLE_FIELDS: &[&str] = &[
    "title",
    "description",
    "priority",
    "status",
    "recurrence",
    "routine_instance_template",
];
/// Each routine object and the counter it carries, as in the importer.
const COUNTERS: [(&str, &str); 2] = [
    ("recurrence", "schedule_version"),
    ("routine_instance_template", "template_version"),
];
pub const TEMPLATE_NOTICE: &str = "The routine template changed. Occurrences already materialized keep the template they were created with; the change applies at the next materialize.";

pub struct Edited {
    pub objective_id: String,
    pub version: i64,
    pub template_changed: bool,
}

fn internal(error: impl std::fmt::Display) -> AppError {
    AppError::Internal(error.to_string())
}
fn reject(code: &str, message: impl Into<String>) -> AppError {
    AppError::bad_request_diagnostic(code, message)
}
fn present(payload: &Value, field: &str) -> bool {
    payload.get(field).is_some_and(|v| !v.is_null())
}
fn unknown(id: &str) -> AppError {
    AppError::Diagnostic {
        status: StatusCode::NOT_FOUND,
        code: "unknown_objective".into(),
        message: format!("Objective `{id}` does not exist"),
    }
}

/// The three authoring rejections, in their specified order.
fn validate(payload: &Value) -> Result<()> {
    if payload
        .get("title")
        .is_none_or(|title| title.is_null() || title.as_str() == Some(""))
    {
        return Err(reject(
            "objective_missing_title",
            "a nonempty title is required",
        ));
    }
    let (recurrence, template) = (
        present(payload, "recurrence"),
        present(payload, "routine_instance_template"),
    );
    if recurrence != template {
        return Err(reject(
            "objective_routine_fields_incomplete",
            "recurrence and routine_instance_template must be supplied together",
        ));
    }
    if recurrence && payload["mode"] != "evergreen" {
        return Err(reject(
            "objective_routine_requires_evergreen",
            "a routine must be an evergreen Objective; set mode to `evergreen`",
        ));
    }
    if payload
        .get("priority")
        .is_some_and(|p| !p.as_i64().is_some_and(|p| (0..=100).contains(&p)))
    {
        return Err(reject(
            "objective_invalid",
            "priority must be an integer from 0 to 100",
        ));
    }
    Ok(())
}

/// Exactly the importer's `normalize`: the core type decides what is admissible.
fn normalize(payload: Value) -> Result<Value> {
    let objective: Objective =
        serde_json::from_value(payload).map_err(|e| reject("objective_invalid", e.to_string()))?;
    serde_json::to_value(objective).map_err(internal)
}

/// Counters are server controlled; whatever the caller sent is overwritten.
fn set_counter(payload: &mut Value, field: &str, counter: &str, value: u64) -> Result<()> {
    if !present(payload, field) {
        return Ok(());
    }
    payload[field]
        .as_object_mut()
        .ok_or_else(|| reject("objective_invalid", format!("`{field}` must be an object")))?
        .insert(counter.into(), json!(value));
    Ok(())
}

/// Live routines are rows that are `active` with an `open` or `active` payload,
/// so a withdrawn Objective must leave `active` in the row as well.
fn row_status(payload: &Value) -> String {
    match payload["status"].as_str() {
        Some("open" | "active") | None => "active".into(),
        Some(other) => other.into(),
    }
}

pub async fn create(state: &AppState, fields: Map<String, Value>) -> Result<(String, i64)> {
    let _import = state.inner().quick_ubu_import_lock.lock().await;
    let id = UbuId::new(ObjectType::Objective);
    let now = state.planning_now();
    let mut payload = json!({"id":id,"status":"active","provenance":{"created_at":now,"authority_source":"user"}});
    let object = payload.as_object_mut().unwrap();
    for (key, value) in fields {
        if !value.is_null() {
            object.insert(key, value);
        }
    }
    validate(&payload)?;
    for (field, counter) in COUNTERS {
        set_counter(&mut payload, field, counter, 1)?;
    }
    let payload = normalize(payload)?;
    let envelope = state.envelope_for(
        [(id.clone(), VersionRef::Absent)].into_iter().collect(),
        AuthoritySource::User,
        now,
    )?;
    let row = queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        NewObjectRecord {
            id: id.to_string(),
            object_type: ObjectType::Objective.as_str().into(),
            version: 1,
            status: "active".into(),
            compartment_label: "user-capture".into(),
            payload,
            created_at: now.to_string(),
            updated_at: now.to_string(),
        },
    )
    .await?;
    Ok((row.id, row.version))
}

async fn current(state: &AppState, id: &str) -> Result<ObjectRecord> {
    UbuId::parse(id)
        .and_then(|parsed| parsed.require_object_type(ObjectType::Objective))
        .map_err(|_| unknown(id))?;
    queries::get_current_state(state.inner().store.pool(), id)
        .await?
        .filter(|row| row.object_type == ObjectType::Objective.as_str())
        .ok_or_else(|| unknown(id))
}

pub async fn edit(
    state: &AppState,
    id: &str,
    expected_version: i64,
    fields: Map<String, Value>,
) -> Result<Edited> {
    let _import = state.inner().quick_ubu_import_lock.lock().await;
    let row = current(state, id).await?;
    if expected_version != row.version {
        let message = format!(
            "Objective `{id}` expected version {expected_version}, current version {}",
            row.version
        );
        return Err(AppError::Diagnostics {
            status: StatusCode::CONFLICT,
            summary: message.clone(),
            items: vec![("version_conflict".into(), message)],
        });
    }
    let old: Value = serde_json::from_str(&row.payload_json).map_err(internal)?;
    let mut payload = old.clone();
    let object = payload
        .as_object_mut()
        .ok_or_else(|| internal("stored Objective is not an object"))?;
    for (key, value) in fields {
        if !EDITABLE_FIELDS.contains(&key.as_str()) {
            return Err(reject(
                "unsupported_objective_field",
                format!("field `{key}` is not editable"),
            ));
        }
        if value.is_null() {
            object.remove(&key);
        } else {
            object.insert(key, value);
        }
    }
    validate(&payload)?;
    // Normalize with the stored counters first, so a caller restating a default
    // (or a counter) is compared in canonical form and changes nothing.
    let stored = |field: &str, counter: &str| old[field][counter].as_u64().unwrap_or(1);
    for (field, counter) in COUNTERS {
        set_counter(&mut payload, field, counter, stored(field, counter))?;
    }
    let mut payload = normalize(payload)?;
    let mut template_changed = false;
    for (field, counter) in COUNTERS {
        let mut prior = old[field].clone();
        let mut next = payload[field].clone();
        if let (Some(prior), Some(next)) = (prior.as_object_mut(), next.as_object_mut()) {
            prior.remove(counter);
            next.remove(counter);
            if prior != next {
                let version = stored(field, counter)
                    .checked_add(1)
                    .ok_or_else(|| internal("routine version exhausted"))?;
                payload[field][counter] = json!(version);
                template_changed |= field == "routine_instance_template";
            }
        }
    }
    let payload = normalize(payload)?;
    let observed = u64::try_from(row.version).map_err(internal)?;
    let version = row.version.checked_add(1).ok_or_else(|| {
        AppError::Store(ubu_store::StoreError::ObjectVersionExhausted {
            object_id: id.into(),
            version: observed,
        })
    })?;
    let now = state.planning_now();
    let envelope = state.envelope_for(
        [(UbuId::parse(id)?, VersionRef::Version(observed))]
            .into_iter()
            .collect(),
        AuthoritySource::User,
        now,
    )?;
    let result = queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        NewObjectRecord {
            id: row.id,
            object_type: row.object_type,
            version,
            status: row_status(&payload),
            compartment_label: row.compartment_label,
            payload,
            created_at: row.created_at,
            updated_at: now.to_string(),
        },
    )
    .await
    .map_err(|error| match error {
        ubu_store::StoreError::PreconditionFailed { .. } => AppError::conflict_diagnostic(
            "version_conflict",
            format!("Objective `{id}` changed before admission"),
        ),
        other => AppError::Store(other),
    })?;
    Ok(Edited {
        objective_id: result.id,
        version: result.version,
        template_changed,
    })
}

pub async fn get(state: &AppState, id: &str) -> Result<(ObjectRecord, Value)> {
    let row = current(state, id).await?;
    let payload = serde_json::from_str(&row.payload_json).map_err(internal)?;
    Ok((row, payload))
}

pub async fn objectives(state: &AppState) -> Result<Vec<(ObjectRecord, Value)>> {
    let rows = sqlx::query_as::<_, ObjectRecord>(
        "SELECT * FROM objects WHERE object_type='Objective' ORDER BY created_at, id",
    )
    .fetch_all(state.inner().store.pool())
    .await
    .map_err(internal)?;
    rows.into_iter()
        .map(|row| {
            let payload = serde_json::from_str(&row.payload_json).map_err(internal)?;
            Ok((row, payload))
        })
        .collect()
}

pub fn is_routine(payload: &Value) -> bool {
    present(payload, "recurrence") && present(payload, "routine_instance_template")
}
