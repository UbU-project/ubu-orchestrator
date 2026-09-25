//! Calendar reconciliation I/O; classification and repair remain pure.
use std::collections::BTreeSet;

use crate::{errors::Result, services::calendar_projection::external_id};

/// All active Tasks contribute evidence, whether currently scheduled or not.
/// A derivable id does not confer ownership; only an applied record does that.
pub async fn known_external_ids(pool: &sqlx::SqlitePool) -> Result<BTreeSet<String>> {
    Ok(ubu_store::queries::query_active_tasks(pool)
        .await?
        .into_iter()
        .filter_map(|task| external_id(&task.id))
        .collect())
}
