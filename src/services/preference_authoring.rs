//! Native pairwise authoring. Only enabled statements constrain admission.
use crate::{
    errors::{AppError, Result},
    state::AppState,
};
use serde_json::{json, Value};
use sqlx::Row;
use ubu_core::{
    core::{Preference, PreferenceOrder, PreferenceSubjects},
    AuthoritySource, ObjectType, UbuId, VersionRef,
};
use ubu_store::{
    models::object_record::{NewObjectRecord, ObjectRecord},
    queries,
};

fn internal(error: impl std::fmt::Display) -> AppError {
    AppError::Internal(error.to_string())
}
fn reject(code: &str, message: impl Into<String>) -> AppError {
    AppError::bad_request_diagnostic(code, message)
}

/// Preserve the specified semantic rejection order, including mixed API pairs.
pub async fn validate_subjects(
    state: &AppState,
    a: Option<&str>,
    b: Option<&str>,
    objectives: bool,
) -> Result<()> {
    if a.is_some() && a == b {
        return Err(reject(
            "preference_self_reference",
            "A Preference cannot compare a Task with itself",
        ));
    }
    for id in [a, b].into_iter().flatten() {
        let row = queries::get_current_state(state.inner().store.pool(), id).await?;
        if row.is_none_or(|row| row.object_type != "Task" || row.status != "active") {
            return Err(reject(
                "preference_unknown_task",
                format!("Task `{id}` is unknown or inactive"),
            ));
        }
    }
    if objectives {
        return Err(reject(
            "preference_objective_pair_unsupported",
            "Planning does not yet consume Objective-level Preferences; supply task_a and task_b",
        ));
    }
    if a.is_none() || b.is_none() {
        return Err(reject(
            "preference_unknown_task",
            "Both task_a and task_b are required",
        ));
    }
    Ok(())
}

pub async fn preferences(state: &AppState) -> Result<Vec<(ObjectRecord, Preference)>> {
    let rows = sqlx::query_as::<_, ObjectRecord>(
        "SELECT * FROM objects WHERE object_type='Preference' ORDER BY id",
    )
    .fetch_all(state.inner().store.pool())
    .await
    .map_err(internal)?;
    rows.into_iter()
        .map(|row| {
            let preference = serde_json::from_str(&row.payload_json).map_err(internal)?;
            Ok((row, preference))
        })
        .collect()
}

async fn validate_graph(state: &AppState, proposed: &Preference) -> Result<()> {
    let PreferenceSubjects::Tasks { a, b } = &proposed.subjects else {
        return Err(reject(
            "preference_objective_pair_unsupported",
            "Planning does not yet consume Objective-level Preferences",
        ));
    };
    validate_subjects(state, Some(a.as_str()), Some(b.as_str()), false).await?;
    let existing: Vec<_> = preferences(state)
        .await?
        .into_iter()
        .filter(|(row, p)| row.status == "active" && p.enabled && p.id != proposed.id)
        .map(|(_, p)| p)
        .collect();
    let same_pair = |p: &&Preference| matches!(&p.subjects, PreferenceSubjects::Tasks {a:c,b:d} if (a==c && b==d)||(a==d && b==c));
    let equivalent = |p: &&Preference| {
        p.order == proposed.order
            && (p.order == PreferenceOrder::AIndifferentToB
                || matches!(&p.subjects, PreferenceSubjects::Tasks {a:c,..} if a==c))
    };
    if let Some(p) = existing.iter().filter(same_pair).find(equivalent) {
        return Err(reject(
            "preference_duplicate_pair",
            format!("Preference `{}` already states this pair and order", p.id),
        ));
    }
    if let Some(p) = existing.iter().find(same_pair) {
        return Err(reject(
            "preference_contradiction",
            format!(
                "Preference `{}` contradicts this pair; disable or delete it first",
                p.id
            ),
        ));
    }
    let mut graph = existing;
    graph.push(proposed.clone());
    // Include every Task subject, even currently ineligible imported subjects:
    // write-time consistency concerns enabled statements, not today's horizon.
    let ids: Vec<_> = graph
        .iter()
        .flat_map(|p| match &p.subjects {
            PreferenceSubjects::Tasks { a, b } => vec![a.to_string(), b.to_string()],
            _ => Vec::new(),
        })
        .collect();
    let layered = super::task_priority::layer_preferences(&ids, &graph);
    if let Some(members) = layered.cycles.first() {
        let ordered = super::task_priority::cycle_witness(members, &graph);
        return Err(reject("preference_cycle_rejected", format!("Preference cycle among Tasks [{}]; disable or delete a conflicting Preference first", ordered.join(" -> "))));
    }
    Ok(())
}

