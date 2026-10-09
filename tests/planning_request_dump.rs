//! Synthetic state only; replay runs the kernel in memory, never a process or HTTP server.
#[path = "support/precondition_fixture.rs"]
mod fixture;
use serde_json::{json, Value};
use std::{fs, path::PathBuf};
use ubu_core::{ObjectType, UbuId, UbuTimestamp};
use ubu_orchestrator::{
    api::planning::{legitimization_report_body, GeneratePlanningRequest},
    config::ServerConfig,
    planning_time::FixedClock,
    services::planning_service,
    state::AppState,
};
use ubu_planning_core::{legitimization, PlanningRequest};
use ubu_planning_cpu::CpuStrategy;

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "synthetic-p80-replay-{}",
            UbuId::new(ObjectType::Plan)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn path(&self) -> PathBuf {
        self.0.join("request.json")
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

async fn state(path: Option<PathBuf>) -> AppState {
    let config = ServerConfig::from_env()
        .with_planner_strategy("greedy")
        .with_planning_request_dump(path);
    let state = AppState::in_memory(config)
        .await
        .unwrap()
        .with_clock(FixedClock(UbuTimestamp::parse(fixture::NOW).unwrap()));
    fixture::seed(&state,fixture::A,"active",json!({"duration_estimate":{"type":"fixed","seconds":120},"description":"Synthetic orbital teapot notes"})).await;
    state
}

#[tokio::test]
async fn store_dump_replays_exact_effective_input_affect_and_legitimization() {
    let scratch = Scratch::new();
    let state = state(Some(scratch.path())).await;
    let id = UbuId::new(ObjectType::Snapshot);
    let envelope = state
        .envelope_for(
            [(id.clone(), ubu_core::VersionRef::Absent)]
                .into_iter()
                .collect(),
            ubu_core::AuthoritySource::User,
            state.planning_now(),
        )
        .unwrap();
    ubu_store::queries::admit_object(state.inner().store.pool(),&envelope,ubu_store::models::object_record::NewObjectRecord {
        id:id.to_string(),object_type:ObjectType::Snapshot.as_str().into(),version:1,status:"active".into(),compartment_label:"synthetic-private-compartment".into(),
        payload:json!({"id":id,"captured_at":fixture::NOW,"objects":[],"affect":{"source_kind":"live_observation","observed_at":fixture::NOW,"dimensions":{"energy":{"value":9.123456789},"stress":{"value":1.23456789},"mood_intensity":{"value":1.875}}}}),
        created_at:fixture::NOW.into(),updated_at:fixture::NOW.into(),
    }).await.unwrap();
    let body = planning_service::build_request_from_store(&state)
        .await
        .unwrap();
    let mut expected = PlanningRequest::from(body.clone());
    expected.request_id = format!("store-{:016x}", expected.rng_seed);
    let response = planning_service::generate(
        state.clone(),
        serde_json::from_value::<GeneratePlanningRequest>(json!({"request":null})).unwrap(),
    )
    .await
    .unwrap();
    let serialized_response = serde_json::to_value(&response).unwrap();
    for key in ["unplaced_tasks", "blocked_tasks", "invalid_tasks"] {
        assert_eq!(
            serialized_response[key],
            json!([]),
            "empty sibling must be present: {key}"
        );
    }
    let bytes = fs::read(scratch.path()).unwrap();
    // This is the same deserializer as `ubu-planning-cli plan`.
    let replay: PlanningRequest = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(replay, expected);
    assert_eq!(replay.affect_profile, expected.affect_profile);
    assert_eq!(replay.affect_observation, expected.affect_observation);
    assert!(!replay
        .affect_observation
        .as_ref()
        .unwrap()
        .dimensions
        .is_empty());
    assert!(replay
        .affect_observation
        .as_ref()
        .unwrap()
        .dimensions
        .values()
        .all(|v| v.source_kind == "live_observation"));
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    for key in [
        "title",
        "summary",
        "event_id",
        "description",
        "affect_warning",
        "preconditions",
        "target",
    ] {
        assert!(value.get(key).is_none());
    }
    assert!(!String::from_utf8(bytes)
        .unwrap()
        .contains("Synthetic orbital teapot notes"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(scratch.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let planned = ubu_planning_core::plan(replay.clone(), &CpuStrategy);
    let selected = planned
        .plan_candidates
        .iter()
        .find(|p| p.rank == 1)
        .unwrap();
    let full = legitimization::full_legitimize(
        &selected.schedule,
        replay.affect_profile.as_ref(),
        replay.affect_observation.as_ref(),
    );
    let actual = serde_json::to_value(response.legitimization.unwrap()).unwrap();
    // The server's display-only warning is deliberately outside the kernel input.
    let expected =
        serde_json::to_value(legitimization_report_body(full.report, body.affect_warning)).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(
        response.selected_candidate.unwrap().candidate_id,
        selected.candidate_id
    );
}

#[tokio::test]
async fn dump_is_disabled_by_default_and_does_not_copy_supplied_requests() {
    let scratch = Scratch::new();
    let s = state(None).await;
    planning_service::generate(s, serde_json::from_value(json!({"request":null})).unwrap())
        .await
        .unwrap();
    assert!(!scratch.path().exists());
    let s = state(Some(scratch.path())).await;
    let body = planning_service::build_request_from_store(&s)
        .await
        .unwrap();
    planning_service::generate(s, serde_json::from_value(json!({"request":body})).unwrap())
        .await
        .unwrap();
    assert!(!scratch.path().exists());
}
