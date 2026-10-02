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
    // P1B-57: an insert also stamps the Task it mints the event for, and nothing else differs.
    assert_eq!(
        insert_body.as_object_mut().unwrap().remove("extendedProperties"),
        Some(json!({"private": {"ubu_task": event.task_id}}))
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

// P1B-57 §A. Only an insert stamps. A PATCH is also what UbU sends to an event the
// operator made and UbU captured: stamping it would mark his own event as UbU's, and a
// fresh store would then discard his commitment.
#[test]
fn a_patch_body_carries_no_extended_properties_and_an_insert_carries_the_stamp() {
    // An event UbU minted for its own Task, and one the operator made that UbU captured:
    // that one keeps his id, and its Task has a handle of its own.
    let minted = event();
    let mut captured = event();
    captured.external_id = "0inv3nt3dkett1edescaling".into();
    captured.task_id = "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e70".into();
    let mut uncoloured = event();
    uncoloured.color_id = None;
    for event in [&minted, &captured, &uncoloured] {
        let patch = event_request(Operation::Patch, CALENDAR_API_BASE, "primary", event)
            .body
            .unwrap();
        assert!(patch.get("extendedProperties").is_none(), "{patch}");
        assert!(patch.get("id").is_none(), "{patch}");
        assert!(!patch.to_string().contains(UBU_TASK_PROPERTY), "{patch}");
    }
    // The insert's stamp is the Task the event is minted for, under the private key.
    let insert = event_request(Operation::Insert, CALENDAR_API_BASE, "primary", &minted)
        .body
        .unwrap();
    assert_eq!(UBU_TASK_PROPERTY, "ubu_task");
    assert_eq!(
        insert["extendedProperties"],
        json!({"private": {"ubu_task": "task_0123456789abcdef0123456789abcdef"}})
    );
    assert_eq!(insert["id"], "0123456789abcdef0123456789abcdef");
    println!("P1B57_A_INSERT_BODY {insert}");
    // List and delete requests have no body at all.
    assert!(delete_request(CALENDAR_API_BASE, "primary", &minted.external_id).body.is_none());
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
    for field in ["id", "summary", "start", "end"] {
        let mut malformed = response.clone();
        malformed.as_object_mut().unwrap().remove(field);
        assert!(parse_event(&malformed).is_err(), "{field}");
    }
    for (field, invalid) in [
        ("id", json!(42)),
        ("summary", json!(null)),
        ("start", json!({"date":"2026-09-25"})),
        ("end", json!({"dateTime":"not a time"})),
        ("colorId", json!(null)),
        ("transparency", json!(false)),
        ("status", json!("cancelled")),
        ("reminders", json!({"useDefault":"invalid"})),
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
    for reminders in [None, Some(json!({"useDefault":true}))] {
        let mut ordinary = response.clone();
        ordinary.as_object_mut().unwrap().remove("reminders");
        if let Some(reminders) = reminders { ordinary["reminders"] = reminders; }
        assert!(parse_event(&ordinary).unwrap().reminders_minutes.is_empty());
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
        vec!["list event `0123456789abcdef0123456789abcdef` entry 1: invalid start.dateTime"]
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

// ---- P1B-57 §B: reading the stamp back.

/// A Google list item with only what the reader looks at, plus whatever is given.
fn listed(id: &str, extra: Value) -> Value {
    let mut item = json!({"id": id, "summary": "Synthetic event", "start": {"dateTime": "2026-09-25T09:00:00Z"}, "end": {"dateTime": "2026-09-25T09:30:00Z"}, "reminders": {"useDefault": false}});
    item.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    item
}
fn stamped(task: &str) -> Value {
    json!({"extendedProperties": {"private": {"ubu_task": task}}})
}

#[test]
fn the_reader_keeps_only_an_id_whose_stamp_names_its_own_task() {
    const MINTED: &str = "018f3c8e9b2a7c4d8f1e2a3b4c5d6e70";
    const OPERATORS: &str = "018f3c8e9b2a7c4d8f1e2a3b4c5d6e71";
    const MISNAMED: &str = "018f3c8e9b2a7c4d8f1e2a3b4c5d6e72";
    let list = json!({"items": [
        // UbU minted this one: the stamp names the Task whose handle the id is.
        listed(MINTED, stamped(&format!("task_{MINTED}"))),
        // The same shape of id with no stamp: the operator's, or UbU's from before P1B-57.
        listed(OPERATORS, json!({})),
        // A stamp that names some other Task is evidence of nothing.
        listed(MISNAMED, stamped(&format!("task_{MINTED}"))),
    ]});
    assert_eq!(ubu_created_ids(&list), BTreeSet::from([MINTED.to_owned()]));
    // The events themselves are read as before: the stamp is not part of an event.
    let (events, skipped) = parse_event_list(&list);
    assert_eq!(events.iter().map(|e| e.external_id.as_str()).collect::<Vec<_>>(), [MINTED, OPERATORS, MISNAMED]);
    assert!(skipped.is_empty());
}

#[test]
fn the_reader_tolerates_every_absence_by_returning_fewer_ids() {
    const ID: &str = "018f3c8e9b2a7c4d8f1e2a3b4c5d6e70";
    let own = format!("task_{ID}");
    for (what, value) in [
        ("no items", json!({})),
        ("items is not a list", json!({"items": {"id": ID}})),
        ("an empty list", json!({"items": []})),
        ("an item that is not an object", json!({"items": ["synthetic", 7, null]})),
        ("no id", json!({"items": [stamped(&own)]})),
        ("an id that is not a string", json!({"items": [{"id": 7, "extendedProperties": {"private": {"ubu_task": own}}}]})),
        ("no extendedProperties", json!({"items": [listed(ID, json!({}))]})),
        ("extendedProperties is not an object", json!({"items": [listed(ID, json!({"extendedProperties": "synthetic"}))]})),
        ("no private", json!({"items": [listed(ID, json!({"extendedProperties": {"shared": {"ubu_task": own}}}))]})),
        ("private is not an object", json!({"items": [listed(ID, json!({"extendedProperties": {"private": [own]}}))]})),
        ("another private property", json!({"items": [listed(ID, json!({"extendedProperties": {"private": {"synthetic": own}}}))]})),
        ("a stamp that is not a string", json!({"items": [listed(ID, json!({"extendedProperties": {"private": {"ubu_task": 7}}}))]})),
        ("a stamp without the prefix", json!({"items": [listed(ID, stamped(ID))]})),
        ("an empty stamp", json!({"items": [listed(ID, stamped(""))]})),
    ] {
        assert_eq!(ubu_created_ids(&value), BTreeSet::new(), "{what}");
    }
    // One malformed neighbour does not cost the others.
    let mixed = json!({"items": ["synthetic", listed(ID, stamped(&own)), {"id": 7}]});
    assert_eq!(ubu_created_ids(&mixed), BTreeSet::from([ID.to_owned()]));
    // An entry the event parser skips can still be recognised: the two passes are independent.
    let all_day = json!({"items": [{"id": ID, "summary": "Synthetic all day", "start": {"date": "2026-09-25"}, "end": {"date": "2026-09-26"}, "extendedProperties": {"private": {"ubu_task": own}}}]});
    assert_eq!(ubu_created_ids(&all_day), BTreeSet::from([ID.to_owned()]));
    assert!(parse_event_list(&all_day).0.is_empty());
}

#[test]
fn an_inserts_body_read_back_through_the_reader_yields_that_events_id() {
    // §A and §B agreeing: the one assertion that fails if either side's key or shape drifts.
    let event = event();
    let insert = event_request(Operation::Insert, CALENDAR_API_BASE, "primary", &event).body.unwrap();
    // Google returns what was inserted, so the insert body stands for the listed item.
    assert_eq!(ubu_created_ids(&json!({"items": [insert]})), BTreeSet::from([event.external_id.clone()]));
    // What a PATCH sends, listed back, is recognised as nobody's.
    let mut patched = event_request(Operation::Patch, CALENDAR_API_BASE, "primary", &event).body.unwrap();
    patched["id"] = json!(event.external_id);
    assert_eq!(ubu_created_ids(&json!({"items": [patched]})), BTreeSet::new());
    // An event UbU re-creates for a captured Task keeps the operator's id, so its stamp
    // names a Task that id is not the handle of: it is not read as UbU-minted.
    let mut recreated = event.clone();
    recreated.external_id = "0inv3nt3dkett1edescaling".into();
    let insert = event_request(Operation::Insert, CALENDAR_API_BASE, "primary", &recreated).body.unwrap();
    assert_eq!(ubu_created_ids(&json!({"items": [insert]})), BTreeSet::new());
}

#[tokio::test]
async fn the_recording_client_reads_stamps_through_the_reader_and_drains_them_after_a_list() {
    use ubu_orchestrator::services::{calendar_client::CalendarApi, calendar_range::CalendarTimeRange};
    const MINTED: &str = "018f3c8e9b2a7c4d8f1e2a3b4c5d6e70";
    const OPERATORS: &str = "018f3c8e9b2a7c4d8f1e2a3b4c5d6e71";
    let list = json!({"items": [listed(MINTED, stamped(&format!("task_{MINTED}"))), listed(OPERATORS, json!({}))]});
    let range = CalendarTimeRange::parse("2026-09-25T00:00:00Z", "2026-09-26T00:00:00Z").unwrap();
    let recorder = RecordingCalendarApi::with_wire_events(&list);
    // Nothing is known before a list, as with diagnostics.
    assert!(recorder.take_ubu_created_ids().await.is_empty());
    assert_eq!(recorder.list_events(&range).await.unwrap().len(), 2);
    assert_eq!(recorder.take_ubu_created_ids().await, BTreeSet::from([MINTED.to_owned()]));
    // Drained: a second take with no list between them is empty.
    assert!(recorder.take_ubu_created_ids().await.is_empty());
    // The stamp stays on the event, as it does on the calendar: the next list finds it again.
    recorder.list_events(&range).await.unwrap();
    assert_eq!(recorder.take_ubu_created_ids().await, BTreeSet::from([MINTED.to_owned()]));
    // An event out of the listed range is not reported.
    let elsewhere = CalendarTimeRange::parse("2026-10-25T00:00:00Z", "2026-10-26T00:00:00Z").unwrap();
    recorder.list_events(&elsewhere).await.unwrap();
    assert!(recorder.take_ubu_created_ids().await.is_empty());

    // The builder stands in for a stamp without a synthetic list.
    let built = RecordingCalendarApi::with_events([event()]).with_ubu_created_ids([event().external_id]);
    built.list_events(&range).await.unwrap();
    assert_eq!(built.take_ubu_created_ids().await, BTreeSet::from([event().external_id]));

    // An insert stamps, a patch does not, and a delete takes the stamp with the event.
    let fresh = RecordingCalendarApi::new();
    let mut captured = event();
    captured.external_id = "0inv3nt3dkett1edescaling".into();
    fresh.insert_event(&event()).await.unwrap();
    fresh.insert_event(&captured).await.unwrap();
    fresh.patch_event(&event()).await.unwrap();
    fresh.list_events(&range).await.unwrap();
    assert_eq!(fresh.take_ubu_created_ids().await, BTreeSet::from([event().external_id]));
    fresh.delete_event(&event().external_id).await.unwrap();
    fresh.list_events(&range).await.unwrap();
    assert!(fresh.take_ubu_created_ids().await.is_empty());
}

#[tokio::test]
async fn a_client_that_does_not_know_reports_no_minted_ids() {
    use ubu_orchestrator::services::{
        calendar_client::{CalendarApi, CalendarApiFuture},
        calendar_range::CalendarTimeRange,
    };
    // The trait's default: "nothing is known to be UbU's".
    struct Bare;
    impl CalendarApi for Bare {
        fn list_events<'a>(&'a self, _: &'a CalendarTimeRange) -> CalendarApiFuture<'a, Vec<DesiredEvent>> { Box::pin(async { Ok(vec![event()]) }) }
        fn insert_event<'a>(&'a self, _: &'a DesiredEvent) -> CalendarApiFuture<'a, ()> { Box::pin(async { Ok(()) }) }
        fn patch_event<'a>(&'a self, _: &'a DesiredEvent) -> CalendarApiFuture<'a, ()> { Box::pin(async { Ok(()) }) }
        fn delete_event<'a>(&'a self, _: &'a str) -> CalendarApiFuture<'a, ()> { Box::pin(async { Ok(()) }) }
    }
    let range = CalendarTimeRange::parse("2026-09-25T00:00:00Z", "2026-09-26T00:00:00Z").unwrap();
    Bare.list_events(&range).await.unwrap();
    assert!(Bare.take_ubu_created_ids().await.is_empty());
    assert!(Bare.take_diagnostics().await.is_empty());
}
