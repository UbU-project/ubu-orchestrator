//! P1B-48 §D: undo of a completion made in the app. Synthetic and offline.
#[path = "support/clarify_fixture.rs"]
mod fixture;
use axum::http::StatusCode;
use fixture::*;
use serde_json::{json, Value};
use ubu_orchestrator::state::AppState;

const ACTION: &str = "ubu.orchestrator.task_action.v1";

async fn act(state: &AppState, id: &str, action: &str) -> Value {
    let (status, body) = request(state, "POST", &format!("/task/{id}/action"), json!({"schema_version":ACTION,"action":action})).await;
    assert_eq!(status, StatusCode::OK, "{action}: {body}");
    body
}
async fn reopen(state: &AppState, id: &str, completion: &str) -> (StatusCode, Value) {
    request(state, "POST", &format!("/task/{id}/reopen"), json!({"schema_version":ACTION,"completion_log_id":completion})).await
}
async fn decisions(state: &AppState, id: &str) -> Vec<Value> {
    let rows: Vec<(String, String, String)> = sqlx::query_as("SELECT id, object_refs_json, payload_json FROM logs WHERE event_type='decision_recorded' AND EXISTS (SELECT 1 FROM json_each(logs.object_refs_json) WHERE value=?) ORDER BY rowid")
        .bind(id).fetch_all(state.inner().store.pool()).await.unwrap();
    rows.into_iter()
        .map(|(id, refs, payload)| json!({"id":id,"object_refs":serde_json::from_str::<Value>(&refs).unwrap(),"payload":serde_json::from_str::<Value>(&payload).unwrap()}))
        .collect()
}
async fn universe(state: &AppState) -> Vec<(String, i64, String)> {
    sqlx::query_as("SELECT id, version, payload_json FROM objects WHERE object_type='UniverseState' ORDER BY id")
        .fetch_all(state.inner().store.pool())
        .await
        .unwrap()
}
fn code(body: &Value) -> &str {
    body["diagnostics"][0]["code"].as_str().unwrap_or("<none>")
}

#[tokio::test]
async fn completing_then_reopening_returns_the_task_to_active_and_records_which_completion_was_undone() {
    let state = bare().await;
    seed(&state, A, "active", json!({})).await;
    let completed = act(&state, A, "complete").await;
    assert_eq!(completed["task_status"], "completed");
    let completion = completed["log_id"].as_str().unwrap();
    assert_eq!(task(&state, A).await["__status"], "completed");

    let (status, body) = reopen(&state, A, completion).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["schema_version"], ACTION);
    assert_eq!(body["task_id"], A);
    assert_eq!(body["completion_log_id"], completion);
    assert_eq!(body["task_status"], "active");
    assert_eq!(body["diagnostics"], json!([]));
    let stored = task(&state, A).await;
    assert_eq!((stored["__status"].as_str(), stored["status"].as_str()), (Some("active"), Some("active")));
    assert_eq!(stored["__version"], 3);

    let log = decisions(&state, A).await;
    assert_eq!(log.len(), 2);
    assert_eq!(log[1]["id"], body["log_id"]);
    assert_eq!(log[1]["object_refs"], json!([A, completion]));
    assert_eq!(
        log[1]["payload"],
        json!({"schema_version":ACTION,"action":"reopen","decision":"task_reopened","task_status":"active","transition_applied":true,"completion_log_id":completion})
    );
    // This undo did not come from Google, and does not say it did.
    assert!(log[1]["payload"].get("source").is_none());
    println!("P1B48_D_REOPEN response={body} decision={}", log[1]["payload"]);
}

#[tokio::test]
async fn undoing_twice_a_wrong_completion_and_a_task_never_completed_are_each_refused() {
    let state = bare().await;
    seed(&state, A, "active", json!({})).await;
    seed(&state, B, "active", json!({})).await;
    let other = act(&state, B, "complete").await["log_id"].as_str().unwrap().to_owned();
    // Never completed.
    let (status, body) = reopen(&state, A, &other).await;
    assert_eq!((status, code(&body)), (StatusCode::CONFLICT, "reopen_not_completed"));
    let completion = act(&state, A, "complete").await["log_id"].as_str().unwrap().to_owned();
    let before = task(&state, A).await;
    let decisions_before = decisions(&state, A).await;
    // Another Task's completion, an id that names nothing, and a blank.
    for wrong in [other.as_str(), "log_018f3c8e9b2a7c4d8f1e2a3b4c5d6e7f", ""] {
        let (status, body) = reopen(&state, A, wrong).await;
        assert_eq!((status, code(&body)), (StatusCode::CONFLICT, "reopen_stale_completion"), "{wrong}");
        assert_eq!(task(&state, A).await, before);
        assert_eq!(decisions(&state, A).await, decisions_before);
    }
    let (status, _) = reopen(&state, A, &completion).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = reopen(&state, A, &completion).await;
    assert_eq!((status, code(&body)), (StatusCode::CONFLICT, "reopen_not_completed"));
    println!("P1B48_D_TWICE {body}");
    // Started is not completed, and a start is not a completion to undo.
    seed(&state, C, "active", json!({})).await;
    let started = act(&state, C, "start").await["log_id"].as_str().unwrap().to_owned();
    let (status, body) = reopen(&state, C, &started).await;
    assert_eq!((status, code(&body)), (StatusCode::CONFLICT, "reopen_not_completed"));
    // B was never touched by any of it.
    assert_eq!(task(&state, B).await["__status"], "completed");
}

