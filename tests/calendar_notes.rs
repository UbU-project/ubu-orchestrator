//! Invented notes only. Router requests are in process; Calendar and model are stubs.
#[path = "support/clarify_fixture.rs"]
mod fixture;
use fixture::{bare, configure, questions, request, Stub as StubTransport};
use serde_json::{json, Value};
use std::sync::Arc;
use ubu_orchestrator::{
    services::{
        calendar_client::{CalendarApi, RecordingCalendarApi},
        calendar_projection::{diff, DesiredEvent},
        calendar_reconcile,
        calendar_wire::*,
        clarify::MAX_DESCRIPTION_BYTES,
    },
    state::AppState,
};

fn wire_event(description: Option<Value>) -> Value {
    let mut event = json!({"id":"0inv3nt3dn0tes","summary":"Synthetic lunar kettle inspection","start":{"dateTime":"2026-09-29T09:00:00Z"},"end":{"dateTime":"2026-09-29T09:30:00Z"},"colorId":"3"});
    if let Some(description) = description {
        event["description"] = description;
    }
    event
}
async fn capture(state: &AppState) -> Value {
    let (status, body) = request(
        state,
        "POST",
        "/projection/calendar/capture",
        json!({"schema_version":"ubu.orchestrator.calendar_capture.v1","export_mode":"mock"}),
    )
    .await;
    assert_eq!(status, 200);
    body
}
async fn task(state: &AppState) -> Value {
    let raw: String =
        sqlx::query_scalar("SELECT payload_json FROM objects WHERE object_type='Task'")
            .fetch_one(state.inner().store.pool())
            .await
            .unwrap();
    serde_json::from_str(&raw).unwrap()
}

#[test]
fn parser_trims_notes_without_interpreting_markup_and_keeps_legacy_events_valid() {
    let event = parse_event(&wire_event(Some(json!(
        "  <b>Synthetic</b>\n\n orbital notes.  "
    ))))
    .unwrap();
    assert!(event.description.as_deref() == Some("<b>Synthetic</b>\n\n orbital notes."));
    for description in [None, Some(json!("")), Some(json!(" \n\t"))] {
        let event = parse_event(&wire_event(description)).unwrap();
        assert!(event.description.is_none());
        let serialized = serde_json::to_value(&event).unwrap();
        assert!(serialized.get("description").is_none());
        assert!(serde_json::from_value::<DesiredEvent>(serialized)
            .unwrap()
            .description
            .is_none());
    }
    for value in [Value::Null, json!(13), json!(false), json!({}), json!([])] {
        assert_eq!(
            parse_event(&wire_event(Some(value))).unwrap_err(),
            "invalid description"
        );
    }
}

#[test]
fn description_byte_bound_refuses_whole_text_and_reports_only_id_and_bound() {
    let exact = "é".repeat(MAX_DESCRIPTION_BYTES / 2);
    assert!(parse_event(&wire_event(Some(json!(exact))))
        .unwrap()
        .description
        .is_some());
    let too_long = format!("{exact}x");
    let (events, messages) =
        parse_event_list(&json!({"items":[wire_event(Some(json!(too_long)))]}));
    assert_eq!(events.len(), 1);
    assert!(events[0].description.is_none());
    assert_eq!(messages.len(), 1);
    assert!(
        messages[0].contains("0inv3nt3dn0tes")
            && messages[0].contains(&MAX_DESCRIPTION_BYTES.to_string())
    );
    assert!(!messages[0].contains('é'));
    assert_eq!(
        list_diagnostic(messages[0].clone()).code,
        DESCRIPTION_TOO_LARGE_CODE
    );
}

#[test]
fn insert_and_patch_omit_notes_and_notes_alone_are_never_projection_drift() {
    let mut old = parse_event(&wire_event(Some(json!("Synthetic orbital notes")))).unwrap();
    assert!(serde_json::to_value(event_body(&old))
        .unwrap()
        .get("description")
        .is_none());
    for operation in [Operation::Insert, Operation::Patch] {
        assert!(
            event_request(operation, CALENDAR_API_BASE, "synthetic", &old)
                .body
                .unwrap()
                .get("description")
                .is_none()
        );
    }
    let mut desired = old.clone();
    desired.description = None;
    assert!(diff(&[desired.clone()], &[old.clone()], &Default::default()).is_empty());
    old.description = Some("Different synthetic notes".into());
    assert!(calendar_reconcile::classify(&[desired], &[old], &Default::default(), &Default::default()).is_empty());
}

