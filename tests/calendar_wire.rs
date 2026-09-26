//! All requests use axum::Router::oneshot or pure values; no transport is constructed.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    sync::{atomic::Ordering, Arc},
};
use tower::ServiceExt;
use ubu_orchestrator::{
    build_router,
    config::ServerConfig,
    services::{
        calendar_client::{CalendarExportMode, RecordingCalendarApi},
        calendar_projection::DesiredEvent,
        calendar_wire::*,
    },
    state::AppState,
};

fn event() -> DesiredEvent {
    DesiredEvent {
        external_id: "0123456789abcdef0123456789abcdef".into(),
        task_id: "task_0123456789abcdef0123456789abcdef".into(),
        summary: "Synthetic focus".into(),
        start_at: "2026-09-25T09:00:00Z".into(),
        end_at: "2026-09-25T09:30:00Z".into(),
        color_id: Some("5".into()),
        transparent: true,
        reminders_minutes: vec![10, 0],
    }
}
fn response_fixture() -> Value {
    serde_json::from_str(include_str!(
        "../fixtures/calendar/google-event-response.json"
    ))
    .unwrap()
}

#[test]
fn populated_body_matches_expected_fixture_bytes() {
    let event = event();
    let body = serde_json::to_vec(&event_body(&event)).unwrap();
    assert_eq!(
        body,
        include_bytes!("../fixtures/calendar/expected-event-body.json")
    );
    println!("P1B30_EXPECTED_BODY {}", String::from_utf8(body).unwrap());
    let expected: Value = serde_json::from_slice(include_bytes!(
        "../fixtures/calendar/expected-event-body.json"
    ))
    .unwrap();
    let patch = event_request(Operation::Patch, CALENDAR_API_BASE, "primary", &event);
    assert_eq!(patch.body, Some(expected.clone()));
    let insert = event_request(Operation::Insert, CALENDAR_API_BASE, "primary", &event);
    let mut insert_body = insert.body.unwrap();
    assert_eq!(
        insert_body.as_object_mut().unwrap().remove("id"),
        Some(json!(event.external_id))
    );
    assert_eq!(insert_body, expected);
    assert_eq!(
        patch.headers("SYNTHETIC_NOT_A_TOKEN"),
        vec![
            ("authorization", "Bearer SYNTHETIC_NOT_A_TOKEN".into()),
            ("accept", "application/json".into()),
            ("content-type", "application/json".into()),
        ]
    );
}

#[test]
fn absent_colour_is_omitted_and_empty_reminders_disable_defaults() {
    let mut event = event();
    event.color_id = None;
    event.reminders_minutes.clear();
    let value = serde_json::to_value(event_body(&event)).unwrap();
    assert!(value.get("colorId").is_none());
    assert_eq!(
        value["reminders"],
        json!({"useDefault":false,"overrides":[]})
    );
    let mut response = value;
    response["id"] = event.external_id.clone().into();
    assert_eq!(parse_event(&response).unwrap(), event);
    response["reminders"]
        .as_object_mut()
        .unwrap()
        .remove("overrides");
    assert_eq!(parse_event(&response).unwrap(), event);
}

#[test]
fn transparency_uses_google_strings() {
    for (transparent, expected) in [(true, "transparent"), (false, "opaque")] {
        let mut event = event();
        event.transparent = transparent;
        let mut body = serde_json::to_value(event_body(&event)).unwrap();
        assert_eq!(body["transparency"], expected);
        body["id"] = event.external_id.clone().into();
        assert_eq!(parse_event(&body).unwrap(), event);
        body.as_object_mut().unwrap().remove("transparency");
        assert!(!parse_event(&body).unwrap().transparent);
    }
}

#[test]
fn realistic_google_response_round_trips_and_required_fields_are_strict() {
    let response = response_fixture();
    assert_eq!(parse_event(&response).unwrap(), event());
    for field in ["id", "summary", "start", "end", "reminders"] {
        let mut malformed = response.clone();
        malformed.as_object_mut().unwrap().remove(field);
        assert!(parse_event(&malformed).is_err(), "{field}");
    }
    for (field, invalid) in [
        ("id", json!("bad/id")),
        ("summary", json!(null)),
        ("start", json!({"date":"2026-09-25"})),
        ("end", json!({"dateTime":"not a time"})),
        ("colorId", json!(null)),
        ("transparency", json!(false)),
        ("status", json!("cancelled")),
        ("reminders", json!({"useDefault":true})),
        (
            "reminders",
            json!({"useDefault":false,"overrides":[{"method":"email","minutes":10}]}),
        ),
        (
            "reminders",
            json!({"useDefault":false,"overrides":[{"method":"popup","minutes":-1}]}),
        ),
    ] {
        let mut malformed = response.clone();
        malformed[field] = invalid;
        assert!(parse_event(&malformed).is_err(), "{field}");
    }
    let mut backward = response;
    backward["end"] = backward["start"].clone();
    assert!(parse_event(&backward).is_err());
}

