//! Named configuration through ordinary Setting admission; never a Preference.
use crate::{
    category_palette::{valid_color_id, CategoryPalette, ALLOWED_COLOR_IDS},
    errors::{AppError, Result},
    state::AppState,
};
use serde_json::{json, Value};
use ubu_core::{AuthoritySource, ObjectType, UbuId, VersionRef};
use ubu_store::{
    models::object_record::{NewObjectRecord, ObjectRecord},
    queries,
};

fn internal(error: impl std::fmt::Display) -> AppError {
    AppError::Internal(error.to_string())
}

pub async fn settings(pool: &sqlx::SqlitePool) -> Result<Vec<ObjectRecord>> {
    sqlx::query_as("SELECT * FROM objects WHERE object_type='Setting' ORDER BY id")
        .fetch_all(pool)
        .await
        .map_err(internal)
}

fn validate_name(name: &str) -> Result<()> {
    if !name
        .strip_prefix("calendar.color.")
        .is_some_and(|category| !category.trim().is_empty())
    {
        return Err(AppError::bad_request_diagnostic(
            "setting_unknown_name",
            "Only calendar.color.<category> Settings can be authored",
        ));
    }
    Ok(())
}

async fn current(state: &AppState, name: &str) -> Result<Option<ObjectRecord>> {
    let rows: Vec<ObjectRecord> = sqlx::query_as("SELECT * FROM objects WHERE object_type='Setting' AND json_extract(payload_json,'$.name')=? ORDER BY id")
        .bind(name).fetch_all(state.inner().store.pool()).await.map_err(internal)?;
    if rows.len() > 1 {
        return Err(AppError::conflict_diagnostic(
            "setting_duplicate_name",
            format!(
                "Multiple Settings are named `{name}`; resolve duplicate records before editing"
            ),
        ));
    }
    Ok(rows.into_iter().next())
}

pub async fn put(state: &AppState, name: &str, value: Value) -> Result<(String, i64)> {
    validate_name(name)?;
    if !value.as_str().is_some_and(valid_color_id) {
        return Err(AppError::bad_request_diagnostic(
            "setting_invalid_color",
            format!(
                "Colour must be a string with one of the allowed ids: {}",
                ALLOWED_COLOR_IDS.join(", ")
            ),
        ));
    }
    let _import = state.inner().quick_ubu_import_lock.lock().await;
    let _action = state.inner().task_action_lock.lock().await;
    let old = current(state, name).await?;
    let now = state.planning_now();
    let id = match &old {
        Some(row) => UbuId::parse(&row.id)?,
        None => UbuId::new(ObjectType::Setting),
    };
    let version = match &old {
        Some(row) => row
            .version
            .checked_add(1)
            .ok_or_else(|| internal("Setting version exhausted"))?,
        None => 1,
    };
    let observed = match &old {
        Some(row) => VersionRef::Version(u64::try_from(row.version).map_err(internal)?),
        None => VersionRef::Absent,
    };
    let created_at = old
        .as_ref()
        .map(|row| row.created_at.clone())
        .unwrap_or_else(|| now.to_string());
    let envelope = state.envelope_for(
        [(id.clone(), observed)].into_iter().collect(),
        AuthoritySource::User,
        now,
    )?;
    let row = queries::admit_object(state.inner().store.pool(), &envelope, NewObjectRecord {
        id: id.to_string(), object_type: "Setting".into(), version, status: "active".into(),
        compartment_label: old.map(|row| row.compartment_label).unwrap_or_else(|| "user-capture".into()),
        payload: json!({"id":id,"name":name,"value":value,"authority_source":"user","provenance":{"created_at":created_at,"authority_source":"user"}}),
        created_at, updated_at: now.to_string(),
    }).await?;
    Ok((row.id, row.version))
}

pub async fn delete(state: &AppState, name: &str) -> Result<()> {
    validate_name(name)?;
    let _import = state.inner().quick_ubu_import_lock.lock().await;
    let _action = state.inner().task_action_lock.lock().await;
    let row = current(state, name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Setting `{name}` does not exist")))?;
    // Like Preference withdrawal, remove the canonical row and retain the mutation ledger.
    sqlx::query("DELETE FROM objects WHERE id=? AND object_type='Setting'")
        .bind(row.id)
        .execute(state.inner().store.pool())
        .await
        .map_err(internal)?;
    Ok(())
}

pub async fn list(state: &AppState) -> Result<crate::api::setting::SettingsResponse> {
    let rows = settings(state.inner().store.pool()).await?;
    let palette = CategoryPalette::from_layers(state.inner().store.pool(), &rows).await?;
    let settings = rows
        .into_iter()
        .map(|row| {
            let payload: Value = serde_json::from_str(&row.payload_json).map_err(internal)?;
            let setting: ubu_core::core::Setting =
                serde_json::from_value(payload.clone()).map_err(internal)?;
            Ok(crate::api::setting::SettingSummary {
                id: row.id,
                name: setting.name,
                value: setting.value,
                authority_source: payload["authority_source"]
                    .as_str()
                    .unwrap_or_default()
                    .into(),
                version: row.version,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(crate::api::setting::SettingsResponse {
        schema_version: crate::api::setting::SETTING_SCHEMA_VERSION.into(),
        settings,
        palette: palette.entries(),
        inverse: palette.inverse_entries(),
    })
}
