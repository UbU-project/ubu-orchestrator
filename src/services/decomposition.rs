//! One batch per structural replacement; no mutation escapes before validation.
use super::task_capture;
use crate::{
    api::container::{
        ContainerListResponse, DecomposeRequest, DecomposeResponse, UndoRequest, UndoResponse,
        CONTAINER_SCHEMA_VERSION,
    },
    errors::{AppError, Result},
    state::AppState,
};
use serde_json::{json, Value};
use ubu_core::{
    core::{Container, ContainerStatus, Task, TaskStatus},
    AuthoritySource, MutationEnvelope, ObjectType, UbuId, VersionRef,
};
use ubu_store::{
    models::{
        log_record::{LogRecord, NewLogRecord},
        object_record::{NewObjectRecord, ObjectRecord},
    },
    queries::{self, BatchWrite},
};

fn internal(e: impl std::fmt::Display) -> AppError {
    AppError::Internal(e.to_string())
}
fn reject(code: &str, message: impl Into<String>) -> AppError {
    AppError::bad_request_diagnostic(code, message)
}
fn validate_schema(version: Option<&str>) -> Result<()> {
    match version {
        Some(CONTAINER_SCHEMA_VERSION) => Ok(()),
        None => Err(reject(
            "missing_schema_version",
            "schema_version is required",
        )),
        Some(other) => Err(reject(
            "unknown_schema_version",
            format!("unsupported schema_version `{other}`"),
        )),
    }
}
fn version(value: i64) -> Result<VersionRef> {
    let v = u64::try_from(value).map_err(|_| {
        reject(
            "invalid_expected_version",
            "expected_version must be positive",
        )
    })?;
    if v == 0 {
        return Err(reject(
            "invalid_expected_version",
            "expected_version must be positive",
        ));
    }
    Ok(VersionRef::Version(v))
}
fn envelope(state: &AppState, id: &str, observed: VersionRef) -> Result<MutationEnvelope> {
    state.envelope_for(
        [(UbuId::parse(id)?, observed)].into_iter().collect(),
        AuthoritySource::User,
        state.planning_now(),
    )
}
async fn load(state: &AppState, id: &str, kind: &str) -> Result<ObjectRecord> {
    queries::get_current_state(state.inner().store.pool(), id)
        .await?
        .filter(|r| r.object_type == kind)
        .ok_or_else(|| AppError::NotFound(format!("{kind} `{id}` does not exist")))
}
fn payload(row: &ObjectRecord) -> Result<Value> {
    serde_json::from_str(&row.payload_json).map_err(internal)
}
pub(crate) async fn containers(state: &AppState) -> Result<Vec<(ObjectRecord, Container)>> {
    let rows = sqlx::query_as::<_, ObjectRecord>(
        "SELECT * FROM objects WHERE object_type='Container' ORDER BY id",
    )
    .fetch_all(state.inner().store.pool())
    .await
    .map_err(internal)?;
    rows.into_iter()
        .map(|row| {
            let c = serde_json::from_str(&row.payload_json).map_err(internal)?;
            Ok((row, c))
        })
        .collect()
}
fn label(children: &[Value], indices: impl IntoIterator<Item = usize>) -> String {
    indices
        .into_iter()
        .map(|i| {
            format!(
                "child {} ({})",
                i + 1,
                children[i]["title"].as_str().unwrap_or("untitled")
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}
fn segments(len: usize, splits: &[usize]) -> Vec<std::ops::Range<usize>> {
    let mut start = 0;
    splits
        .iter()
        .copied()
        .chain(std::iter::once(len))
        .map(|end| {
            let r = start..end;
            start = end;
            r
        })
        .collect()
}
pub(crate) fn placement_seconds(value: &Value) -> u64 {
    value["duration_estimate"]["seconds"]
        .as_u64()
        .or_else(|| value["duration_estimate"]["mode_seconds"].as_u64())
        .unwrap_or(1800)
}
fn seconds(value: &Value) -> Result<i64> {
    Ok(ubu_core::UbuTimestamp::parse(
        value
            .as_str()
            .ok_or_else(|| reject("invalid_timestamp", "expected timestamp"))?,
    )?
    .inner()
    .unix_timestamp())
}
fn validate_segments(state: &AppState, children: &[Value], splits: &[usize]) -> Result<()> {
    for range in segments(children.len(), splits)
        .into_iter()
        .filter(|r| r.len() > 1)
    {
        let names = label(children, range.clone());
        let mut start = state.planning_now().inner().unix_timestamp();
        let mut end = i64::MAX;
        let mut duration = 0u64;
        for i in range.clone() {
            let c = &children[i];
            if c.get("preconditions").is_some_and(|v| !v.is_null()) {
                return Err(reject("decompose_interior_precondition_needs_boundary", format!("{} declares preconditions; add boundaries around this child in segment [{names}]",label(children,[i]))));
            }
            if c.get("static_window").is_some_and(|v| !v.is_null()) {
                return Err(reject(
                    "decompose_static_child_needs_boundary",
                    format!(
                        "{} is Static; add boundaries around this child in segment [{names}]",
                        label(children, [i])
                    ),
                ));
            }
            if let Some(r) = c.get("allowed_time_range").filter(|v| !v.is_null()) {
                start = start.max(seconds(&r["earliest_start"])?);
                end = end.min(seconds(&r["latest_finish"])?);
            }
            if let Some(d) = c.get("due_at").filter(|v| !v.is_null()) {
                end = end.min(seconds(d)?);
            }
            duration=duration.checked_add(placement_seconds(c)).ok_or_else(|| reject("decompose_segment_bounds_disjoint",format!("Segment [{names}] exceeds the supported contiguous duration; add a split point")))?;
        }
        if end <= start
            || i64::try_from(duration)
                .ok()
                .and_then(|d| start.checked_add(d))
                .is_none_or(|finish| finish > end)
        {
            return Err(reject("decompose_segment_bounds_disjoint",format!("Segment [{names}] has no shared window long enough; add split points or revise the child bounds")));
        }
    }
    Ok(())
}
fn update(
    state: &AppState,
    row: &ObjectRecord,
    value: Value,
    observed: VersionRef,
) -> Result<BatchWrite> {
    Ok(BatchWrite::Object {
        envelope: envelope(state, &row.id, observed)?,
        record: NewObjectRecord {
            id: row.id.clone(),
            object_type: row.object_type.clone(),
            version: row.version,
            status: value["status"]
                .as_str()
                .ok_or_else(|| internal("missing status"))?
                .into(),
            compartment_label: row.compartment_label.clone(),
            payload: value,
            created_at: row.created_at.clone(),
            updated_at: state.planning_now().to_string(),
        },
    })
}
fn moot(mut value: Value) -> Value {
    value["status"] = json!("moot");
    value["moot_reason_code"] = json!("replaced_by_new_plan_structure");
    value
}
fn log(
    state: &AppState,
    id: &str,
    event: &str,
    refs: Value,
    data: Value,
    observed: MutationEnvelope,
) -> BatchWrite {
    BatchWrite::Log {
        envelope: observed,
        record: NewLogRecord {
            id: id.into(),
            event_type: event.into(),
            object_refs: refs,
            payload: data,
            provenance: json!({"created_at":state.planning_now(),"authority_source":"user"}),
            created_at: state.planning_now().to_string(),
        },
    }
}
async fn admit(state: &AppState, writes: Vec<BatchWrite>) -> Result<()> {
    queries::admit_batch(state.inner().store.pool(), writes)
        .await
        .map_err(|e| match e {
            ubu_store::StoreError::PreconditionFailed { .. } => {
                AppError::conflict_diagnostic("version_conflict", e.to_string())
            }
            other => AppError::Store(other),
        })?;
    Ok(())
}
pub async fn decompose(
    state: &AppState,
    id: &str,
    request: DecomposeRequest,
) -> Result<DecomposeResponse> {
    let _guard = state.inner().task_action_lock.lock().await;
    validate_schema(request.schema_version.as_deref())?;
    let origin = load(state, id, "Task").await?;
    let original = payload(&origin)?;
    let names = label(&request.children, 0..request.children.len());
    if original.get("occurrence").is_some_and(|v| !v.is_null()) {
        return Err(reject(
            "decompose_routine_occurrence_unsupported",
            format!("Routine occurrence cannot become [{names}]; edit its template instead"),
        ));
    }
    if origin.status != "active" {
        return Err(reject(
            "decompose_inactive_task",
            format!("Only active work can become [{names}]"),
        ));
    }
    if request.children.len() < 2 {
        return Err(reject(
            "decompose_needs_children",
            format!("At least two children are required; proposed [{names}]"),
        ));
    }
    if containers(state)
        .await?
        .iter()
        .any(|(_, c)| c.items.iter().any(|i| i.object_ref.id.as_str() == id))
    {
        return Err(reject(
            "decompose_nested_unsupported",
            "A decomposition child cannot itself be decomposed",
        ));
    }
    let splits: Vec<usize> = request
        .segment_split_points
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| {
            reject(
                "decompose_invalid_split_points",
                format!("Split points for [{names}] must be integer boundaries"),
            )
        })?
        .unwrap_or_default();
    if splits
        .iter()
        .any(|&p| p == 0 || p >= request.children.len())
        || splits.windows(2).any(|w| w[0] >= w[1])
    {
        return Err(reject(
            "decompose_invalid_split_points",
            format!("Split points for [{names}] must increase strictly inside the list"),
        ));
    }
    let observed = version(request.expected_version)?;
    // Prepare once to mint fresh handles, then resolve the proposed-child references.
    let mut prepared = Vec::new();
    let mut dependencies = Vec::new();
    for (index, mut fields) in request.children.iter().cloned().enumerate() {
        let deps = fields
            .as_object_mut()
            .and_then(|m| m.remove("blocked_by"))
            .unwrap_or_else(|| json!([]));
        let deps: Vec<String> = serde_json::from_value(deps).map_err(|_| {
            reject(
                "decompose_child_order_conflict",
                format!(
                    "{} needs a list of prerequisite references",
                    label(&request.children, [index])
                ),
            )
        })?;
        dependencies.push(deps);
        prepared.push(task_capture::prepare(
            state,
            fields,
            &origin.compartment_label,
        )?);
    }
    validate_segments(state, &request.children, &splits)?;
    let ids: Vec<_> = prepared.iter().map(|(_, r)| r.id.clone()).collect();
    for (i, (_, record)) in prepared.iter_mut().enumerate() {
        let mut deps = Vec::new();
        for dep in &dependencies[i] {
            let resolved = if let Some(position) = dep.strip_prefix("child:") {
                let index = position
                    .parse::<usize>()
                    .ok()
                    .and_then(|p| p.checked_sub(1))
                    .filter(|&p| p < ids.len())
                    .ok_or_else(|| {
                        reject(
                            "decompose_child_order_conflict",
                            format!(
                                "{} names an invalid sibling position",
                                label(&request.children, [i])
                            ),
                        )
                    })?;
                if index >= i {
                    return Err(reject(
                        "decompose_child_order_conflict",
                        format!(
                            "{} depends on {}; reorder children or revise the dependency",
                            label(&request.children, [i]),
                            label(&request.children, [index])
                        ),
                    ));
                }
                ids[index].clone()
            } else {
                dep.clone()
            };
            if !deps.contains(&resolved) {
                deps.push(resolved);
            }
        }
        if i > 0 && !deps.contains(&ids[i - 1]) {
            deps.push(ids[i - 1].clone());
        }
        record.payload["blocked_by"] = json!(deps);
        let task: Task = serde_json::from_value(record.payload.clone()).map_err(|e| {
            reject(
                "invalid_child",
                format!("{}: {e}", label(&request.children, [i])),
            )
        })?;
        task.validate()?;
    }
    let container_id = UbuId::new(ObjectType::Container).to_string();
    let log_id = UbuId::new(ObjectType::LogEntry).to_string();
    let items:Vec<_>=prepared.iter().map(|(_,r)| json!({"ref":{"id":r.id,"object_type":"Task"},"summary":r.payload["title"]})).collect();
    let value = json!({"id":container_id,"name":original["title"],"status":"active","origin_task_ref":id,"origin_task_version":request.expected_version,
        "mutation_reason":"decomposition","mutation_log_ref":log_id,"items":items,"segment_split_points":splits,
        "provenance":{"created_at":state.planning_now(),"authority_source":"user"}});
    let _: Container = serde_json::from_value(value.clone()).map_err(internal)?;
    let mut writes = vec![log(
        state,
        &log_id,
        "task_decomposed",
        json!([id, container_id]),
        json!({"origin_task_snapshot":original,"container_id":container_id,"child_task_ids":ids}),
        envelope(state, id, observed.clone())?,
    )];
    writes.extend(
        prepared
            .into_iter()
            .map(|(envelope, record)| BatchWrite::Object { envelope, record }),
    );
    writes.push(BatchWrite::Object {
        envelope: envelope(state, &container_id, VersionRef::Absent)?,
        record: NewObjectRecord {
            id: container_id.clone(),
            object_type: "Container".into(),
            version: 1,
            status: "active".into(),
            compartment_label: origin.compartment_label.clone(),
            payload: value,
            created_at: state.planning_now().to_string(),
            updated_at: state.planning_now().to_string(),
        },
    });
    writes.push(update(state, &origin, moot(original), observed)?);
    admit(state, writes).await?;
    Ok(DecomposeResponse {
        schema_version: CONTAINER_SCHEMA_VERSION.into(),
        container_id,
        origin_task_id: id.into(),
        child_task_ids: ids,
        log_id,
    })
}

pub async fn undo(state: &AppState, id: &str, request: UndoRequest) -> Result<UndoResponse> {
    let _guard = state.inner().task_action_lock.lock().await;
    validate_schema(request.schema_version.as_deref())?;
    let row = load(state, id, "Container").await?;
    let mut c: Container = serde_json::from_str(&row.payload_json).map_err(internal)?;
    if c.status != ContainerStatus::Active {
        return Err(reject(
            "container_already_superseded",
            "This Container has already been undone",
        ));
    }
    let origin = load(state, c.origin_task_ref.as_str(), "Task").await?;
    let mutation = sqlx::query_as::<_, LogRecord>("SELECT * FROM logs WHERE id=?")
        .bind(c.mutation_log_ref.as_str())
        .fetch_one(state.inner().store.pool())
        .await
        .map_err(internal)?;
    let data: Value = serde_json::from_str(&mutation.payload_json).map_err(internal)?;
    let original = data
        .get("origin_task_snapshot")
        .cloned()
        .unwrap_or(payload(&origin)?);
    let mut fields = json!({});
    for key in [
        "title",
        "description",
        "objective_id",
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
    ] {
        if let Some(value) = original.get(key) {
            fields[key] = value.clone();
        }
    }
    let now = state.planning_now().inner().unix_timestamp();
    for (key, end) in [
        ("static_window", "end"),
        ("allowed_time_range", "latest_finish"),
    ] {
        if let Some(value) = fields.get(key) {
            if !seconds(&value[end]).is_ok_and(|end| end > now) {
                fields.as_object_mut().unwrap().remove(key);
            }
        }
    }
    if fields
        .get("due_at")
        .is_some_and(|v| !seconds(v).is_ok_and(|end| end > now))
    {
        fields.as_object_mut().unwrap().remove("due_at");
    }
    let (prepared, record) = task_capture::prepare(state, fields, &origin.compartment_label)?;
    let restored_task_id = record.id.clone();
    let log_id = UbuId::new(ObjectType::LogEntry).to_string();
    let mut read_versions = [
        (UbuId::parse(id)?, version(row.version)?),
        (UbuId::parse(&origin.id)?, version(origin.version)?),
    ]
    .into_iter()
    .collect::<std::collections::BTreeMap<_, _>>();
    let mut children_mooted = Vec::new();
    let mut children_left_completed = Vec::new();
    let mut child_writes = Vec::new();
    for item in &c.items {
        let child = load(state, item.object_ref.id.as_str(), "Task").await?;
        read_versions.insert(item.object_ref.id.clone(), version(child.version)?);
        if child.status == "completed" {
            children_left_completed.push(child.id.clone());
        } else {
            children_mooted.push(child.id.clone());
            child_writes.push(update(
                state,
                &child,
                moot(payload(&child)?),
                version(child.version)?,
            )?);
        }
    }
    let mut writes = vec![
        log(
            state,
            &log_id,
            "container_decomposition_undone",
            json!([id, restored_task_id]),
            json!({"restored_task_id":restored_task_id,"children_mooted":children_mooted,"children_left_completed":children_left_completed}),
            state.envelope_for(read_versions, AuthoritySource::User, state.planning_now())?,
        ),
        BatchWrite::Object {
            envelope: prepared,
            record,
        },
    ];
    writes.extend(child_writes);
    c.status = ContainerStatus::Superseded;
    c.superseded_by_task_ref = Some(UbuId::parse(&restored_task_id)?);
    c.validate()?;
    writes.push(update(
        state,
        &row,
        serde_json::to_value(c).map_err(internal)?,
        version(row.version)?,
    )?);
    admit(state, writes).await?;
    Ok(UndoResponse {
        schema_version: CONTAINER_SCHEMA_VERSION.into(),
        container_id: id.into(),
        restored_task_id,
        children_mooted,
        children_left_completed,
        log_id,
    })
}
pub async fn list(state: &AppState) -> Result<ContainerListResponse> {
    let mut result = Vec::new();
    for (row, c) in containers(state).await? {
        let origin = load(state, c.origin_task_ref.as_str(), "Task").await?;
        let mut children = Vec::new();
        let mut states = Vec::new();
        for (position, item) in c.items.iter().enumerate() {
            let child = load(state, item.object_ref.id.as_str(), "Task").await?;
            let value = payload(&child)?;
            states
                .push(serde_json::from_value::<TaskStatus>(json!(child.status)).map_err(internal)?);
            children.push(json!({"id":child.id,"title":value["title"],"status":child.status,"position":position}));
        }
        let mut summary = json!({"id":row.id,"version":row.version,"name":c.name,"status":c.status,
            "origin_task_ref":c.origin_task_ref,"origin_title":payload(&origin)?["title"],"children":children,
            "segments":c.segments(),"completion_state":c.completion_state(&states)?});
        if let Some(id) = c.superseded_by_task_ref {
            summary["superseded_by_task_ref"] = json!(id);
        }
        result.push(summary);
    }
    Ok(ContainerListResponse {
        schema_version: CONTAINER_SCHEMA_VERSION.into(),
        containers: result,
    })
}