#[test]
fn event_list_skips_only_the_malformed_entry_and_paginates_safely() {
    let valid = response_fixture();
    let mut other = valid.clone();
    other["id"] = json!("00000000000000000000000000000002");
    let mut malformed = valid.clone();
    malformed["start"]["dateTime"] = json!("synthetic-invalid-time");
    let list = json!({"kind":"calendar#events","items":[valid,malformed,other],"nextPageToken":"synthetic/page+2"});
    let (events, messages) = parse_event_list(&list);
    assert_eq!(
        events,
        vec![event(), parse_event(&list["items"][2]).unwrap()]
    );
    assert_eq!(
        messages,
        vec!["list event `*` entry 1: invalid start.dateTime"]
    );
    assert_eq!(parse_event_list(&json!({"items":[]})), (vec![], vec![]));
    assert_eq!(parse_event_list(&json!({"items":false})).1.len(), 1);
    let mut seen = BTreeSet::new();
    assert_eq!(
        next_page(&list, &mut seen).unwrap().as_deref(),
        Some("synthetic/page+2")
    );
    assert_eq!(
        next_page(&list, &mut seen).unwrap_err(),
        "list event `*`: invalid or repeated nextPageToken"
    );
    assert_eq!(next_page(&json!({}), &mut seen).unwrap(), None);
    assert_eq!(
        next_page(&json!({"nextPageToken":""}), &mut seen).unwrap(),
        None
    );
    assert!(next_page(&json!({"nextPageToken":12}), &mut seen).is_err());
}

#[test]
fn urls_encode_both_ids_and_requests_choose_method_and_headers() {
    let base = CALENDAR_API_BASE;
    let calendar = "fake-calendar@example.invalid";
    assert_eq!(events_url(base, calendar, Some("retired/event")),
        "https://www.googleapis.com/calendar/v3/calendars/fake-calendar%40example.invalid/events/retired%2Fevent");
    assert_eq!(
        events_url(&format!("{base}/"), "fake/calendar", None),
        format!("{base}/fake%2Fcalendar/events")
    );
    assert_eq!(
        events_url(base, "fake ?#é", Some("a%2Fb")),
        format!("{base}/fake%20%3F%23%C3%A9/events/a%252Fb")
    );
    let range = ubu_orchestrator::services::calendar_range::CalendarTimeRange::parse("2026-09-25T08:00:00Z", "2026-09-25T20:00:00Z").unwrap();
    let request = list_request(base, calendar, &range, Some("synthetic/page+2"));
    assert!(request
        .url
        .ends_with("?singleEvents=true&timeMin=2026-09-25T08%3A00%3A00Z&timeMax=2026-09-25T20%3A00%3A00Z&pageToken=synthetic%2Fpage%2B2"));
    assert_eq!(request.operation.method(), "GET");
    assert!(request.body.is_none());
    assert_eq!(request.headers("SYNTHETIC_NOT_A_TOKEN").len(), 2);
    let delete = delete_request(base, calendar, "retired/event");
    assert_eq!(delete.operation.method(), "DELETE");
    assert_eq!(
        delete.url,
        events_url(base, calendar, Some("retired/event"))
    );
    assert!(delete.body.is_none());
    for (operation, method, id) in [
        (Operation::Insert, "POST", None),
        (Operation::Patch, "PATCH", Some(event().external_id)),
    ] {
        let request = event_request(operation, base, calendar, &event());
        assert_eq!(request.operation.method(), method);
        assert_eq!(request.url, events_url(base, calendar, id.as_deref()));
    }
}

#[test]
fn outcome_table_and_operation_specific_transitions() {
    let body = "synthetic provider failure";
    for (status, expected) in [
        (404, WireOutcome::AlreadyGone),
        (410, WireOutcome::AlreadyGone),
        (409, WireOutcome::Conflict),
        (200, WireOutcome::Ok),
        (204, WireOutcome::Ok),
        (400, WireOutcome::Failed(body.into())),
        (500, WireOutcome::Failed(body.into())),
    ] {
        let actual = outcome_for(status, body);
        assert_eq!(actual, expected);
        println!("P1B30_OUTCOME {status} | {body} | {actual:?}");
    }
    for status in 100..600 {
        for operation in [
            Operation::List,
            Operation::Insert,
            Operation::Patch,
            Operation::Delete,
        ] {
            let actual = response_action(operation, "synthetic-id", status, body);
            if (200..300).contains(&status)
                || (operation == Operation::Delete && matches!(status, 404 | 410))
            {
                assert_eq!(actual, ResponseAction::Done);
            } else if operation == Operation::Insert && status == 409 {
                assert_eq!(actual, ResponseAction::Patch);
            } else {
                assert_eq!(
                    actual,
                    ResponseAction::Failed(format!(
                        "{} event `synthetic-id`: Google Calendar returned HTTP {status}",
                        operation.name()
                    ))
                );
                assert!(!format!("{actual:?}").contains(body));
            }
        }
    }
}

