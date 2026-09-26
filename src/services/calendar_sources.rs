//! Persisted source identity, shared by capture and projection.
use crate::errors::{AppError, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use ubu_store::models::object_record::ObjectRecord;

pub async fn by_source(pool: &sqlx::SqlitePool) -> Result<BTreeMap<String, (ObjectRecord, Value)>> {
    let rows = sqlx::query_as::<_, ObjectRecord>("SELECT * FROM objects WHERE object_type='Task' AND json_extract(payload_json, '$.provenance.source.source_kind')='google_calendar' ORDER BY id")
        .fetch_all(pool).await.map_err(|e| AppError::Internal(e.to_string()))?;
    let mut sources = BTreeMap::new();
    for row in rows {
        let payload: Value = serde_json::from_str(&row.payload_json)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let source = payload["provenance"]["source"]["source_id"]
            .as_str()
            .ok_or_else(|| AppError::Internal("Calendar source has no source_id".into()))?
            .to_owned();
        if sources.insert(source, (row, payload)).is_some() {
            return Err(AppError::Internal(
                "duplicate Calendar source identity".into(),
            ));
        }
    }
    Ok(sources)
}

pub async fn origins_by_task(pool: &sqlx::SqlitePool) -> Result<BTreeMap<String, String>> {
    Ok(by_source(pool)
        .await?
        .into_iter()
        .map(|(source, (row, _))| (row.id, source))
        .collect())
}
