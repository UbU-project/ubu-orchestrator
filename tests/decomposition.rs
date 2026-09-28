use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use ubu_core::{AuthoritySource, ObjectType, UbuId, UbuTimestamp, VersionRef};
use ubu_orchestrator::{
    api::container::CONTAINER_SCHEMA_VERSION,
    build_router,
    config::ServerConfig,
    planning_time::FixedClock,
    services::{planning_service, task_capture},
    state::AppState,
};
use ubu_store::{
    models::object_record::{NewObjectRecord, ObjectRecord},
    queries,
};

const NOW: &str = "2026-09-27T09:00:00Z";
const END: &str = "2026-09-27T18:00:00Z";
async fn state() -> AppState {
    AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(NOW).unwrap()))
}
async fn request(s: &AppState, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = build_router(s.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"raw":String::from_utf8_lossy(&bytes)})),
    )
}
fn child(title: &str, seconds: u64) -> Value {
    json!({"title":title,"duration_estimate":{"type":"fixed","seconds":seconds}})
}
fn children() -> Vec<Value> {
    vec![
        child("Buy hinges", 1800),
        child("Remove old hinges", 1800),
        child("Hang the gate", 1800),
    ]
}
async fn task(s: &AppState, fields: Value) -> String {
    task_capture::capture(s, fields).await.unwrap().0
}
async fn origin(s: &AppState) -> String {
    task(s,json!({"title":"Repair gate","description":"Keep the gate working","duration_estimate":{"type":"fixed","seconds":3600},"tags":["home"],"category_tag":"home"})).await
}
async fn row(s: &AppState, id: &str) -> ObjectRecord {
    queries::get_current_state(s.inner().store.pool(), id)
        .await
        .unwrap()
        .unwrap()
}
fn payload(r: &ObjectRecord) -> Value {
    serde_json::from_str(&r.payload_json).unwrap()
}
async fn counts(s: &AppState) -> (i64, i64, i64) {
    sqlx::query_as("SELECT (SELECT COUNT(*) FROM objects),(SELECT COUNT(*) FROM logs),(SELECT COUNT(*) FROM mutation_envelopes)").fetch_one(s.inner().store.pool()).await.unwrap()
}
async fn snapshot(s: &AppState) -> String {
    let objects: Vec<(String, i64, String, String)> =
        sqlx::query_as("SELECT id,version,status,payload_json FROM objects ORDER BY id")
            .fetch_all(s.inner().store.pool())
            .await
            .unwrap();
    format!("{:?}{:?}", objects, counts(s).await)
}
fn body(version: i64, children: Vec<Value>, splits: Value) -> Value {
    json!({"schema_version":CONTAINER_SCHEMA_VERSION,"expected_version":version,"children":children,"segment_split_points":splits})
}
async fn decompose(s: &AppState, id: &str, children: Vec<Value>, splits: Value) -> Value {
    let (status, value) = request(
        s,
        "POST",
        &format!("/task/{id}/decompose"),
        body(1, children, splits),
    )
    .await;
    assert_eq!(status, 200, "{value}");
    value
}
fn ids(value: &Value) -> Vec<String> {
    value["child_task_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().into())
        .collect()
}
async fn generate(s: &AppState, end: &str) -> Value {
    let (status, value) = request(
        s,
        "POST",
        "/planning/generate",
        json!({"horizon":{"start":NOW,"end":end}}),
    )
    .await;
    assert_eq!(status, 200, "{value}");
    value
}
fn steps(value: &Value) -> &Vec<Value> {
    value["plan"]["steps"]
        .as_array()
        .unwrap_or_else(|| panic!("{value}"))
}
fn contiguous(value: &Value, ids: &[String]) {
    let list: Vec<_> = ids
        .iter()
        .map(|id| {
            steps(value)
                .iter()
                .find(|v| v["task_id"] == *id)
                .unwrap_or_else(|| panic!("missing {id}: {value}"))
        })
        .collect();
    for pair in list.windows(2) {
        assert_eq!(pair[0]["end"], pair[1]["start"], "{value}");
    }
    assert!(
        !value["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "container_segment_scattered"),
        "{value}"
    );
}
async fn change(s: &AppState, id: &str, status: &str) {
    let r = row(s, id).await;
    let mut p = payload(&r);
    p["status"] = json!(status);
    if status == "moot" {
        p["moot_reason_code"] = json!("user_declared_moot");
    }
    let e = s
        .envelope_for(
            [(
                UbuId::parse(id).unwrap(),
                VersionRef::Version(r.version as u64),
            )]
            .into_iter()
            .collect(),
            AuthoritySource::User,
            s.planning_now(),
        )
        .unwrap();
    queries::admit_object(
        s.inner().store.pool(),
        &e,
        NewObjectRecord {
            id: r.id,
            object_type: r.object_type,
            version: r.version,
            status: status.into(),
            compartment_label: r.compartment_label,
            payload: p,
            created_at: r.created_at,
            updated_at: NOW.into(),
        },
    )
    .await
    .unwrap();
}
async fn undo(s: &AppState, id: &str) -> Value {
    let (status, value) = request(
        s,
        "POST",
        &format!("/container/{id}/undo"),
        json!({"schema_version":CONTAINER_SCHEMA_VERSION}),
    )
    .await;
    assert_eq!(status, 200, "{value}");
    value
}
fn range(start: &str, end: &str) -> Value {
    json!({"earliest_start":start,"latest_finish":end})
}
async fn fixed(s: &AppState, title: &str, start: &str, end: &str) {
    task(
        s,
        json!({"title":title,"static_window":{"start":start,"end":end}}),
    )
    .await;
}

#[tokio::test]
async fn g1_atomic_decomposition_records_structure_lineage_and_origin() {
    let s = state().await;
    let o = origin(&s).await;
    let before = counts(&s).await;
    let d = decompose(&s, &o, children(), json!([])).await;
    let child_ids = ids(&d);
    let c = payload(&row(&s, d["container_id"].as_str().unwrap()).await);
    assert_eq!(c["origin_task_ref"], o);
    assert_eq!(c["origin_task_version"], 1);
    assert_eq!(c["name"], "Repair gate");
    assert_eq!(c["mutation_reason"], "decomposition");
    assert_eq!(c["mutation_log_ref"], d["log_id"]);
    assert_eq!(c["provenance"]["authority_source"], "user");
    assert_eq!(c["status"], "active");
    assert_eq!(c["segment_split_points"], json!([]));
    assert_ne!(c["id"], o);
    for (i, id) in child_ids.iter().enumerate() {
        let r = row(&s, id).await;
        let p = payload(&r);
        assert_eq!(c["items"][i]["ref"]["id"], *id);
        assert_eq!(r.compartment_label, "user-capture");
        assert!(p.get("category_tag").is_none());
        assert!(p.get("due_at").is_none());
        if i > 0 {
            assert!(p["blocked_by"]
                .as_array()
                .unwrap()
                .contains(&json!(child_ids[i - 1])));
        }
    }
    let old = row(&s, &o).await;
    assert_eq!(old.status, "moot");
    assert_eq!(
        payload(&old)["moot_reason_code"],
        "replaced_by_new_plan_structure"
    );
    let log: (String, String) =
        sqlx::query_as("SELECT event_type,payload_json FROM logs WHERE id=?")
            .bind(d["log_id"].as_str().unwrap())
            .fetch_one(s.inner().store.pool())
            .await
            .unwrap();
    assert_eq!(log.0, "task_decomposed");
    assert_eq!(
        serde_json::from_str::<Value>(&log.1).unwrap()["origin_task_snapshot"]["title"],
        "Repair gate"
    );
    let after = counts(&s).await;
    assert_eq!(after, (before.0 + 4, before.1 + 1, before.2 + 6));
    println!("EVIDENCE[g1] before={before:?} after={after:?} batch_writes=6");
}

#[tokio::test]
async fn g2_checklist_stays_contiguous_among_errands() {
    let s = state().await;
    let o = origin(&s).await;
    let d = decompose(&s, &o, children(), json!([])).await;
    for title in ["Post parcel", "Buy milk", "Collect key"] {
        task(&s, child(title, 1200)).await;
    }
    fixed(&s, "Lunch", "2026-09-27T12:00:00Z", "2026-09-27T13:00:00Z").await;
    let p = generate(&s, END).await;
    contiguous(&p, &ids(&d));
    assert!(!steps(&p).iter().any(|v| v["task_id"] == o));
    println!("EVIDENCE[g2] {}", p["plan"]["steps"]);
}

#[tokio::test]
async fn g3_two_segments_allow_other_work_between() {
    let s = state().await;
    let o = origin(&s).await;
    let mut cs = vec![
        child("A", 900),
        child("B", 900),
        child("C", 900),
        child("D", 900),
    ];
    for c in &mut cs[..2] {
        c["allowed_time_range"] = range(NOW, "2026-09-27T09:30:00Z");
    }
    for c in &mut cs[2..] {
        c["allowed_time_range"] = range("2026-09-27T10:00:00Z", "2026-09-27T10:30:00Z");
    }
    let d = decompose(&s, &o, cs, json!([2])).await;
    let ids = ids(&d);
    fixed(
        &s,
        "Other work",
        "2026-09-27T09:30:00Z",
        "2026-09-27T10:00:00Z",
    )
    .await;
    let p = generate(&s, END).await;
    contiguous(&p, &ids[..2]);
    contiguous(&p, &ids[2..]);
    let a = steps(&p).iter().find(|v| v["task_id"] == ids[1]).unwrap();
    let b = steps(&p).iter().find(|v| v["task_id"] == ids[2]).unwrap();
    assert!(a["end"].as_u64().unwrap() < b["start"].as_u64().unwrap());
}

#[tokio::test]
async fn g4_fixed_duration_and_expansion_are_exact() {
    let s = state().await;
    let o = origin(&s).await;
    let d = decompose(
        &s,
        &o,
        vec![child("A", 601), child("B", 1202), child("C", 1803)],
        json!([]),
    )
    .await;
    let ids = ids(&d);
    let req = planning_service::build_request_from_store(&s)
        .await
        .unwrap();
    assert_eq!(req.tasks.len(), 1);
    assert_eq!(
        serde_json::to_value(&req.tasks[0].duration_estimate).unwrap(),
        json!({"type":"fixed","seconds":3606})
    );
    assert_eq!(req.tasks[0].duration, 5400); // Sum of the legacy fallback durations; the typed model drives placement.
    let p = generate(&s, END).await;
    contiguous(&p, &ids);
    assert_eq!(
        steps(&p).last().unwrap()["end"].as_u64().unwrap()
            - steps(&p)[0]["start"].as_u64().unwrap(),
        3606
    );
}

#[tokio::test]
async fn g5_mixed_model_sums_on_the_kernel_request() {
    let s = state().await;
    let o = origin(&s).await;
    let mut stochastic = child("B", 1);
    stochastic["duration_estimate"] = json!({"type":"shifted_lognormal_p95","min_seconds":101,"mode_seconds":302,"p95_seconds":903});
    stochastic["correlation_groups"] =
        json!([{"group":"shared","strength":0.8},{"group":"second","strength":0.3}]);
    stochastic["tags"] = json!(["work"]);
    stochastic["category_tag"] = json!("work");
    let mut first = child("A", 600);
    first["correlation_groups"] = json!([{"group":"shared","strength":0.2}]);
    first["allowed_time_range"] = range(NOW, "2026-09-27T12:00:00Z");
    stochastic["allowed_time_range"] = range("2026-09-27T10:00:00Z", END);
    stochastic["due_at"] = json!("2026-09-27T11:00:00Z");
    let d = decompose(&s, &o, vec![first, stochastic], json!([])).await;
    let ids = ids(&d);
    let request = planning_service::build_request_from_store(&s)
        .await
        .unwrap();
    let kernel = ubu_planning_core::PlanningRequest::from(request);
    assert_eq!(kernel.task_graph.tasks.len(), 1);
    let unit = &kernel.task_graph.tasks[0];
    assert_eq!(unit.correlation_groups.len(), 2);
    assert_eq!(
        unit.correlation_groups
            .iter()
            .find(|g| g.group == "shared")
            .unwrap()
            .strength,
        0.8
    );
    assert_eq!(
        unit.window.as_ref().unwrap().start,
        UbuTimestamp::parse("2026-09-27T10:00:00Z")
            .unwrap()
            .inner()
            .unix_timestamp() as u64
    );
    assert_eq!(
        unit.window.as_ref().unwrap().end,
        UbuTimestamp::parse("2026-09-27T11:00:00Z")
            .unwrap()
            .inner()
            .unix_timestamp() as u64
    );
    assert_eq!(
        serde_json::to_value(&kernel.task_graph.tasks[0].duration).unwrap(),
        json!({"type":"shifted_lognormal_p95","min_seconds":701,"mode_seconds":902,"p95_seconds":1503})
    );
    let p = generate(&s, END).await;
    contiguous(&p, &ids);
    assert_eq!(steps(&p)[1]["category_tag"], "work");
    assert_eq!(
        steps(&p).last().unwrap()["end"].as_u64().unwrap()
            - steps(&p)[0]["start"].as_u64().unwrap(),
        902
    );
    println!(
        "EVIDENCE[g5] kernel_duration={:?}",
        kernel.task_graph.tasks[0].duration
    );
}

#[tokio::test]
async fn g6_stale_version_rolls_back_everything() {
    let s = state().await;
    let o = origin(&s).await;
    task_capture::edit(&s, &o, 1, json!({"title":"Renamed gate"}))
        .await
        .unwrap();
    let before = snapshot(&s).await;
    let (status, v) = request(
        &s,
        "POST",
        &format!("/task/{o}/decompose"),
        body(1, children(), json!([])),
    )
    .await;
    assert_eq!(status, 409);
    assert_eq!(v["diagnostics"][0]["code"], "version_conflict");
    assert_eq!(snapshot(&s).await, before);
    assert_eq!(row(&s, &o).await.status, "active");
    println!("EVIDENCE[g6] status=409 unchanged=true");
}

async fn occurrence(s: &AppState) -> String {
    let id = UbuId::new(ObjectType::Task).to_string();
    let objective = UbuId::new(ObjectType::Objective);
    let p = json!({"id":id,"title":"Routine","status":"active","occurrence":{"routine_objective_id":objective,"local_date":"2026-09-27","key":format!("{objective}/s1/2026-09-27T09:00:00/planned/t1")},"provenance":{"created_at":NOW,"authority_source":"user"}});
    let e = s
        .envelope_for(
            [(UbuId::parse(&id).unwrap(), VersionRef::Absent)]
                .into_iter()
                .collect(),
            AuthoritySource::User,
            s.planning_now(),
        )
        .unwrap();
    queries::admit_object(
        s.inner().store.pool(),
        &e,
        NewObjectRecord {
            id: id.clone(),
            object_type: "Task".into(),
            version: 1,
            status: "active".into(),
            compartment_label: "test".into(),
            payload: p,
            created_at: NOW.into(),
            updated_at: NOW.into(),
        },
    )
    .await
    .unwrap();
    id
}

#[tokio::test]
async fn g7_all_eight_admission_rejections_leave_no_writes() {
    for code in [
        "decompose_routine_occurrence_unsupported",
        "decompose_inactive_task",
        "decompose_needs_children",
        "decompose_invalid_split_points",
        "decompose_child_order_conflict",
        "decompose_interior_precondition_needs_boundary",
        "decompose_static_child_needs_boundary",
        "decompose_segment_bounds_disjoint",
    ] {
        let s = state().await;
        let o = if code == "decompose_routine_occurrence_unsupported" {
            occurrence(&s).await
        } else {
            origin(&s).await
        };
        let mut cs = children();
        let mut split = json!([]);
        match code {
            "decompose_inactive_task" => change(&s, &o, "completed").await,
            "decompose_needs_children" => cs.truncate(1),
            "decompose_invalid_split_points" => split = json!([0]),
            "decompose_child_order_conflict" => cs[0]["blocked_by"] = json!(["child:2"]),
            "decompose_interior_precondition_needs_boundary" => {
                cs[1]["preconditions"] =
                    json!({"target":"numeric_values.ready","predicate":"equals","expected":1.0})
            }
            "decompose_static_child_needs_boundary" => {
                cs[1]["static_window"] = json!({"start":NOW,"end":"2026-09-27T09:30:00Z"})
            }
            "decompose_segment_bounds_disjoint" => {
                cs[0]["allowed_time_range"] = range(NOW, "2026-09-27T10:00:00Z");
                cs[1]["allowed_time_range"] = range("2026-09-27T11:00:00Z", END);
            }
            _ => {}
        }
        let before = snapshot(&s).await;
        let (status, v) = request(
            &s,
            "POST",
            &format!("/task/{o}/decompose"),
            body(1, cs, split),
        )
        .await;
        assert_eq!(status, 400, "{code}: {v}");
        assert_eq!(v["diagnostics"][0]["code"], code, "{v}");
        assert_eq!(snapshot(&s).await, before, "{code}");
    }
}

#[tokio::test]
async fn g8_completed_prefix_compiles_remaining_suffix() {
    let s = state().await;
    let o = origin(&s).await;
    let d = decompose(&s, &o, children(), json!([])).await;
    let ids = ids(&d);
    change(&s, &ids[0], "completed").await;
    let req = planning_service::build_request_from_store(&s)
        .await
        .unwrap();
    assert_eq!(req.tasks.len(), 1);
    assert_eq!(req.tasks[0].id, ids[1]);
    assert_eq!(req.tasks[0].duration, 3600);
    let p = generate(&s, END).await;
    contiguous(&p, &ids[1..]);
    assert!(!p["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["code"] == "container_segment_partial"));
}

#[tokio::test]
async fn g9_missing_middle_reports_partial_and_does_not_compile() {
    let s = state().await;
    let o = origin(&s).await;
    let d = decompose(&s, &o, children(), json!([])).await;
    let ids = ids(&d);
    change(&s, &ids[1], "completed").await;
    let req = planning_service::build_request_from_store(&s)
        .await
        .unwrap();
    assert_eq!(req.tasks.len(), 2);
    let p = generate(&s, END).await;
    let diagnostic = p["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["code"] == "container_segment_partial")
        .unwrap();
    let text = diagnostic["message"].as_str().unwrap();
    assert!(text.contains(d["container_id"].as_str().unwrap()));
    assert!(text.contains(&ids[1]));
    assert!(text.contains("segment 0"));
}

#[tokio::test]
async fn g10_outside_dependency_waits_for_whole_segment() {
    let s = state().await;
    let o = origin(&s).await;
    let d = decompose(&s, &o, children(), json!([])).await;
    let ids = ids(&d);
    let mut external = child("Inspect gate", 600);
    external["blocked_by"] = json!([ids[1]]);
    let outside = task(&s, external).await;
    for (a, b) in [(&ids[1], &outside), (&outside, &ids[0])] {
        let (status,value)=request(&s,"POST","/preference",json!({
            "schema_version":"ubu.orchestrator.preference.v1","task_a":a,"task_b":b,"order":"a_preferred_to_b"})).await;
        assert_eq!(status, 201, "{value}");
    }
    let req = planning_service::build_request_from_store(&s)
        .await
        .unwrap();
    assert_eq!(
        req.tasks
            .iter()
            .find(|t| t.id == outside)
            .unwrap()
            .depends_on,
        vec![ids[0].clone()]
    );
    assert_eq!(
        req.tasks.iter().find(|t| t.id == ids[0]).unwrap().value,
        1.0,
        "The most valuable removed member protects the whole segment"
    );
    assert!(req.tasks.iter().find(|t| t.id == outside).unwrap().value < 1.0);
    let p = generate(&s, END).await;
    contiguous(&p, &ids);
    let end = steps(&p).iter().find(|v| v["task_id"] == ids[2]).unwrap()["end"]
        .as_u64()
        .unwrap();
    let start = steps(&p).iter().find(|v| v["task_id"] == outside).unwrap()["start"]
        .as_u64()
        .unwrap();
    assert!(start >= end);
}

#[tokio::test]
async fn g11_undo_restores_new_handle_and_preserves_history() {
    let s = state().await;
    let o = origin(&s).await;
    let d = decompose(&s, &o, children(), json!([])).await;
    let ids = ids(&d);
    let origin_before = row(&s, &o).await;
    let u = undo(&s, d["container_id"].as_str().unwrap()).await;
    let restored = u["restored_task_id"].as_str().unwrap();
    assert_ne!(restored, o);
    assert!(!ids.contains(&restored.into()));
    let r = payload(&row(&s, restored).await);
    assert_eq!(r["title"], "Repair gate");
    assert_eq!(r["description"], "Keep the gate working");
    for id in &ids {
        assert_eq!(row(&s, id).await.status, "moot");
    }
    let c = payload(&row(&s, d["container_id"].as_str().unwrap()).await);
    assert_eq!(c["status"], "superseded");
    assert_eq!(c["superseded_by_task_ref"], restored);
    assert_eq!(row(&s, &o).await.payload_json, origin_before.payload_json);
    assert_eq!(row(&s, &o).await.version, origin_before.version);
    let p = generate(&s, END).await;
    assert!(steps(&p).iter().any(|v| v["task_id"] == restored));
    assert!(!steps(&p)
        .iter()
        .any(|v| ids.contains(&v["task_id"].as_str().unwrap().into())));

    // Valid at decomposition time, but too short to restore by undo time.
    let later_state = state().await;
    let objective = UbuId::new(ObjectType::Objective).to_string();
    let timed = task(
        &later_state,
        json!({"title":"Original timed intent","objective_id":objective,
        "duration_estimate":{"type":"fixed","seconds":3600},
        "allowed_time_range":{"earliest_start":NOW,"latest_finish":"2026-09-27T10:00:00Z"},
        "due_at":"2026-09-27T10:00:00Z"}),
    )
    .await;
    let d = decompose(&later_state, &timed, children(), json!([])).await;
    let later_state = later_state.with_clock(FixedClock(
        UbuTimestamp::parse("2026-09-27T09:45:00Z").unwrap(),
    ));
    let u = undo(&later_state, d["container_id"].as_str().unwrap()).await;
    let restored = payload(&row(&later_state, u["restored_task_id"].as_str().unwrap()).await);
    assert_eq!(restored["objective_id"], objective);
    assert_eq!(restored["duration_estimate"]["seconds"], 3600);
    assert!(restored.get("allowed_time_range").is_none());
    assert!(restored.get("due_at").is_none());
}

#[tokio::test]
async fn g12_undo_preserves_and_reports_completed_child() {
    let s = state().await;
    let o = origin(&s).await;
    let d = decompose(&s, &o, children(), json!([])).await;
    let ids = ids(&d);
    change(&s, &ids[0], "completed").await;
    let before = row(&s, &ids[0]).await;
    let u = undo(&s, d["container_id"].as_str().unwrap()).await;
    assert_eq!(u["children_left_completed"], json!([ids[0]]));
    assert_eq!(u["children_mooted"].as_array().unwrap().len(), 2);
    let after = row(&s, &ids[0]).await;
    assert_eq!(after.payload_json, before.payload_json);
    assert_eq!(after.version, before.version);
}

#[tokio::test]
async fn g13_list_derives_completion_and_reflects_undo() {
    let s = state().await;
    let o = origin(&s).await;
    let d = decompose(&s, &o, children(), json!([1])).await;
    let ids = ids(&d);
    let (status, v) = request(&s, "GET", "/containers", json!(null)).await;
    assert_eq!(status, 200);
    let c = &v["containers"][0];
    assert_eq!(c["origin_title"], "Repair gate");
    assert_eq!(c["completion_state"], "in_progress");
    assert_eq!(
        c["segments"],
        json!([{"start":0,"end":1},{"start":1,"end":3}])
    );
    for (i, id) in ids.iter().enumerate() {
        assert_eq!(c["children"][i]["id"], *id);
        assert_eq!(c["children"][i]["position"], i);
        assert_eq!(c["children"][i]["status"], "active");
    }
    let u = undo(&s, d["container_id"].as_str().unwrap()).await;
    let (_, v) = request(&s, "GET", "/containers", json!(null)).await;
    assert_eq!(v["containers"][0]["completion_state"], "complete");
    assert_eq!(v["containers"][0]["status"], "superseded");
    assert_eq!(
        v["containers"][0]["superseded_by_task_ref"],
        u["restored_task_id"]
    );
}

#[tokio::test]
async fn g14_routine_occurrence_rejected_without_admission() {
    let s = state().await;
    let o = occurrence(&s).await;
    let before = snapshot(&s).await;
    let (status, v) = request(
        &s,
        "POST",
        &format!("/task/{o}/decompose"),
        body(1, children(), json!([])),
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(
        v["diagnostics"][0]["code"],
        "decompose_routine_occurrence_unsupported"
    );
    assert_eq!(snapshot(&s).await, before);
}

async fn gaps(s: &AppState) {
    fixed(
        s,
        "Meeting A",
        "2026-09-27T09:30:00Z",
        "2026-09-27T10:00:00Z",
    )
    .await;
    fixed(
        s,
        "Meeting B",
        "2026-09-27T10:30:00Z",
        "2026-09-27T11:00:00Z",
    )
    .await;
}
#[tokio::test]
async fn g15_segment_cost_reports_every_member_with_split_remedy() {
    let s = state().await;
    let o = origin(&s).await;
    let d = decompose(&s, &o, children(), json!([])).await;
    let ids = ids(&d);
    gaps(&s).await;
    let p = generate(&s, "2026-09-27T11:30:00Z").await;
    for (i, id) in ids.iter().enumerate() {
        let u = p["unplaced_tasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["task_id"] == *id)
            .unwrap_or_else(|| panic!("{p}"));
        assert_eq!(u["summary"], children()[i]["title"]);
        assert_eq!(u["reason"], "no_eligible_chunk_large_enough");
        let explanation = u["explanation"].as_str().unwrap();
        assert!(explanation.contains(d["container_id"].as_str().unwrap()));
        assert!(explanation.to_lowercase().contains("split point"));
        assert!(explanation.contains("5400"));
    }
    let other = state().await;
    gaps(&other).await;
    let mut individual = Vec::new();
    for c in children() {
        individual.push(task(&other, c).await);
    }
    let p = generate(&other, "2026-09-27T11:30:00Z").await;
    for id in individual {
        assert!(steps(&p).iter().any(|v| v["task_id"] == id), "{p}");
    }
    println!("EVIDENCE[g15] compiled_members_unplaced=3 individual_members_placed=3");
}