async fn request(state: &AppState, path: &str, body: Value) -> (StatusCode, Value) {
    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn live_refusals_never_call_a_client_and_enablement_is_only_in_memory() {
    let base = ServerConfig::from_env(); // Run with the documented sanitized environment.
    assert!(base.google_credentials_path().is_none());
    assert!(base.google_token_cache_path().is_none());
    assert_eq!(base.google_calendar_id(), "primary");
    let configured = base
        .clone()
        .with_google_credentials_path("/synthetic-not-present/oauth.json")
        .with_google_token_cache_path("/synthetic-not-present/cache.json")
        .with_google_calendar_id("fake-calendar@example.invalid");
    assert_eq!(
        configured.google_calendar_id(),
        "fake-calendar@example.invalid"
    );
    assert_eq!(
        configured
            .google_credentials_path()
            .unwrap()
            .to_str()
            .unwrap(),
        "/synthetic-not-present/oauth.json"
    );
    assert_eq!(
        configured
            .google_token_cache_path()
            .unwrap()
            .to_str()
            .unwrap(),
        "/synthetic-not-present/cache.json"
    );
    for (config, status, diagnostic) in [
        (
            base,
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"code":"calendar_live_export_unconfigured","message":"Live Calendar export requires UBU_GOOGLE_CREDENTIALS_PATH and UBU_GOOGLE_TOKEN_CACHE_PATH"}),
        ),
        (
            configured.clone(),
            StatusCode::FORBIDDEN,
            json!({"code":"calendar_live_export_not_enabled","message":"Live Calendar export is not enabled for this process; POST /desktop/session/google-calendar first"}),
        ),
    ] {
        let client = Arc::new(RecordingCalendarApi::new());
        let state = AppState::in_memory(config)
            .await
            .unwrap()
            .with_calendar_api(client.clone());
        let (actual_status, body) = request(&state, "/projection/calendar/approve", json!({
            "schema_version":"ubu.orchestrator.calendar_projection_approval.v1", "preview_id":"synthetic-preview",
            "authority_source":"automation_worker", "export_mode":"live"
        })).await;
        assert_eq!(actual_status, status);
        assert_eq!(body["diagnostics"], json!([diagnostic.clone()]));
        println!("P1B30_REFUSAL {actual_status} {diagnostic}");
        assert!(client.recorded_calls().is_empty());
        for table in ["projection_results", "logs"] {
            let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(state.inner().store.pool())
                .await
                .unwrap();
            assert_eq!(count, 0);
        }
        CalendarExportMode::Mock.ensure_available(&state).unwrap();
        let (enable_status, _) = request(
            &state,
            "/desktop/session/google-calendar",
            json!({"schema_version":"ubu.orchestrator.desktop_session.v1"}),
        )
        .await;
        if status == StatusCode::SERVICE_UNAVAILABLE {
            assert_eq!(enable_status, StatusCode::SERVICE_UNAVAILABLE);
            assert!(!state
                .inner()
                .google_calendar_enabled
                .load(Ordering::Acquire));
        } else {
            assert_eq!(enable_status, StatusCode::OK);
            CalendarExportMode::Live
                .ensure_available(&state.clone())
                .unwrap();
            assert!(state
                .inner()
                .google_calendar_enabled
                .load(Ordering::Acquire));
            assert!(client.recorded_calls().is_empty());
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM logs")
                .fetch_one(state.inner().store.pool())
                .await
                .unwrap();
            assert_eq!(count, 0); // Enablement neither persists nor opens the fake files.
        }
    }
    let fresh = AppState::in_memory(configured).await.unwrap();
    assert!(!fresh
        .inner()
        .google_calendar_enabled
        .load(Ordering::Acquire));
    let (status, _) = request(&fresh, "/desktop/session/google-calendar", json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!fresh
        .inner()
        .google_calendar_enabled
        .load(Ordering::Acquire));
    assert!(CalendarExportMode::Live.ensure_available(&fresh).is_err());
}
