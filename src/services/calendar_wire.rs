//! Pure Google Calendar request, response and status policy. No transport types.
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;

use super::calendar_projection::{external_id, DesiredEvent};

pub const CALENDAR_SCOPE: &str = "https://www.googleapis.com/auth/calendar";
pub const CALENDAR_API_BASE: &str = "https://www.googleapis.com/calendar/v3/calendars";

// Field order and omission rules match quick-ubu/gcal's GoogleEventBody.
#[derive(Debug, Serialize)]
pub struct GoogleEventBody {
    pub summary: String,
    pub start: GoogleEventTime,
    pub end: GoogleEventTime,
    #[serde(rename = "colorId", skip_serializing_if = "Option::is_none")]
    pub color_id: Option<String>,
    pub transparency: String,
    pub reminders: GoogleReminders,
}

#[derive(Debug, Serialize)]
pub struct GoogleReminders {
    #[serde(rename = "useDefault")]
    pub use_default: bool,
    pub overrides: Vec<GoogleReminder>,
}

#[derive(Debug, Serialize)]
pub struct GoogleReminder {
    pub method: &'static str,
    // DesiredEvent uses i64; preserve it without narrowing or truncation.
    pub minutes: i64,
}

#[derive(Debug, Serialize)]
pub struct GoogleEventTime {
    #[serde(rename = "dateTime")]
    pub date_time: String,
}

pub fn event_body(event: &DesiredEvent) -> GoogleEventBody {
    GoogleEventBody {
        summary: event.summary.clone(),
        start: GoogleEventTime {
            date_time: event.start_at.clone(),
        },
        end: GoogleEventTime {
            date_time: event.end_at.clone(),
        },
        color_id: event.color_id.clone(),
        transparency: if event.transparent {
            "transparent"
        } else {
            "opaque"
        }
        .into(),
        reminders: GoogleReminders {
            use_default: false,
            overrides: event
                .reminders_minutes
                .iter()
                .map(|&minutes| GoogleReminder {
                    method: "popup",
                    minutes,
                })
                .collect(),
        },
    }
}

fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing or invalid {field}"))
}

pub fn parse_event(value: &Value) -> Result<DesiredEvent, String> {
    if value.get("status").and_then(Value::as_str) == Some("cancelled") {
        return Err("cancelled event".into());
    }
    let id = required_string(value, "id")?;
    let task_id = format!("task_{id}");
    if external_id(&task_id).as_deref() != Some(id) {
        return Err("event id cannot map to a Task".into());
    }
    let summary = required_string(value, "summary")?;
    let start_at = required_string(&value["start"], "dateTime")?;
    let end_at = required_string(&value["end"], "dateTime")?;
    let start =
        chrono::DateTime::parse_from_rfc3339(start_at).map_err(|_| "invalid start.dateTime")?;
    let end = chrono::DateTime::parse_from_rfc3339(end_at).map_err(|_| "invalid end.dateTime")?;
    if end <= start {
        return Err("end.dateTime must follow start.dateTime".into());
    }
    let color_id = match value.get("colorId") {
        None => None,
        Some(Value::String(color)) => Some(color.clone()),
        _ => return Err("invalid colorId".into()),
    };
    // Like Quick UbU, an absent transparency means opaque.
    let transparent = match value.get("transparency") {
        None => false,
        Some(Value::String(transparency)) => transparency == "transparent",
        _ => return Err("invalid transparency".into()),
    };
    let reminders = &value["reminders"];
    if reminders["useDefault"] != false {
        return Err("reminders.useDefault must be false to recover explicit reminders".into());
    }
    let overrides = match reminders.get("overrides") {
        None => &[][..], // Google may omit an empty overrides array.
        Some(Value::Array(overrides)) => overrides.as_slice(),
        _ => return Err("invalid reminders.overrides".into()),
    };
    let reminders_minutes = overrides
        .iter()
        .map(|reminder| {
            if reminder["method"] != "popup" {
                return Err("unsupported reminder method".into());
            }
            reminder["minutes"]
                .as_i64()
                .filter(|minutes| *minutes >= 0)
                .ok_or_else(|| "invalid reminder minutes".into())
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(DesiredEvent {
        external_id: id.into(),
        task_id,
        summary: summary.into(),
        start_at: start_at.into(),
        end_at: end_at.into(),
        color_id,
        transparent,
        reminders_minutes,
    })
}

pub fn parse_event_list(value: &Value) -> (Vec<DesiredEvent>, Vec<String>) {
    let Some(items) = value.get("items").and_then(Value::as_array) else {
        return (
            Vec::new(),
            vec!["list event `*`: missing or invalid items".into()],
        );
    };
    let mut events = Vec::new();
    let mut messages = Vec::new();
    for (index, item) in items.iter().enumerate() {
        match parse_event(item) {
            Ok(event) => events.push(event),
            // Do not echo untrusted response content into diagnostics.
            Err(message) => messages.push(format!("list event `*` entry {index}: {message}")),
        }
    }
    (events, messages)
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write;
            write!(encoded, "%{byte:02X}").expect("writing to a String cannot fail");
        }
    }
    encoded
}