#[tokio::test]
async fn a_reopened_task_can_be_completed_again_and_only_the_new_completion_can_be_undone() {
    let state = bare().await;
    seed(&state, A, "active", json!({})).await;
    let first = act(&state, A, "complete").await["log_id"].as_str().unwrap().to_owned();
    assert_eq!(reopen(&state, A, &first).await.0, StatusCode::OK);
    let again = act(&state, A, "complete").await;
    assert_eq!(again["task_status"], "completed");
    assert_eq!(again["transition_applied"], true);
    let second = again["log_id"].as_str().unwrap().to_owned();
    assert_ne!(second, first);
    let (status, body) = reopen(&state, A, &first).await;
    assert_eq!((status, code(&body)), (StatusCode::CONFLICT, "reopen_stale_completion"));
    assert_eq!(reopen(&state, A, &second).await.0, StatusCode::OK);
    assert_eq!(task(&state, A).await["__status"], "active");
    let actions: Vec<_> = decisions(&state, A).await.iter().map(|d| d["payload"]["action"].as_str().unwrap().to_owned()).collect();
    assert_eq!(actions, ["complete", "reopen", "complete", "reopen"]);
}

#[tokio::test]
async fn effects_are_not_reversed_and_the_reopen_says_so() {
    let state = bare().await;
    seed(&state, A, "active", json!({"effects":{"mutations":[{"operation":"set_fact","target":"facts.synthetic_teapot_bought","payload":true}]}})).await;
    seed(&state, B, "active", json!({})).await;
    seed(&state, C, "active", json!({"effects":{"mutations":[]}})).await;
    let completion = act(&state, A, "complete").await["log_id"].as_str().unwrap().to_owned();
    let applied = universe(&state).await;
    assert!(applied.iter().any(|(_, _, payload)| payload.contains("synthetic_teapot_bought")));
    let (status, body) = reopen(&state, A, &completion).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["diagnostics"],
        json!([{"code":"reopen_effects_not_reversed","message":"The Task is active again. The effects it applied when it completed were not reversed, and will not be applied a second time if it is completed again"}])
    );
    // Said, and true: the fact the completion set is still set.
    assert_eq!(universe(&state).await, applied);
    println!("P1B48_D_EFFECTS {}", body["diagnostics"]);
    // A Task with no effects, or with none listed, reports nothing.
    for id in [B, C] {
        let completion = act(&state, id, "complete").await["log_id"].as_str().unwrap().to_owned();
        let (status, body) = reopen(&state, id, &completion).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["diagnostics"], json!([]), "{id}");
    }
}

#[tokio::test]
async fn there_is_no_placement_restriction() {
    let state = bare().await;
    // A Static Task, and a Task captured from the calendar: the Calendar undo refuses both.
    seed(&state, A, "active", json!({"static_window":{"start":"2026-09-29T10:00:00Z","end":"2026-09-29T10:30:00Z"}})).await;
    seed(&state, B, "active", json!({"static_window":{"start":"2026-09-29T11:00:00Z","end":"2026-09-29T11:30:00Z"},"provenance":{"created_at":NOW,"authority_source":"user","source":{"source_kind":"google_calendar","source_id":"5n0q8c9h7g4k2m1p3r6t8v0a2c"}}})).await;
    for id in [A, B] {
        let completion = act(&state, id, "complete").await["log_id"].as_str().unwrap().to_owned();
        let (status, body) = reopen(&state, id, &completion).await;
        assert_eq!(status, StatusCode::OK, "{id}: {body}");
        assert_eq!(task(&state, id).await["__status"], "active");
    }
}

