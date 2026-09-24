//! Narrow, one-way import. Mirrors deliberately ignore Quick UbU fields outside this contract.
use crate::{
    api::quick_ubu::*,
    errors::{AppError, Result},
    state::AppState,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use ubu_core::{
    core::{validate_local_time, validate_timezone, Objective, Preference, Task},
    AuthoritySource, ObjectType, UbuId, UbuTimestamp, VersionRef,
};
use ubu_store::{
    models::object_record::{NewObjectRecord, ObjectRecord},
    queries,
};

#[derive(Deserialize)]
struct Snapshot {
    snapshot_version: u32,
    store: SnapshotStore,
    task_origins: BTreeMap<String, TaskOrigin>,
}
#[derive(Deserialize)]
struct SnapshotStore {
    routines: BTreeMap<String, Routine>,
    tasks: BTreeMap<String, QuickTask>,
    objectives: BTreeMap<String, Value>,
    bundles: BTreeMap<String, Bundle>,
    preferences: Vec<QuickPreference>,
}
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum TaskOrigin {
    RoutineOccurrence,
    OrphanedRoutineOccurrence,
    CalendarCapture,
    Manual,
}
#[derive(Deserialize)]
struct Routine {
    id: String,
    title: String,
    recurrence: Recurrence,
    start_time: String,
    duration: (i64, i32),
    #[serde(default)]
    dynamic: bool,
    latest_tod: Option<String>,
    category: Option<String>,
    #[serde(default)]
    transparent: bool,
    #[serde(default)]
    reminders: Vec<i64>,
    #[serde(default)]
    after: Vec<After>,
    #[serde(default)]
    establishes: Vec<String>,
    #[serde(default)]
    requires: Vec<Requires>,
}
#[derive(Deserialize)]
enum Recurrence {
    Daily,
    Weekly { weekdays: Vec<String> },
    MonthlyDay { days: Vec<u8> },
    MonthlyFirstWorkday,
    QuarterlyFirstWorkday,
}
#[derive(Deserialize)]
struct After {
    template_id: String,
    offset: (i64, i32),
    #[serde(default)]
    maximum: Option<(i64, i32)>,
}
#[derive(Deserialize)]
struct Requires {
    fact: String,
    #[serde(default)]
    offset: Option<(i64, i32)>,
    #[serde(default)]
    maximum: Option<(i64, i32)>,
}
#[derive(Deserialize)]
struct Window {
    start: String,
    end: String,
}
#[derive(Deserialize)]
struct QuickTask {
    id: String,
    title: String,
    status: QuickStatus,
    detail: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    category: Option<String>,
    pinned: Option<Window>,
    #[serde(default)]
    transparent: bool,
    est_duration: (i64, i32),
    due: Option<String>,
    earliest_start: Option<String>,
    must_finish_by: Option<String>,
    #[serde(default)]
    blocked_by: Vec<String>,
    #[serde(default)]
    after: Vec<Value>,
}
#[derive(Deserialize)]
enum QuickStatus {
    Backlog,
    Scheduled,
    Active,
    Deferred,
    Done,
}
#[derive(Deserialize)]
struct Bundle {
    members: Vec<String>,
}
#[derive(Deserialize)]
struct QuickPreference {
    left: String,
    right: String,
    relation: Relation,
}
#[derive(Deserialize)]
enum Relation {
    Strict,
    Indifferent,
}

struct Mapping {
    kind: &'static str,
    object_type: ObjectType,
    source: String,
    payload: Value,
}
fn skip(response: &mut QuickUbuImportResponse, kind: &str, id: &str, reason: impl Into<String>) {
    response.skipped.push(QuickUbuSkipped {
        kind: kind.into(),
        quick_ubu_id: id.into(),
        reason: reason.into(),
    });
}
fn kind(object_type: &str) -> &'static str {
    match object_type {
        "Objective" => "routine",
        "Task" => "task",
        _ => "preference",
    }
}
fn invalid_snapshot(error: impl std::fmt::Display) -> AppError {
    AppError::bad_request_diagnostic("invalid_quick_ubu_snapshot", error.to_string())
}
fn base(id: &str, source: &str, title: Option<&str>, now: &str) -> Value {
    let mut v = json!({"id":id,"provenance":{"created_at":now,"authority_source":"user","source":{"source_kind":"quick_ubu","source_id":source}}});
    if let Some(title) = title {
        v["title"] = json!(title);
        v["status"] = json!("active");
    }
    v
}
fn normalize(object_type: ObjectType, payload: Value) -> std::result::Result<Value, String> {
    fn normalized<T: serde::de::DeserializeOwned + serde::Serialize>(
        v: Value,
    ) -> std::result::Result<Value, String> {
        let parsed: T = serde_json::from_value(v).map_err(|e| e.to_string())?;
        serde_json::to_value(parsed).map_err(|e| e.to_string())
    }
    match object_type {
        ObjectType::Objective => normalized::<Objective>(payload),
        ObjectType::Task => {
            let task: Task = serde_json::from_value(payload).map_err(|e| e.to_string())?;
            task.validate().map_err(|e| e.to_string())?;
            serde_json::to_value(task).map_err(|e| e.to_string())
        }
        ObjectType::Preference => normalized::<Preference>(payload),
        _ => unreachable!(),
    }
}
fn schedule_rule(rule: &Recurrence) -> Value {
    match rule {
        Recurrence::Daily => json!({"kind":"daily"}),
        Recurrence::Weekly { weekdays } => {
            json!({"kind":"weekly","weekdays":weekdays.iter().map(|d|d.to_lowercase()).collect::<Vec<_>>()})
        }
        Recurrence::MonthlyDay { days } => json!({"kind":"monthly_day","days":days}),
        Recurrence::MonthlyFirstWorkday => json!({"kind":"first_workday_of_month"}),
        Recurrence::QuarterlyFirstWorkday => json!({"kind":"first_workday_of_quarter"}),
    }
}
fn routine_payload(
    r: &Routine,
    id: &str,
    timezone: &str,
    now: &str,
    response: &mut QuickUbuImportResponse,
) -> std::result::Result<Value, String> {
    if r.duration.1 != 0 {
        return Err("invalid: fractional duration is not representable in whole seconds".into());
    }
    let mut payload = base(id, &r.id, Some(&r.title), now);
    payload["mode"] = json!("evergreen");
    payload["recurrence"] =
        json!({"timezone":timezone,"rule":schedule_rule(&r.recurrence),"schedule_version":1});
    let mut template = json!({"title":r.title,"duration_estimate":{"type":"fixed","seconds":r.duration.0},"nominal_start":r.start_time,
        "placement":if r.dynamic{"planned"}else{"static"},"occupies_capacity":!r.transparent,"template_version":1});
    if let Some(category) = &r.category {
        template["category_tag"] = json!(category);
        template["tags"] = json!([category]);
    }
    let mut reminders = Vec::new();
    for &n in &r.reminders {
        if n < 0 {
            skip(response, "routine", &r.id, "negative_reminder");
        } else {
            reminders.push(n);
        }
    }
    template["reminder_minutes"] = json!(reminders);
    if r.dynamic {
        let latest = r.latest_tod.as_deref().unwrap_or("23:59:59");
        if validate_local_time(&r.start_time).is_err()
            || validate_local_time(latest).is_err()
            || r.start_time.as_str() >= latest
        {
            return Err("invalid_local_range".into());
        }
        template["allowed_local_range"] = json!({"earliest":r.start_time,"latest":latest});
    }
    payload["routine_instance_template"] = template;
    normalize(ObjectType::Objective, payload).map_err(|e| format!("invalid: {e}"))
}
fn task_payload(
    t: &QuickTask,
    id: &str,
    origin: TaskOrigin,
    now: UbuTimestamp,
    response: &mut QuickUbuImportResponse,
) -> std::result::Result<Value, String> {
    match origin {
        TaskOrigin::RoutineOccurrence => return Err("routine_occurrence".into()),
        TaskOrigin::OrphanedRoutineOccurrence => return Err("orphaned_routine_occurrence".into()),
        _ => {}
    }
    if matches!(t.status, QuickStatus::Done) {
        return Err("completed".into());
    }
    let mut payload = base(id, &t.id, Some(&t.title), &now.to_string());
    if let Some(pin) = &t.pinned {
        let end = UbuTimestamp::parse(&pin.end).map_err(|e| format!("invalid: {e}"))?;
        if end <= now {
            return Err("past_static_window".into());
        }
        payload["static_window"] = json!({"start":pin.start,"end":pin.end});
    } else {
        match (&t.earliest_start, &t.must_finish_by) {
            (Some(start), Some(end)) => {
                let parsed = UbuTimestamp::parse(start)
                    .and_then(|a| UbuTimestamp::parse(end).map(|b| (a, b)));
                match parsed {
                    Ok((a, b)) if a < b => {
                        payload["allowed_time_range"] =
                            json!({"earliest_start":a,"latest_finish":b})
                    }
                    _ => skip(response, "task", &t.id, "invalid_allowed_range"),
                }
            }
            (None, None) => {}
            _ => skip(response, "task", &t.id, "partial_allowed_range"),
        }
    }
    if let Some(detail) = &t.detail {
        payload["description"] = json!(detail);
    }
    let mut tags = t.tags.clone();
    if let Some(category) = &t.category {
        if !tags.contains(category) {
            tags.push(category.clone());
        }
        payload["category_tag"] = json!(category);
    }
    // Union preserves first appearance and never persists duplicate tags.
    let mut seen = BTreeSet::new();
    tags.retain(|tag| seen.insert(tag.clone()));
    payload["tags"] = json!(tags);
    payload["occupies_capacity"] = json!(!t.transparent);
    if t.est_duration.1 != 0 {
        return Err("invalid: fractional duration is not representable in whole seconds".into());
    }
    if t.est_duration.0 > 0 {
        payload["duration_estimate"] = json!({"type":"fixed","seconds":t.est_duration.0});
    }
    if let Some(due) = &t.due {
        payload["due_at"] = json!(due);
    }
    if !t.after.is_empty() {
        skip(response, "task", &t.id, "task_after_unsupported");
    }
    normalize(ObjectType::Task, payload).map_err(|e| format!("invalid: {e}"))
}