pub async fn create(
    state: &AppState,
    task_a: &str,
    task_b: &str,
    order: PreferenceOrder,
) -> Result<(String, i64)> {
    let _import = state.inner().quick_ubu_import_lock.lock().await;
    let _action = state.inner().task_action_lock.lock().await;
    validate_subjects(state, Some(task_a), Some(task_b), false).await?;
    let id = UbuId::new(ObjectType::Preference);
    let now = state.planning_now();
    let payload = json!({"id":id,"task_a":task_a,"task_b":task_b,"order":order,"acquired_method":"user_defined","acquired_date":now,"enabled":true,"provenance":{"created_at":now,"authority_source":"user"}});
    let preference: Preference = serde_json::from_value(payload.clone()).map_err(internal)?;
    validate_graph(state, &preference).await?;
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
            object_type: "Preference".into(),
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
    queries::get_current_state(state.inner().store.pool(), id)
        .await?
        .filter(|row| row.object_type == "Preference")
        .ok_or_else(|| AppError::NotFound(format!("Preference `{id}` does not exist")))
}

pub async fn set_enabled(
    state: &AppState,
    id: &str,
    enabled: bool,
    expected_version: i64,
) -> Result<(String, i64)> {
    let _import = state.inner().quick_ubu_import_lock.lock().await;
    let _action = state.inner().task_action_lock.lock().await;
    let row = current(state, id).await?;
    if row.version != expected_version {
        return Err(AppError::conflict_diagnostic(
            "version_conflict",
            format!(
                "Preference `{id}` expected version {expected_version}, current version {}",
                row.version
            ),
        ));
    }
    let mut payload: Value = serde_json::from_str(&row.payload_json).map_err(internal)?;
    payload["enabled"] = enabled.into();
    let preference: Preference = serde_json::from_value(payload.clone()).map_err(internal)?;
    if enabled {
        validate_graph(state, &preference).await?;
    }
    let observed = u64::try_from(row.version).map_err(internal)?;
    let version = row.version.checked_add(1).ok_or_else(|| {
        AppError::Store(ubu_store::StoreError::ObjectVersionExhausted {
            object_id: id.into(),
            version: observed,
        })
    })?;
    let now = state.planning_now();
    let envelope = state.envelope_for(
        [(preference.id, VersionRef::Version(observed))]
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
            status: row.status,
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
            format!("Preference `{id}` changed before admission"),
        ),
        other => AppError::Store(other),
    })?;
    Ok((result.id, result.version))
}

/// This ticket explicitly withdraws the canonical row, rather than tombstoning
/// it. The store exposes no delete writer; restrict this SQL to Preferences and
/// leave the Device mutation ledger intact (it is not object history).
pub async fn delete(state: &AppState, id: &str) -> Result<()> {
    let _import = state.inner().quick_ubu_import_lock.lock().await;
    let _action = state.inner().task_action_lock.lock().await;
    current(state, id).await?;
    let result = sqlx::query("DELETE FROM objects WHERE id=? AND object_type='Preference'")
        .bind(id)
        .execute(state.inner().store.pool())
        .await
        .map_err(internal)?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "Preference `{id}` does not exist"
        )));
    }
    Ok(())
}

/// Titles are read afresh; dangling and imported Objective subjects stay visible.
pub async fn title(state: &AppState, id: &str) -> Result<Option<String>> {
    let row = sqlx::query("SELECT payload_json FROM objects WHERE id=?")
        .bind(id)
        .fetch_optional(state.inner().store.pool())
        .await
        .map_err(internal)?;
    row.map(|row| {
        let raw: String = row.try_get("payload_json").map_err(internal)?;
        let payload: Value = serde_json::from_str(&raw).map_err(internal)?;
        Ok(payload["title"].as_str().map(str::to_owned))
    })
    .transpose()
    .map(Option::flatten)
}