#[tokio::test]
async fn the_request_is_checked_before_anything_is_read() {
    let state = bare().await;
    seed(&state, A, "active", json!({})).await;
    let completion = act(&state, A, "complete").await["log_id"].as_str().unwrap().to_owned();
    for (body, expected) in [
        (json!({"completion_log_id":completion}), "missing_schema_version"),
        (json!({"schema_version":"synthetic.v0","completion_log_id":completion}), "unknown_schema_version"),
    ] {
        let (status, refused) = request(&state, "POST", &format!("/task/{A}/reopen"), body).await;
        assert_eq!((status, code(&refused)), (StatusCode::BAD_REQUEST, expected));
    }
    for malformed in [json!({"schema_version":ACTION}), json!({"schema_version":ACTION,"completion_log_id":completion,"note":"synthetic"})] {
        let (status, _) = request(&state, "POST", &format!("/task/{A}/reopen"), malformed).await;
        assert!(status.is_client_error());
    }
    let (status, _) = reopen(&state, "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e7f", &completion).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(task(&state, A).await["__status"], "completed");
}

// P1B-53 §B: effects apply once per Task. Complete, undo, complete used to apply
// a Task's UniverseState mutations twice, because reopen reverses nothing.
async fn completions_counted(state: &AppState) -> Value {
    universe(state)
        .await
        .first()
        .map(|(_, _, payload)| serde_json::from_str::<Value>(payload).unwrap()["numeric_values"]["synthetic_completions"].clone())
        .unwrap_or(Value::Null)
}
fn counting() -> Value {
    json!({"effects":{"mutations":[
        {"operation":"increment_numeric","target":"numeric_values.synthetic_completions","payload":1.0},
        {"operation":"set_fact","target":"facts.synthetic_teapot_bought","payload":true}
    ]}})
}

#[tokio::test]
async fn effects_apply_once_per_task_across_an_undo_and_the_second_completion_says_so() {
    let state = bare().await;
    seed(&state, A, "active", counting()).await;
    // The first completion applies the effects, and says nothing about it.
    let first = act(&state, A, "complete").await;
    assert_eq!(first["task_status"], "completed");
    assert_eq!(first["transition_applied"], true);
    assert_eq!(first["diagnostics"], json!([]));
    assert_eq!(completions_counted(&state).await, json!(1.0));
    let after_first = universe(&state).await;

    // The undo reverses nothing, and says that a second completion will not apply them again.
    let (status, undone) = reopen(&state, A, first["log_id"].as_str().unwrap()).await;
    assert_eq!(status, StatusCode::OK, "{undone}");
    assert_eq!(code(&undone), "reopen_effects_not_reversed");
    assert!(undone["diagnostics"][0]["message"].as_str().unwrap().contains("will not be applied a second time"));
    assert_eq!(universe(&state).await, after_first);

    // The second completion still completes the Task, and applies nothing.
    let second = act(&state, A, "complete").await;
    assert_eq!(second["task_status"], "completed");
    assert_eq!(second["transition_applied"], true);
    assert_eq!(task(&state, A).await["__status"], "completed");
    assert_ne!(second["log_id"], first["log_id"]);
    assert_eq!(
        second["diagnostics"],
        json!([{"code":"task_effects_already_applied","message":format!("Task `{A}` completed before, so its recorded effects were not applied a second time")}])
    );
    // Once: the count is 1 and not 2, and UniverseState has not moved at all, not even a version.
    assert_eq!(completions_counted(&state).await, json!(1.0));
    assert_eq!(universe(&state).await, after_first);

    // And again: a third completion after a second undo is the same.
    let (_, undone) = reopen(&state, A, second["log_id"].as_str().unwrap()).await;
    assert_eq!(code(&undone), "reopen_effects_not_reversed");
    let third = act(&state, A, "complete").await;
    assert_eq!(code(&third), "task_effects_already_applied");
    assert_eq!(universe(&state).await, after_first);
    println!("P1B53_B={}", json!({"first":first["diagnostics"],"undo":undone["diagnostics"],"second":second["diagnostics"],"synthetic_completions":completions_counted(&state).await}));
}

#[tokio::test]
async fn a_task_with_no_effects_never_reports_them_and_another_tasks_effects_still_apply() {
    let state = bare().await;
    seed(&state, A, "active", counting()).await;
    seed(&state, B, "active", json!({})).await;
    seed(&state, C, "active", json!({"effects":{"mutations":[]}})).await;
    // No effects, or none listed: complete, undo, complete says nothing at any point.
    for id in [B, C] {
        let first = act(&state, id, "complete").await;
        assert_eq!(first["diagnostics"], json!([]), "{id}");
        let (_, undone) = reopen(&state, id, first["log_id"].as_str().unwrap()).await;
        assert_eq!(undone["diagnostics"], json!([]), "{id}");
        let second = act(&state, id, "complete").await;
        assert_eq!(second["diagnostics"], json!([]), "{id}");
        assert_eq!(second["task_status"], "completed");
    }
    assert!(universe(&state).await.is_empty(), "nothing was applied, so no UniverseState was seeded");
    // Once per Task, not once ever: a different Task's effects apply on its own first completion.
    assert_eq!(act(&state, A, "complete").await["diagnostics"], json!([]));
    assert_eq!(completions_counted(&state).await, json!(1.0));
}