pub fn events_url(base: &str, calendar_id: &str, event_id: Option<&str>) -> String {
    let mut url = format!(
        "{}/{}/events",
        base.trim_end_matches('/'),
        percent_encode(calendar_id)
    );
    if let Some(id) = event_id {
        url.push('/');
        url.push_str(&percent_encode(id));
    }
    url
}

#[derive(Debug, PartialEq, Eq)]
pub enum WireOutcome {
    Ok,
    AlreadyGone,
    Conflict,
    Failed(String),
}

pub fn outcome_for(status: u16, body: &str) -> WireOutcome {
    match status {
        200..=299 => WireOutcome::Ok,
        404 | 410 => WireOutcome::AlreadyGone,
        409 => WireOutcome::Conflict,
        _ => WireOutcome::Failed(body.into()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    List,
    Insert,
    Patch,
    Delete,
}
impl Operation {
    pub fn name(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Insert => "insert",
            Self::Patch => "patch",
            Self::Delete => "delete",
        }
    }
    pub fn method(self) -> &'static str {
        match self {
            Self::List => "GET",
            Self::Insert => "POST",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }
}

#[derive(Debug)]
pub struct WireRequest {
    pub operation: Operation,
    pub event_id: String,
    pub url: String,
    pub body: Option<Value>,
}
impl WireRequest {
    pub fn headers(&self, token: &str) -> Vec<(&'static str, String)> {
        let mut headers = vec![
            ("authorization", format!("Bearer {token}")),
            ("accept", "application/json".into()),
        ];
        if self.body.is_some() {
            headers.push(("content-type", "application/json".into()));
        }
        headers
    }
}

pub fn event_request(
    operation: Operation,
    base: &str,
    calendar_id: &str,
    event: &DesiredEvent,
) -> WireRequest {
    let mut body =
        serde_json::to_value(event_body(event)).expect("GoogleEventBody is serializable");
    // The writable fields remain identical to Quick UbU. Insert additionally
    // supplies P1B-28's deterministic id instead of accepting a random Google id.
    if operation == Operation::Insert {
        body["id"] = event.external_id.clone().into();
    }
    WireRequest {
        operation,
        event_id: event.external_id.clone(),
        url: events_url(
            base,
            calendar_id,
            (operation != Operation::Insert).then_some(event.external_id.as_str()),
        ),
        body: Some(body),
    }
}

pub fn delete_request(base: &str, calendar_id: &str, event_id: &str) -> WireRequest {
    WireRequest {
        operation: Operation::Delete,
        event_id: event_id.into(),
        url: events_url(base, calendar_id, Some(event_id)),
        body: None,
    }
}

pub fn list_request(base: &str, calendar_id: &str, range: &super::calendar_range::CalendarTimeRange, page_token: Option<&str>) -> WireRequest {
    let mut url = format!("{}?singleEvents=true&timeMin={}&timeMax={}", events_url(base, calendar_id, None), percent_encode(&range.start.to_string()), percent_encode(&range.end.to_string()));
    if let Some(token) = page_token {
        url.push_str(&format!("&pageToken={}", percent_encode(token)));
    }
    WireRequest {
        operation: Operation::List,
        event_id: "*".into(),
        url,
        body: None,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ResponseAction {
    Done,
    Patch,
    Failed(String),
}

pub fn operation_error(operation: Operation, event_id: &str, reason: &str) -> String {
    format!("{} event `{event_id}`: {reason}", operation.name())
}

pub fn response_action(
    operation: Operation,
    event_id: &str,
    status: u16,
    body: &str,
) -> ResponseAction {
    match (operation, outcome_for(status, body)) {
        (_, WireOutcome::Ok) | (Operation::Delete, WireOutcome::AlreadyGone) => {
            ResponseAction::Done
        }
        (Operation::Insert, WireOutcome::Conflict) => ResponseAction::Patch,
        // A provider body can contain secrets. Preserve it in the pure outcome
        // for inspection, but never send it to a response, log or StateStore.
        _ => ResponseAction::Failed(operation_error(
            operation,
            event_id,
            &format!("Google Calendar returned HTTP {status}"),
        )),
    }
}

/// Pagination policy, including Quick UbU's repeated-token guard.
pub fn next_page(value: &Value, seen: &mut BTreeSet<String>) -> Result<Option<String>, String> {
    match value.get("nextPageToken") {
        None => Ok(None),
        Some(Value::String(token)) if token.is_empty() => Ok(None),
        Some(Value::String(token)) if seen.insert(token.clone()) => Ok(Some(token.clone())),
        _ => Err("list event `*`: invalid or repeated nextPageToken".into()),
    }
}
