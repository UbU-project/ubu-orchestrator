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

pub const REVIEW_SEED: &str = "advisory.review_seed_days";
pub const REVIEW_CEILING: &str = "advisory.review_ceiling_days";

pub const ADVISORY_TIMEOUT: &str = "advisory.timeout_ms";
pub const DEFAULT_ADVISORY_TIMEOUT_MS: u64 = 120_000;
/// A zero would fail every run at once; no ceiling would let one run hold the advisory path.
pub const MIN_ADVISORY_TIMEOUT_MS: u64 = 5_000;
pub const MAX_ADVISORY_TIMEOUT_MS: u64 = 3_600_000;

/// An integer number of milliseconds within the bounds; a float or a string is refused.
fn valid_advisory_timeout(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .filter(|ms| (MIN_ADVISORY_TIMEOUT_MS..=MAX_ADVISORY_TIMEOUT_MS).contains(ms))
}

fn validate_name(name: &str) -> Result<()> {
    if let Some(root) = name.strip_prefix(super::subject_vocabulary::PREFIX) { return super::subject_vocabulary::validate_root(root); }
    if matches!(name, "advisory.model" | "advisory.endpoint" | ADVISORY_TIMEOUT | REVIEW_SEED | REVIEW_CEILING) { return Ok(()); }
    if !name
        .strip_prefix("calendar.color.")
        .is_some_and(|category| !category.trim().is_empty())
    {
        return Err(AppError::bad_request_diagnostic(
            "setting_unknown_name",
            "Only calendar.color.<category>, universe.subject.<root> and supported advisory model, endpoint, timeout and review interval Settings can be authored",
        ));
    }
    Ok(())
}

pub async fn current(state: &AppState, name: &str) -> Result<Option<ObjectRecord>> {
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

/// Accept only a literal loopback HTTP origin with an explicit nonzero port.
pub fn valid_advisory_endpoint(value: &str) -> bool {
    value.strip_prefix("http://127.0.0.1:").is_some_and(|port| {
        !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit())
            && port.parse::<u16>().is_ok_and(|port| port != 0)
    })
}

pub async fn advisory_value(state: &AppState, name: &str) -> Result<Option<String>> {
    current(state, name).await?.map(|row| {
        let payload: Value = serde_json::from_str(&row.payload_json).map_err(internal)?;
        Ok(payload["value"].as_str().filter(|s| !s.trim().is_empty()).map(str::to_owned))
    }).transpose().map(Option::flatten)
}

/// The budget for one advisory run, and whether a Setting supplied it.
pub async fn advisory_timeout_ms(state: &AppState) -> Result<(u64, bool)> {
    let Some(row) = current(state, ADVISORY_TIMEOUT).await? else {
        return Ok((DEFAULT_ADVISORY_TIMEOUT_MS, false));
    };
    let payload: Value = serde_json::from_str(&row.payload_json).map_err(internal)?;
    // Only a validated value is ever admitted; anything else is treated as absent.
    Ok(valid_advisory_timeout(&payload["value"])
        .map_or((DEFAULT_ADVISORY_TIMEOUT_MS, false), |ms| (ms, true)))
}

pub async fn put(state: &AppState, name: &str, value: Value) -> Result<(String, i64)> {
    validate_name(name)?;
    if name.starts_with(super::subject_vocabulary::PREFIX) {
        if value != true {
            return Err(AppError::bad_request_diagnostic("subject_invalid_value", "A provisional subject Setting must have the boolean value true; retire it with DELETE."));
        }
    } else if name == ADVISORY_TIMEOUT {
        if valid_advisory_timeout(&value).is_none() {
            return Err(AppError::bad_request_diagnostic(
                "setting_invalid_advisory_timeout",
                format!("advisory.timeout_ms must be an integer number of milliseconds from {MIN_ADVISORY_TIMEOUT_MS} to {MAX_ADVISORY_TIMEOUT_MS}"),
            ));
        }
    } else if matches!(name, REVIEW_SEED | REVIEW_CEILING) {
        if value.as_u64().is_none_or(|days| !(1..=365).contains(&days)) {
            return Err(AppError::bad_request_diagnostic("setting_invalid_review_interval", "Review intervals must be whole days from 1 to 365"));
        }
    } else if name.starts_with("advisory.") {
        if !value.as_str().is_some_and(|value| !value.trim().is_empty()) {
            return Err(AppError::bad_request_diagnostic("setting_invalid_advisory", "Advisory Settings must be non-empty strings"));
        }
        if name == "advisory.endpoint" && !valid_advisory_endpoint(value.as_str().unwrap()) {
            return Err(AppError::bad_request_diagnostic("setting_invalid_advisory_endpoint", "advisory.endpoint must be http://127.0.0.1:<port>, with no path, credentials, query or fragment"));
        }
    } else if !value.as_str().is_some_and(valid_color_id) {
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
    check_review_pair(state, name, value.as_u64()).await?;
    let old = current(state, name).await?;
    if let Some(root) = name.strip_prefix(super::subject_vocabulary::PREFIX) {
        if old.is_some() { return Err(AppError::bad_request_diagnostic("subject_already_registered", format!("Subject `{root}` is already provisional; choose it from the subject list."))); }
    }
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
    check_review_pair(state, name, None).await?;
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
    let mut advisory = Vec::new();
    for name in ["advisory.model", "advisory.endpoint"] {
        let value = advisory_value(state, name).await?;
        advisory.push(crate::api::setting::AdvisorySettingEntry {
            name: name.into(), origin: if value.is_some() { "setting" } else { "unconfigured" }.into(), value,
        });
    }
    let (timeout_ms, configured) = advisory_timeout_ms(state).await?;
    advisory.push(crate::api::setting::AdvisorySettingEntry {
        name: ADVISORY_TIMEOUT.into(),
        value: Some(timeout_ms.to_string()),
        origin: if configured { "setting" } else { "default" }.into(),
    });
    for name in [REVIEW_SEED, REVIEW_CEILING] {
        let (days, configured) = review_days(state, name).await?;
        advisory.push(crate::api::setting::AdvisorySettingEntry { name:name.into(),value:Some(days.to_string()),origin:if configured {"setting"} else {"default"}.into() });
    }
    Ok(crate::api::setting::SettingsResponse {
        schema_version: crate::api::setting::SETTING_SCHEMA_VERSION.into(),
        settings,
        palette: palette.entries(),
        inverse: palette.inverse_entries(),
        advisory,
    })
}

/// The ceiling cannot exceed a year: configuration cannot turn a snooze into silence.
pub async fn review_days(state: &AppState, name: &str) -> Result<(u64, bool)> {
    let default = if name == REVIEW_SEED {7} else {365};
    let Some(row) = current(state,name).await? else { return Ok((default,false)); };
    let payload:Value=serde_json::from_str(&row.payload_json).map_err(internal)?;
    Ok(payload["value"].as_u64().filter(|n|(1..=365).contains(n)).map_or((default,false),|n|(n,true)))
}
async fn check_review_pair(state: &AppState, name: &str, proposed: Option<u64>) -> Result<()> {
    if !matches!(name, REVIEW_SEED|REVIEW_CEILING) {return Ok(());}
    let seed = if name == REVIEW_SEED {proposed.unwrap_or(7)} else {review_days(state,REVIEW_SEED).await?.0};
    let ceiling = if name == REVIEW_CEILING {proposed.unwrap_or(365)} else {review_days(state,REVIEW_CEILING).await?.0};
    if seed > ceiling {return Err(AppError::bad_request_diagnostic("setting_invalid_review_interval", "The review seed must not exceed its ceiling"));}
    Ok(())
}