pub async fn import(
    state: AppState,
    request: QuickUbuImportRequest,
) -> Result<QuickUbuImportResponse> {
    validate_timezone(&request.timezone)
        .map_err(|e| AppError::bad_request_diagnostic("invalid_timezone", e.to_string()))?;
    let content = std::fs::read_to_string(&request.snapshot_path).map_err(invalid_snapshot)?;
    let snapshot: Snapshot = serde_json::from_str(&content).map_err(invalid_snapshot)?;
    if snapshot.snapshot_version != 1 {
        return Err(invalid_snapshot("snapshot_version must be 1"));
    }
    if snapshot
        .store
        .tasks
        .iter()
        .any(|(id, t)| id != &t.id || !snapshot.task_origins.contains_key(id))
        || snapshot.store.routines.iter().any(|(id, r)| id != &r.id)
    {
        return Err(invalid_snapshot(
            "store ids must match keys and every Task must have an origin",
        ));
    }
    // Serialize imports sharing this store so concurrent retries reuse source identities.
    let _import_guard = state.inner().quick_ubu_import_lock.lock().await;
    let now = state.planning_now();
    let now_string = now.to_string();
    let mut response = QuickUbuImportResponse {
        schema_version: "quick-ubu-import/1".into(),
        dry_run: request.dry_run,
        routines: Default::default(),
        tasks: Default::default(),
        preferences: Default::default(),
        objectives_not_imported: snapshot.store.objectives.len(),
        skipped: Vec::new(),
        diverged: Vec::new(),
        stale: Vec::new(),
    };
    let rows=sqlx::query_as::<_,ObjectRecord>("SELECT * FROM objects WHERE object_type IN ('Objective','Task','Preference') AND json_extract(payload_json, '$.provenance.source.source_kind') = 'quick_ubu' ORDER BY object_type, id")
        .fetch_all(state.inner().store.pool()).await.map_err(|e|AppError::Internal(e.to_string()))?;
    let mut existing = BTreeMap::new();
    for row in rows {
        let payload: Value = serde_json::from_str(&row.payload_json)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let source = payload["provenance"]["source"]["source_id"]
            .as_str()
            .ok_or_else(|| AppError::Internal("import source has no source_id".into()))?
            .to_owned();
        if existing
            .insert((row.object_type.clone(), source), (row, payload))
            .is_some()
        {
            return Err(AppError::Internal(
                "duplicate Quick UbU source identity".into(),
            ));
        }
    }
    let object_id = |object_type: ObjectType, source: &str| {
        existing
            .get(&(object_type.as_str().into(), source.into()))
            .map(|(r, _)| r.id.clone())
            .unwrap_or_else(|| UbuId::new(object_type).to_string())
    };
    let mut routines = BTreeMap::new();
    for r in snapshot.store.routines.values() {
        match routine_payload(
            r,
            &object_id(ObjectType::Objective, &r.id),
            &request.timezone,
            &now_string,
            &mut response,
        ) {
            Ok(payload) => {
                routines.insert(r.id.clone(), payload);
            }
            Err(reason) => skip(&mut response, "routine", &r.id, reason),
        }
    }
    let routine_ids: BTreeMap<_, _> = routines
        .iter()
        .map(|(source, p)| (source.clone(), p["id"].clone()))
        .collect();
    for (source, payload) in &mut routines {
        let mut after = Vec::new();
        for a in &snapshot.store.routines[source].after {
            let reason = if a.template_id == *source {
                Some("after_self_reference")
            } else if !routine_ids.contains_key(&a.template_id) {
                Some("after_reference_missing")
            } else if a.offset.0 < 0 {
                Some("negative_after_offset")
            } else if a.offset.1 != 0 {
                Some("invalid: fractional after offset is not representable in whole seconds")
            } else if a.maximum.is_some_and(|(_, nanos)| nanos != 0) {
                Some("invalid: fractional after maximum is not representable in whole seconds")
            } else if a.maximum.is_some_and(|(maximum, _)| maximum < a.offset.0) {
                Some("inverted_after_bounds")
            } else {
                None
            };
            if let Some(reason) = reason {
                skip(&mut response, "routine", source, reason);
            } else {
                let mut edge = json!({"objective_id":routine_ids[&a.template_id],"minimum_seconds":a.offset.0});
                if let Some((maximum, _)) = a.maximum {
                    edge["maximum_seconds"] = json!(maximum);
                }
                after.push(edge);
            }
        }
        if !after.is_empty() {
            payload["routine_instance_template"]["after"] = json!(after);
        }
    }
    reject_routine_overlaps(&state, &routines, &existing, now).await?;
    let mut tasks = BTreeMap::new();
    for t in snapshot.store.tasks.values() {
        match task_payload(
            t,
            &object_id(ObjectType::Task, &t.id),
            snapshot.task_origins[&t.id],
            now,
            &mut response,
        ) {
            Ok(payload) => {
                tasks.insert(t.id.clone(), payload);
            }
            Err(reason) => skip(&mut response, "task", &t.id, reason),
        }
    }
    // Reach a fixpoint before wiring dependencies, so no imported Task names a skipped one.
    loop {
        let ids: BTreeMap<_, _> = tasks
            .iter()
            .map(|(source, p)| (source.clone(), p["id"].clone()))
            .collect();
        let mut invalid = Vec::new();
        for (source, payload) in &mut tasks {
            payload["blocked_by"] = json!(snapshot.store.tasks[source]
                .blocked_by
                .iter()
                .filter_map(|dep| ids.get(dep))
                .collect::<Vec<_>>());
            if let Err(error) = normalize(ObjectType::Task, payload.clone()) {
                invalid.push((source.clone(), error));
            }
        }
        if invalid.is_empty() {
            break;
        }
        for (source, error) in invalid {
            tasks.remove(&source);
            skip(&mut response, "task", &source, format!("invalid: {error}"));
        }
    }
    for source in tasks.keys() {
        for dep in &snapshot.store.tasks[source].blocked_by {
            if !tasks.contains_key(dep) {
                skip(&mut response, "task", source, "dependency_not_imported");
            }
        }
    }
    let mut preferences = Vec::new();
    let mut preference_sources = BTreeSet::new();
    for p in &snapshot.store.preferences {
        let source = format!("{}|{}", p.left, p.right);
        // The Quick UbU identity is the ordered pair, so repeated identical pairs map once.
        if !preference_sources.insert(source.clone()) {
            skip(
                &mut response,
                "preference",
                &source,
                "invalid: duplicate preference source identity",
            );
            continue;
        }
        let members = |id: &str| {
            snapshot
                .store
                .bundles
                .get(id)
                .filter(|b| b.members.len() == 1)
                .map(|b| b.members[0].as_str())
        };
        let (Some(left), Some(right)) = (members(&p.left), members(&p.right)) else {
            skip(&mut response, "preference", &source, "non_singleton_bundle");
            continue;
        };
        let (Some(a), Some(b)) = (tasks.get(left), tasks.get(right)) else {
            skip(&mut response, "preference", &source, "task_not_imported");
            continue;
        };
        let mut payload = base(
            &object_id(ObjectType::Preference, &source),
            &source,
            None,
            &now_string,
        );
        payload["task_a"] = a["id"].clone();
        payload["task_b"] = b["id"].clone();
        payload["order"] = json!(match p.relation {
            Relation::Strict => "a_preferred_to_b",
            Relation::Indifferent => "a_indifferent_to_b",
        });
        payload["acquired_method"] = json!("user_defined");
        payload["acquired_date"] = json!(now_string);
        payload["enabled"] = json!(true);
        preferences.push((source, payload));
    }
    let mappings = routines
        .into_iter()
        .map(|(source, payload)| Mapping {
            kind: "routine",
            object_type: ObjectType::Objective,
            source,
            payload,
        })
        .chain(tasks.into_iter().map(|(source, payload)| Mapping {
            kind: "task",
            object_type: ObjectType::Task,
            source,
            payload,
        }))
        .chain(preferences.into_iter().map(|(source, payload)| Mapping {
            kind: "preference",
            object_type: ObjectType::Preference,
            source,
            payload,
        }));
    let mut mapped = BTreeSet::new();
    for mapping in mappings {
        let Mapping {
            kind,
            object_type,
            source,
            mut payload,
        } = mapping;
        let key = (object_type.as_str().to_owned(), source.clone());
        let previous = existing.get(&key);
        if let Some((row, _)) = previous {
            if row.status != "active" {
                response.diverged.push(QuickUbuObjectRef {
                    kind: kind.into(),
                    id: row.id.clone(),
                    quick_ubu_id: source,
                });
                mapped.insert(key);
                continue;
            }
        }
        if let Some((_, old)) = previous {
            payload["id"] = old["id"].clone();
            payload["provenance"]["created_at"] = old["provenance"]["created_at"].clone();
            if object_type == ObjectType::Preference {
                for field in ["acquired_date", "enabled"] {
                    payload[field] = old[field].clone();
                }
            }
            if object_type == ObjectType::Objective {
                for (field, version) in [
                    ("recurrence", "schedule_version"),
                    ("routine_instance_template", "template_version"),
                ] {
                    let mut prior = old[field].clone();
                    let mut next = payload[field].clone();
                    if let (Some(prior_obj), Some(next_obj)) =
                        (prior.as_object_mut(), next.as_object_mut())
                    {
                        let current = prior_obj
                            .remove(version)
                            .and_then(|v| v.as_u64())
                            .unwrap_or(1);
                        next_obj.remove(version);
                        let version_value = if prior_obj == next_obj {
                            current
                        } else {
                            current.checked_add(1).ok_or_else(|| {
                                AppError::Internal("routine version exhausted".into())
                            })?
                        };
                        payload[field][version] = json!(version_value);
                    }
                }
            }
        }
        let payload = match normalize(object_type, payload) {
            Ok(p) => p,
            Err(error) => {
                skip(&mut response, kind, &source, format!("invalid: {error}"));
                continue;
            }
        };
        mapped.insert(key);
        let unchanged = previous.is_some_and(|(_, old)| {
            let mut a = old.clone();
            let mut b = payload.clone();
            a.as_object_mut().unwrap().remove("status");
            b.as_object_mut().unwrap().remove("status");
            a == b
        });
        let counts = match object_type {
            ObjectType::Objective => &mut response.routines,
            ObjectType::Task => &mut response.tasks,
            _ => &mut response.preferences,
        };
        if unchanged {
            counts.unchanged += 1;
            continue;
        }
        if previous.is_some() {
            counts.updated += 1;
        } else {
            counts.created += 1;
        }
        if request.dry_run {
            continue;
        }
        let id = payload["id"].as_str().unwrap().to_owned();
        let expected = previous.map_or(VersionRef::Absent, |(r, _)| {
            VersionRef::Version(r.version as u64)
        });
        let envelope = state.envelope_for(
            [(UbuId::parse(&id)?, expected)].into_iter().collect(),
            AuthoritySource::User,
            now,
        )?;
        queries::admit_object(
            state.inner().store.pool(),
            &envelope,
            NewObjectRecord {
                id,
                object_type: object_type.as_str().into(),
                version: 1,
                status: "active".into(),
                compartment_label: "quick-ubu-import".into(),
                payload,
                created_at: previous
                    .map_or_else(|| now_string.clone(), |(r, _)| r.created_at.clone()),
                updated_at: now_string.clone(),
            },
        )
        .await?;
    }
    for (key, (row, _)) in existing {
        if !mapped.contains(&key) {
            response.stale.push(QuickUbuObjectRef {
                kind: kind(&row.object_type).into(),
                id: row.id,
                quick_ubu_id: key.1,
            });
        }
    }
    Ok(response)
}

