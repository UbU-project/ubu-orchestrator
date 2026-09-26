//! Rolling occurrence cache. Import and materialization share a fixed lock order;
//! user actions remain concurrent and are protected by store version preconditions.
use super::routine_instantiation::{diagnostic, instantiate, Occurrence, RoutineDefinition};
use crate::{
    api::planning::{DiagnosticBody, TimeWindowBody},
    errors::{AppError, Result},
    planning_time::timestamp_at,
    state::AppState,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use ubu_core::{
    core::{Objective, RoutinePlacement, Task},
    AuthoritySource, ObjectType, UbuId, UbuTimestamp, VersionRef,
};
use ubu_store::{
    models::{
        log_record::NewLogRecord,
        object_record::{NewObjectRecord, ObjectRecord},
    },
    queries,
};

#[derive(Default)]
pub struct RoutineContext {
    pub diagnostics: Vec<DiagnosticBody>,
    pub realized_floors: HashMap<String, u64>,
    /// Assigned, not intersected with nominal ceilings; already clamped to the declared range.
    pub realized_ceilings: HashMap<String, u64>,
}
fn internal(e: impl std::fmt::Display) -> AppError {
    AppError::Internal(e.to_string())
}
pub(crate) fn seconds(value: &Value) -> Option<u64> {
    value
        .as_str()
        .and_then(|v| UbuTimestamp::parse(v).ok())
        .and_then(|v| u64::try_from(v.inner().unix_timestamp()).ok())
}
pub(crate) fn window(payload: &Value) -> Option<(u64, u64)> {
    if let Some(w) = payload.get("static_window") {
        Some((seconds(&w["start"])?, seconds(&w["end"])?))
    } else {
        let w = payload.get("allowed_time_range")?;
        Some((
            seconds(&w["earliest_start"])?,
            seconds(&w["latest_finish"])?,
        ))
    }
}
pub(crate) fn live(row: &ObjectRecord, payload: &Value) -> bool {
    row.status == "active"
        && matches!(payload["status"].as_str(), Some("open" | "active"))
        && payload.get("recurrence").is_some_and(|v| !v.is_null())
        && payload
            .get("routine_instance_template")
            .is_some_and(|v| !v.is_null())
}
#[derive(Clone)]
struct Stored {
    row: ObjectRecord,
    payload: Value,
}
impl Stored {
    fn parse(row: ObjectRecord) -> Result<Self> {
        let payload = serde_json::from_str(&row.payload_json).map_err(internal)?;
        Ok(Self { row, payload })
    }
    fn key(&self) -> &str {
        self.payload["occurrence"]["key"].as_str().unwrap_or("")
    }
    fn day(&self) -> (String, String) {
        (
            self.payload["occurrence"]["routine_objective_id"]
                .as_str()
                .unwrap_or("")
                .into(),
            self.payload["occurrence"]["local_date"]
                .as_str()
                .unwrap_or("")
                .into(),
        )
    }
    fn superseded(&self) -> bool {
        self.row.status == "moot" && self.payload["moot_reason_code"] == "superseded"
    }
}
async fn write(
    state: &AppState,
    id: &str,
    old: Option<&Stored>,
    label: &str,
    payload: Value,
    now: UbuTimestamp,
) -> Result<Stored> {
    let task: Task = serde_json::from_value(payload).map_err(internal)?;
    task.validate()?;
    let payload = serde_json::to_value(task).map_err(internal)?;
    let envelope = state.envelope_for(
        [(
            UbuId::parse(id)?,
            old.map_or(VersionRef::Absent, |r| {
                VersionRef::Version(r.row.version as u64)
            }),
        )]
        .into_iter()
        .collect(),
        AuthoritySource::System,
        now,
    )?;
    let row = queries::admit_object(
        state.inner().store.pool(),
        &envelope,
        NewObjectRecord {
            id: id.into(),
            object_type: ObjectType::Task.as_str().into(),
            version: old.map_or(1, |r| r.row.version),
            status: payload["status"].as_str().unwrap().into(),
            compartment_label: label.into(),
            payload,
            created_at: old.map_or_else(|| now.to_string(), |r| r.row.created_at.clone()),
            updated_at: now.to_string(),
        },
    )
    .await?;
    Stored::parse(row)
}
async fn outcome(
    state: &AppState,
    old: &Stored,
    status: &str,
    reason: Option<&str>,
    event: &str,
    result: &str,
    now: UbuTimestamp,
) -> Result<Stored> {
    let mut payload = old.payload.clone();
    payload["status"] = json!(status);
    if let Some(reason) = reason {
        payload["moot_reason_code"] = json!(reason);
    }
    let stored = write(
        state,
        &old.row.id,
        Some(old),
        &old.row.compartment_label,
        payload,
        now,
    )
    .await?;
    let envelope = state.envelope_for(BTreeMap::new(), AuthoritySource::System, now)?;
    queries::append_log_entry(
        state.inner().store.pool(),
        &envelope,
        NewLogRecord {
            id: UbuId::new(ObjectType::LogEntry).to_string(),
            event_type: event.into(),
            object_refs: json!([old.row.id]),
            payload: json!({"routine_outcome":result,"occurrence_key":old.key()}),
            provenance: json!({"created_at":now,"authority_source":"system"}),
            created_at: now.to_string(),
        },
    )
    .await?;
    Ok(stored)
}
fn failure(context: &mut RoutineContext, id: &str, date: &str, e: impl std::fmt::Display) {
    context.diagnostics.push(diagnostic(
        "routine_occurrence_write_failed",
        format!("Routine occurrence `{id}` on {date} could not be written: {e}"),
    ));
}
fn canonical(payload: &Value) -> Value {
    let mut p = payload.clone();
    if let Some(p) = p.as_object_mut() {
        p.remove("provenance");
        p.remove("status");
    }
    p
}
fn payload(
    o: &Occurrence,
    id: &str,
    days: &BTreeMap<(String, String), String>,
    now: UbuTimestamp,
) -> Result<Value> {
    let mut p = json!({"id":id,"title":o.template.title,"status":"active","objective_id":o.objective_id,"duration_estimate":o.template.duration_estimate,"occurrence":{"routine_objective_id":o.objective_id,"local_date":o.local_date,"key":o.key},"provenance":{"created_at":now,"authority_source":"system","source":{"source_kind":"routine_instantiation","source_id":o.key}}});
    if !o.template.tags.is_empty() {
        p["tags"] = json!(o.template.tags);
    }
    if let Some(category) = &o.template.category_tag {
        if !category.is_empty() {
            p["category_tag"] = json!(category);
        }
    }
    if !o.template.occupies_capacity {
        p["occupies_capacity"] = json!(false);
    }
    if let Some(effects) = &o.template.effects {
        p["effects"] = json!(effects);
    }
    if let Some(preconditions) = &o.template.preconditions {
        p["preconditions"] = json!(preconditions);
    }
    let edges: BTreeSet<_> = o
        .after
        .iter()
        .filter_map(|(id, _, _)| days.get(&(id.to_string(), o.local_date.clone())).cloned())
        .collect();
    if !edges.is_empty() {
        p["blocked_by"] = json!(edges);
    }
    if o.overridden || o.template.placement == RoutinePlacement::Static {
        p["static_window"] = json!({"start":timestamp_at(o.start)?,"end":timestamp_at(o.end)?});
    } else {
        p["allowed_time_range"] =
            json!({"earliest_start":timestamp_at(o.start)?,"latest_finish":timestamp_at(o.end)?});
    }
    let task: Task = serde_json::from_value(p).map_err(internal)?;
    task.validate()?;
    serde_json::to_value(task).map_err(internal)
}
pub(crate) struct LiveRoutines {
    pub definitions: Vec<RoutineDefinition>,
    pub labels: BTreeMap<String, String>,
    pub unevaluable: HashSet<String>,
    pub diagnostics: Vec<DiagnosticBody>,
    pub titles: BTreeMap<String, String>,
}

/// Callers hold their own locks; acquiring the import lock here would deadlock.
pub(crate) async fn live_definitions(pool: &sqlx::SqlitePool) -> Result<LiveRoutines> {
    let rows=sqlx::query_as::<_,ObjectRecord>("SELECT * FROM objects WHERE object_type='Objective' AND json_extract(payload_json,'$.recurrence') IS NOT NULL").fetch_all(pool).await.map_err(internal)?;
    let mut definitions = Vec::new();
    let mut titles = BTreeMap::new();
    let mut diagnostics = Vec::new();
    let mut labels = BTreeMap::new();
    let mut unevaluable = HashSet::new();
    for row in rows {
        let value: Value = serde_json::from_str(&row.payload_json).map_err(internal)?;
        if row.status != "active"
            || matches!(value["status"].as_str(), Some("satisfied" | "abandoned"))
            || value
                .get("routine_instance_template")
                .is_none_or(Value::is_null)
        {
            continue;
        }
        match serde_json::from_value::<Objective>(value.clone()) {
            Ok(objective) if live(&row, &value) => {
                if let (Some(schedule), Some(template)) =
                    (objective.recurrence, objective.routine_instance_template)
                {
                    if schedule.timezone.parse::<chrono_tz::Tz>().is_err() {
                        unevaluable.insert(row.id.clone());
                    }
                    titles.insert(row.id.clone(), objective.title.clone());
                    labels.insert(row.id.clone(), row.compartment_label);
                    definitions.push(RoutineDefinition {
                        objective_id: objective.id,
                        schedule,
                        template,
                    });
                }
            }
            Ok(_) => {}
            Err(e) => {
                unevaluable.insert(row.id.clone());
                diagnostics.push(diagnostic(
                    "routine_objective_invalid",
                    format!("Routine Objective `{}` is invalid: {e}", row.id),
                ));
            }
        }
    }
    Ok(LiveRoutines {
        definitions,
        labels,
        unevaluable,
        diagnostics,
        titles,
    })
}

pub async fn materialize(
    state: &AppState,
    horizon: &TimeWindowBody,
    now: u64,
) -> Result<RoutineContext> {
    let _import = state.inner().quick_ubu_import_lock.lock().await;
    let _materialize = state.inner().routine_materialization_lock.lock().await;
    let pool = state.inner().store.pool();
    let instant = UbuTimestamp::parse(&timestamp_at(now)?)?;
    let mut context = RoutineContext::default();
    let LiveRoutines {
        definitions,
        labels,
        unevaluable,
        diagnostics,
        ..
    } = live_definitions(pool).await?;
    context.diagnostics = diagnostics;
    let active=sqlx::query_as::<_,ObjectRecord>("SELECT * FROM objects WHERE object_type='Task' AND status='active' AND json_extract(payload_json,'$.occurrence') IS NOT NULL").fetch_all(pool).await.map_err(internal)?;
    // One evidence query across active occurrences; materializer and recalculation Logs do not protect edits.
    let evidence:HashSet<String>=sqlx::query_scalar::<_,String>("SELECT DISTINCT o.id FROM objects o JOIN logs l JOIN json_each(l.object_refs_json) refs ON refs.value=o.id WHERE o.object_type='Task' AND o.status='active' AND json_extract(o.payload_json,'$.occurrence') IS NOT NULL AND l.event_type!='recalculation_requested' AND json_type(l.payload_json,'$.routine_outcome') IS NULL").fetch_all(pool).await.map_err(internal)?.into_iter().collect();
    let mut stored = BTreeMap::new();
    for row in active {
        let mut r = Stored::parse(row)?;
        if window(&r.payload).is_some_and(|(_, end)| end <= now) {
            match outcome(state, &r, "failed", None, "task_failed", "missed", instant).await {
                Ok(next) => r = next,
                Err(e) => failure(&mut context, &r.row.id, &r.day().1, e),
            }
        }
        stored.insert(r.key().to_owned(), r);
    }
    let derived = instantiate(&definitions, horizon.start, horizon.end);
    context.diagnostics.extend(derived.diagnostics);
    // Only history in the evaluated per-Objective dates is loaded, never the entire occurrence ledger.
    let ranges: Vec<_> = derived
        .dates
        .iter()
        .map(|(id, (start, end))| json!({"id":id,"start":start,"end":end}))
        .collect();
    let history=sqlx::query_as::<_,ObjectRecord>("SELECT o.* FROM objects o WHERE o.object_type='Task' AND json_extract(o.payload_json,'$.occurrence') IS NOT NULL AND EXISTS (SELECT 1 FROM json_each(?) d WHERE json_extract(o.payload_json,'$.occurrence.routine_objective_id')=json_extract(d.value,'$.id') AND json_extract(o.payload_json,'$.occurrence.local_date') BETWEEN json_extract(d.value,'$.start') AND json_extract(d.value,'$.end'))").bind(json!(ranges).to_string()).fetch_all(pool).await.map_err(internal)?;
    for row in history {
        let r = Stored::parse(row)?;
        stored.insert(r.key().to_owned(), r);
    }
    let keys: HashSet<_> = derived.occurrences.iter().map(|o| o.key.as_str()).collect();
    for r in stored.values_mut() {
        if r.row.status != "active" || keys.contains(r.key()) {
            continue;
        }
        let (objective, date) = r.day();
        if unevaluable.contains(&objective) {
            continue;
        }
        if let Some((start, end)) = derived.dates.get(&objective) {
            if &date < start || &date > end {
                continue;
            }
        }
        if evidence.contains(&r.row.id) {
            context.diagnostics.push(diagnostic("routine_occurrence_edit_conflict",format!("Routine occurrence `{}` for Objective `{objective}` on {date} has execution evidence and conflicts with the current definition",r.row.id)));
            continue;
        }
        match outcome(
            state,
            r,
            "moot",
            Some("superseded"),
            "task_moot",
            "superseded",
            instant,
        )
        .await
        {
            Ok(next) => *r = next,
            Err(e) => failure(&mut context, &r.row.id, &date, e),
        }
    }
    let mut days = BTreeMap::new();
    for r in stored.values().filter(|r| !r.superseded()) {
        days.entry(r.day()).or_insert_with(|| r.row.id.clone());
    }
    let mut ids = BTreeMap::new();
    let mut creatable = HashSet::new();
    for o in &derived.occurrences {
        let day = (o.objective_id.to_string(), o.local_date.clone());
        let held = stored
            .values()
            .any(|r| r.day() == day && r.key() != o.key && !r.superseded());
        let create = !held && o.start < horizon.end && o.end > horizon.start.max(now);
        if create {
            creatable.insert(o.key.clone());
        }
        if let Some(old) = stored.get(&o.key) {
            if !old.superseded() || create {
                ids.insert(o.key.clone(), old.row.id.clone());
            }
        } else if create {
            ids.insert(o.key.clone(), UbuId::new(ObjectType::Task).to_string());
        }
        if let Some(id) = ids.get(&o.key) {
            days.entry(day).or_insert_with(|| id.clone());
        }
    }
    for o in &derived.occurrences {
        let Some(id) = ids.get(&o.key).cloned() else {
            continue;
        };
        let old = stored.get(&o.key);
        let refresh = old.is_some_and(|r| {
            r.row.status == "active"
                && !evidence.contains(&r.row.id)
                && window(&r.payload).is_some_and(|(_, end)| end > now)
        });
        let create_or_revive = creatable.contains(&o.key) && old.is_none_or(Stored::superseded);
        if !refresh && !create_or_revive {
            continue;
        }
        let next = async {
            let mut p = payload(o, &id, &days, instant)?;
            if let Some(old) = old {
                p["provenance"] = old.payload["provenance"].clone();
                if refresh && canonical(&p) == canonical(&old.payload) {
                    return Ok(None);
                }
            }
            write(
                state,
                &id,
                old,
                &labels[&o.objective_id.to_string()],
                p,
                instant,
            )
            .await
            .map(Some)
        }
        .await;
        match next {
            Ok(Some(r)) => {
                stored.insert(o.key.clone(), r);
            }
            Ok(None) => {}
            Err(e) => {
                failure(&mut context, &id, &o.local_date, e);
                if old.is_none() {
                    ids.remove(&o.key);
                    days.remove(&(o.objective_id.to_string(), o.local_date.clone()));
                }
            }
        }
    }
    let by_id: HashMap<_, _> = stored.values().map(|r| (r.row.id.as_str(), r)).collect();
    for o in &derived.occurrences {
        if o.overridden || o.template.placement != RoutinePlacement::Planned {
            continue;
        }
        let Some(id) = ids.get(&o.key) else {
            continue;
        };
        for (predecessor, minimum, maximum) in &o.after {
            let Some(parent) = days
                .get(&(predecessor.to_string(), o.local_date.clone()))
                .and_then(|id| by_id.get(id.as_str()))
            else {
                continue;
            };
            if parent.row.status == "completed" {
                if let Some(end) = seconds(&json!(parent.row.updated_at)) {
                    let floor = end.saturating_add(*minimum as u64);
                    context
                        .realized_floors
                        .entry(id.clone())
                        .and_modify(|v| *v = (*v).max(floor))
                        .or_insert(floor);
                    if let Some(maximum) = maximum {
                        let ceiling = end.saturating_add(*maximum as u64)
                            .saturating_add(o.template.duration_estimate.scalar_seconds()).min(o.declared_end);
                        context.realized_ceilings.entry(id.clone())
                            .and_modify(|value| *value = (*value).min(ceiling)).or_insert(ceiling);
                    }
                }
            }
        }
    }
    Ok(context)
}

/// Refresh only the placement derived from an explicit date override (or its
/// removal), including active occurrences with start evidence. Callers hold the
/// import, materialization and action locks. Ordinary Task field edits stay closed.
pub(crate) async fn refresh_override_window(
    state: &AppState,
    occurrence: &Occurrence,
    now: UbuTimestamp,
) -> Result<()> {
    let rows = sqlx::query_as::<_, ObjectRecord>("SELECT * FROM objects WHERE object_type='Task' AND status='active' AND json_extract(payload_json,'$.occurrence.routine_objective_id')=? AND json_extract(payload_json,'$.occurrence.local_date')=?")
        .bind(occurrence.objective_id.as_str()).bind(&occurrence.local_date)
        .fetch_all(state.inner().store.pool()).await.map_err(internal)?;
    for row in rows {
        let old = Stored::parse(row)?;
        let mut next = old.payload.clone();
        let object = next.as_object_mut().ok_or_else(|| internal("stored Task is not an object"))?;
        object.remove("static_window");
        object.remove("allowed_time_range");
        if occurrence.overridden || occurrence.template.placement == RoutinePlacement::Static {
            next["static_window"] = json!({"start":timestamp_at(occurrence.start)?,"end":timestamp_at(occurrence.end)?});
        } else {
            next["allowed_time_range"] = json!({"earliest_start":timestamp_at(occurrence.start)?,"latest_finish":timestamp_at(occurrence.end)?});
        }
        if canonical(&next) != canonical(&old.payload) {
            write(state, &old.row.id, Some(&old), &old.row.compartment_label, next, now).await?;
        }
    }
    Ok(())
}