#[tokio::test]
async fn capture_imports_notes_or_omits_the_key_and_oversize_does_not_skip_the_task() {
    for (notes, expected, warnings) in [
        (
            Some(json!(" \nSynthetic orbital notes.\n ")),
            Some("Synthetic orbital notes."),
            0,
        ),
        (None, None, 0),
        (Some(json!(" \t\n")), None, 0),
        (Some(json!("q".repeat(MAX_DESCRIPTION_BYTES + 1))), None, 1),
    ] {
        let recorder = Arc::new(RecordingCalendarApi::with_wire_events(
            &json!({"items":[wire_event(notes)]}),
        ));
        let state = bare().await.with_calendar_api(recorder);
        let result = capture(&state).await;
        assert_eq!(result["captured"], 1);
        assert_eq!(result["skipped"], 0);
        let payload = task(&state).await;
        assert!(payload["description"].as_str() == expected);
        if expected.is_none() {
            assert!(payload.get("description").is_none());
        }
        assert!(payload["title"].as_str().is_some());
        assert_eq!(
            result["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|d| d["code"] == DESCRIPTION_TOO_LARGE_CODE)
                .count(),
            warnings
        );
    }
}

#[tokio::test]
async fn recapture_preserves_clarify_answers_and_reports_unchanged_even_when_event_notes_change() {
    for id in ["0inv3nt3dn0tes", "0inv3nt3dn0tes_20260929T090000Z"] {
        let mut input = wire_event(Some(json!("Synthetic imported note.")));
        input["id"] = json!(id);
        let recorder = Arc::new(RecordingCalendarApi::with_wire_events(
            &json!({"items":[input]}),
        ));
        let stub = StubTransport::answering([questions()]);
        let transport = stub.clone();
        let state = bare()
            .await
            .with_calendar_api(recorder.clone())
            .with_advisory_transport_factory(Arc::new(move |_| transport.clone()));
        configure(&state).await;
        assert_eq!(capture(&state).await["captured"], 1);
        let id = task(&state).await["id"].as_str().unwrap().to_owned();
        let (status, _) = request(&state,"POST","/advisory/run",json!({"schema_version":"ubu.orchestrator.advisory_run.v1","producer":"clarify","task_id":id})).await;
        assert_eq!(status, 200);
        let (_, queue) = request(&state, "GET", "/advisory/queue", Value::Null).await;
        let candidate = &queue["candidates"][0]["candidate"];
        let (status, _) = request(&state,"POST",&format!("/advisory/candidate/{}/answer",candidate["advisory_candidate_id"].as_str().unwrap()),json!({"observed_version":candidate["version"],"answers":{"q1":"y","q2":"synthetic Thursday"}})).await;
        assert_eq!(status, 200);
        let before = task(&state).await;
        assert!(before["description"].as_str().unwrap().contains("Q:"));
        let mut changed = recorder.events()[0].clone();
        changed.description = Some("Different synthetic calendar notes.".into());
        recorder.place_event(changed);
        let result = capture(&state).await;
        assert_eq!(result["updated"], 0);
        assert_eq!(result["unchanged"], 1);
        assert!(task(&state).await == before);
        assert!(!result["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "capture_owned_drift"));
    }
}

#[tokio::test]
async fn capture_fills_only_absent_or_whitespace_notes_on_an_existing_source() {
    let mut input = wire_event(None);
    input["id"] = json!("0inv3nt3dn0tes_20260929T090000Z");
    let recorder = Arc::new(RecordingCalendarApi::with_wire_events(
        &json!({"items":[input]}),
    ));
    let state = bare().await.with_calendar_api(recorder.clone());
    capture(&state).await;
    let id = task(&state).await["id"].as_str().unwrap().to_owned();
    let (status,_)=request(&state,"PATCH",&format!("/task/{id}"),json!({"schema_version":"ubu.orchestrator.task_capture.v1","expected_version":1,"description":" \n"})).await;
    assert_eq!(status, 200);
    let mut event = recorder.events()[0].clone();
    event.description = Some("Synthetic later notes.".into());
    recorder.place_event(event);
    assert_eq!(capture(&state).await["updated"], 1);
    assert!(task(&state).await["description"].as_str() == Some("Synthetic later notes."));
    assert_eq!(capture(&state).await["unchanged"], 1);
}

#[tokio::test]
async fn mock_writes_obey_wire_omission_and_patch_preserves_existing_calendar_notes() {
    let notes = Some("Synthetic calendar notes.".to_owned());
    let mut event = parse_event(&wire_event(Some(json!(notes)))).unwrap();
    let recorder = RecordingCalendarApi::with_events([event.clone()]);
    event.description = Some("Synthetic Task-only notes.".into());
    recorder.patch_event(&event).await.unwrap();
    assert!(recorder.events()[0].description == notes);
    assert!(serde_json::to_value(recorder.recorded_calls())
        .unwrap()
        .to_string()
        .find("description")
        .is_none());
    let insert = RecordingCalendarApi::new();
    insert.insert_event(&event).await.unwrap();
    assert!(insert.events()[0].description.is_none());
}