/// Evaluate the live set this import would leave, before mapping or writing any
/// Tasks or Preferences. The caller already holds quick_ubu_import_lock.
async fn reject_routine_overlaps(
    state: &AppState,
    routines: &BTreeMap<String, Value>,
    existing: &BTreeMap<(String, String), (ObjectRecord, Value)>,
    now: UbuTimestamp,
) -> Result<()> {
    use super::routine_instantiation::{static_overlaps, RoutineDefinition};
    let live = super::routine_service::live_definitions(state.inner().store.pool()).await?;
    let mut names: BTreeMap<_, _> = live
        .titles
        .into_iter()
        .map(|(id, title)| {
            let name = format!("`{id}` ({title})");
            (id, name)
        })
        .collect();
    let mut definitions = BTreeMap::new();
    for definition in live.definitions {
        definitions.insert(definition.objective_id.to_string(), definition);
    }
    for (source, payload) in routines {
        if existing
            .get(&(ObjectType::Objective.as_str().into(), source.clone()))
            .is_some_and(|(row, _)| row.status != "active")
        {
            continue;
        }
        // A mapped Objective has already passed core validation. Reparse after
        // the second-pass wiring so lowering sees the complete prospective graph.
        let Ok(objective) = serde_json::from_value::<Objective>(payload.clone()) else {
            continue;
        };
        let (Some(schedule), Some(template)) =
            (objective.recurrence, objective.routine_instance_template)
        else {
            continue;
        };
        let id = objective.id.to_string();
        names.insert(id.clone(), format!("`{source}` ({})", objective.title));
        definitions.insert(
            id,
            RoutineDefinition {
                objective_id: objective.id,
                schedule,
                template,
            },
        );
    }
    let start = u64::try_from(now.inner().unix_timestamp())
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let (pairs, _) = static_overlaps(
        &definitions.into_values().collect::<Vec<_>>(),
        start,
        start.saturating_add(366 * 86400),
    );
    let pairs: Vec<_> = pairs.into_iter().filter(|pair| !pair.dst_only).collect();
    if pairs.is_empty() {
        return Ok(());
    }
    Err(overlap_rejection(&pairs, &names))
}

