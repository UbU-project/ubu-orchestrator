use serde_json::Value;
use ubu_core::{ObjectType, UbuId, UbuTimestamp};
use ubu_orchestrator::{
    api::{planning::TimeWindowBody, quick_ubu::QuickUbuImportRequest},
    config::ServerConfig,
    planning_time::FixedClock,
    services::{quick_ubu_import, routine_service::materialize},
    state::AppState,
};
fn sec(s: &str) -> u64 {
    UbuTimestamp::parse(s).unwrap().inner().unix_timestamp() as u64
}
async fn rows(state: &AppState) -> Vec<(String, i64, String)> {
    sqlx::query_as("SELECT id,version,status FROM objects WHERE json_extract(payload_json,'$.occurrence') IS NOT NULL ORDER BY id").fetch_all(state.inner().store.pool()).await.unwrap()
}
#[tokio::test]
async fn creates_ten_converges_and_marks_two_missed() {
    let state = AppState::in_memory(ServerConfig::from_env())
        .await
        .unwrap()
        .with_clock(FixedClock(
            UbuTimestamp::parse("2026-09-22T12:30:00Z").unwrap(),
        ));
    let path = std::env::temp_dir().join(format!("p1b19-{}.json", UbuId::new(ObjectType::Task)));
    std::fs::write(
        &path,
        include_str!("../fixtures/routines/snapshot-day.json"),
    )
    .unwrap();
    let imported = quick_ubu_import::import(
        state.clone(),
        QuickUbuImportRequest {
            snapshot_path: path.to_str().unwrap().into(),
            timezone: "America/New_York".into(),
            dry_run: false,
        },
    )
    .await
    .unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(imported.routines.created, 10);
    assert_eq!(imported.tasks.created, 2);
    let h = TimeWindowBody {
        start: sec("2026-09-22T12:30:00Z"),
        end: sec("2026-09-23T12:30:00Z"),
    };
    let first = materialize(&state, &h, h.start).await.unwrap();
    assert!(first.diagnostics.is_empty(), "{:?}", first.diagnostics);
    let before = rows(&state).await;
    assert_eq!(before.len(), 10);
    materialize(&state, &h, h.start).await.unwrap();
    assert_eq!(before, rows(&state).await);
    materialize(&state, &h, sec("2026-09-22T16:00:00Z"))
        .await
        .unwrap();
    assert_eq!(
        rows(&state)
            .await
            .iter()
            .filter(|r| r.2 == "failed")
            .count(),
        2
    );
    let logs: Vec<String> =
        sqlx::query_scalar("SELECT payload_json FROM logs WHERE event_type='task_failed'")
            .fetch_all(state.inner().store.pool())
            .await
            .unwrap();
    assert_eq!(logs.len(), 2);
    assert!(logs
        .iter()
        .all(|s| serde_json::from_str::<Value>(s).unwrap()["routine_outcome"] == "missed"));
    let titles: Vec<String> = sqlx::query_scalar(
        "SELECT json_extract(payload_json,'$.title') FROM objects WHERE status='failed' ORDER BY 1",
    )
    .fetch_all(state.inner().store.pool())
    .await
    .unwrap();
    assert_eq!(titles, vec!["Check-in 1", "Transparent reminder"]);
}