fn overlap_rejection(
    pairs: &[super::routine_instantiation::RoutineOverlap],
    names: &BTreeMap<String, String>,
) -> AppError {
    fn root(parents: &mut [usize], mut n: usize) -> usize {
        while parents[n] != n {
            parents[n] = parents[parents[n]];
            n = parents[n];
        }
        n
    }
    fn count(n: usize, noun: &str) -> String {
        format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
    }
    let ids: BTreeSet<_> = pairs
        .iter()
        .flat_map(|p| [p.first.objective_id.clone(), p.second.objective_id.clone()])
        .collect();
    let indices: BTreeMap<_, _> = ids.into_iter().enumerate().map(|(i, id)| (id, i)).collect();
    let mut parents: Vec<_> = (0..indices.len()).collect();
    for pair in pairs {
        let a = root(&mut parents, indices[&pair.first.objective_id]);
        let b = root(&mut parents, indices[&pair.second.objective_id]);
        parents[a.max(b)] = a.min(b);
    }
    let mut components = BTreeMap::<usize, Vec<_>>::new();
    for pair in pairs {
        components
            .entry(root(&mut parents, indices[&pair.first.objective_id]))
            .or_default()
            .push(pair);
    }
    let mut groups = Vec::new();
    for pairs in components.into_values() {
        let date = pairs
            .iter()
            .map(|p| p.first.local_date.as_str())
            .min()
            .unwrap();
        let dates = pairs.iter().map(|p| p.dates).max().unwrap();
        // The first pair mentioning a member supplies its representative window.
        let mut members = BTreeMap::new();
        let mut self_overlaps = BTreeSet::new();
        for pair in &pairs {
            for side in [&pair.first, &pair.second] {
                members.entry(side.objective_id.clone()).or_insert(side);
            }
            if pair.self_overlap {
                self_overlaps.insert(pair.first.objective_id.clone());
            }
        }
        let smallest = members.first_key_value().unwrap().0.clone();
        let mut members: Vec<_> = members.into_values().collect();
        members.sort_by(|a, b| (&a.window, &a.objective_id).cmp(&(&b.window, &b.objective_id)));
        let member = |s: &super::routine_instantiation::OverlapSide| {
            format!(
                "{} {}-{}",
                names[s.objective_id.as_str()],
                s.window.0,
                s.window.1
            )
        };
        let message = if members.len() == 1 {
            format!(
                "Routine {} runs into its own next occurrence, first {date} ({} in the next year)",
                member(members[0]),
                count(dates, "date")
            )
        } else {
            let mut text = format!(
                "Routines overlap each other, first {date}: {} ({}, up to {} in the next year)",
                members
                    .iter()
                    .map(|s| member(s))
                    .collect::<Vec<_>>()
                    .join("; "),
                count(pairs.len(), "pair"),
                count(dates, "date")
            );
            for id in self_overlaps {
                text.push_str(&format!(
                    " ; {} also runs into its own next occurrence",
                    names[id.as_str()]
                ));
            }
            text
        };
        groups.push((date.to_owned(), smallest, message));
    }
    groups.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    let summary=format!("{} in {}; nothing was imported. Routines must not overlap: stagger their start times, shorten one, or make one transparent.",count(pairs.len(),"overlapping routine pair"),count(groups.len(),"group"));
    let mut items: Vec<_> = groups
        .iter()
        .take(25)
        .map(|(_, _, message)| ("overlapping_routines".into(), message.clone()))
        .collect();
    if groups.len() > 25 {
        items.push((
            "overlapping_routines".into(),
            format!(
                "and {} more overlapping {}",
                groups.len() - 25,
                if groups.len() - 25 == 1 {
                    "group"
                } else {
                    "groups"
                }
            ),
        ));
    }
    AppError::bad_request_diagnostics(summary, items)
}
