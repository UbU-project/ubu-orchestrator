//! Offline Calendar client boundary. No provider types or transport live here.
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
    sync::Mutex,
};

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::calendar_projection::DesiredEvent;

pub type CalendarApiFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;

pub trait CalendarApi: Send + Sync {
    fn list_events(&self) -> CalendarApiFuture<'_, Vec<DesiredEvent>>;
    fn insert_event<'a>(&'a self, event: &'a DesiredEvent) -> CalendarApiFuture<'a, ()>;
    fn patch_event<'a>(&'a self, event: &'a DesiredEvent) -> CalendarApiFuture<'a, ()>;
    fn delete_event<'a>(&'a self, external_id: &'a str) -> CalendarApiFuture<'a, ()>;
}

/// Live requires operator configuration and explicit in-memory enablement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CalendarExportMode {
    Mock,
    Live,
}

impl CalendarExportMode {
    pub fn ensure_available(self, state: &crate::state::AppState) -> crate::errors::Result<()> {
        if self == Self::Live {
            require_live_configuration(state)?;
            if state.inner().google_calendar_enabled.load(std::sync::atomic::Ordering::Acquire) {
                return Ok(());
            }
            return Err(crate::errors::AppError::Diagnostic {
                status: axum::http::StatusCode::FORBIDDEN,
                code: "calendar_live_export_not_enabled".into(),
                message: "Live Calendar export is not enabled for this process; POST /desktop/session/google-calendar first".into(),
            });
        }
        Ok(())
    }
}

pub fn require_live_configuration(state: &crate::state::AppState) -> crate::errors::Result<()> {
    let config = &state.inner().config;
    if [config.google_credentials_path(), config.google_token_cache_path()]
        .iter().all(|path| path.is_some_and(|path| !path.as_os_str().is_empty())) {
        return Ok(());
    }
    Err(crate::errors::AppError::Diagnostic {
        status: axum::http::StatusCode::SERVICE_UNAVAILABLE,
        code: "calendar_live_export_unconfigured".into(),
        message: "Live Calendar export requires UBU_GOOGLE_CREDENTIALS_PATH and UBU_GOOGLE_TOKEN_CACHE_PATH".into(),
    })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecordedCalendarCall {
    ListEvents,
    InsertEvent { event: DesiredEvent },
    PatchEvent { event: DesiredEvent },
    DeleteEvent { external_id: String },
}

#[derive(Debug, Default)]
struct RecordingState {
    events: BTreeMap<String, DesiredEvent>,
    calls: Vec<RecordedCalendarCall>,
    fail_ids: BTreeSet<String>,
}

#[derive(Debug, Default)]
pub struct RecordingCalendarApi {
    state: Mutex<RecordingState>,
}

impl RecordingCalendarApi {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_events(events: impl IntoIterator<Item = DesiredEvent>) -> Self {
        Self {
            state: Mutex::new(RecordingState {
                events: events
                    .into_iter()
                    .map(|event| (event.external_id.clone(), event))
                    .collect(),
                ..RecordingState::default()
            }),
        }
    }

    /// Deterministic failure injection. A failed call is recorded but changes no event.
    pub fn fail_for(&self, external_id: impl Into<String>) {
        self.state
            .lock()
            .unwrap()
            .fail_ids
            .insert(external_id.into());
    }

    pub fn recorded_calls(&self) -> Vec<RecordedCalendarCall> {
        self.state.lock().unwrap().calls.clone()
    }

    pub fn events(&self) -> Vec<DesiredEvent> {
        self.state
            .lock()
            .unwrap()
            .events
            .values()
            .cloned()
            .collect()
    }

    pub fn clear_recorded_calls(&self) {
        self.state.lock().unwrap().calls.clear();
    }

    fn failure(state: &RecordingState, id: &str) -> Result<(), String> {
        if state.fail_ids.contains(id) {
            Err(format!("recording Calendar client rejected event `{id}`"))
        } else {
            Ok(())
        }
    }
}

impl CalendarApi for RecordingCalendarApi {
    fn list_events(&self) -> CalendarApiFuture<'_, Vec<DesiredEvent>> {
        Box::pin(async move {
            let mut state = self.state.lock().unwrap();
            state.calls.push(RecordedCalendarCall::ListEvents);
            Ok(state.events.values().cloned().collect())
        })
    }
    fn insert_event<'a>(&'a self, event: &'a DesiredEvent) -> CalendarApiFuture<'a, ()> {
        Box::pin(async move {
            let mut state = self.state.lock().unwrap();
            state.calls.push(RecordedCalendarCall::InsertEvent {
                event: event.clone(),
            });
            Self::failure(&state, &event.external_id)?;
            if state.events.contains_key(&event.external_id) {
                return Err(format!("event `{}` already exists", event.external_id));
            }
            state
                .events
                .insert(event.external_id.clone(), event.clone());
            Ok(())
        })
    }
    fn patch_event<'a>(&'a self, event: &'a DesiredEvent) -> CalendarApiFuture<'a, ()> {
        Box::pin(async move {
            let mut state = self.state.lock().unwrap();
            state.calls.push(RecordedCalendarCall::PatchEvent {
                event: event.clone(),
            });
            Self::failure(&state, &event.external_id)?;
            let existing = state
                .events
                .get_mut(&event.external_id)
                .ok_or_else(|| format!("event `{}` does not exist", event.external_id))?;
            *existing = event.clone();
            Ok(())
        })
    }
    fn delete_event<'a>(&'a self, external_id: &'a str) -> CalendarApiFuture<'a, ()> {
        Box::pin(async move {
            let mut state = self.state.lock().unwrap();
            state.calls.push(RecordedCalendarCall::DeleteEvent {
                external_id: external_id.into(),
            });
            Self::failure(&state, external_id)?;
            state
                .events
                .remove(external_id)
                .ok_or_else(|| format!("event `{external_id}` does not exist"))?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(id: &str) -> DesiredEvent {
        DesiredEvent {
            external_id: id.into(),
            task_id: format!("task_{id}"),
            summary: "Synthetic event".into(),
            start_at: "2026-09-25T09:00:00Z".into(),
            end_at: "2026-09-25T09:30:00Z".into(),
            color_id: None,
            transparent: false,
            reminders_minutes: vec![],
        }
    }
    #[tokio::test]
    async fn seeded_recording_client_tracks_crud_in_call_order() {
        let seed = event("aaaaa");
        let added = event("bbbbb");
        let api = RecordingCalendarApi::with_events([seed.clone()]);
        assert_eq!(api.list_events().await.unwrap(), vec![seed.clone()]);
        api.insert_event(&added).await.unwrap();
        let mut changed = added.clone();
        changed.summary = "Updated".into();
        api.patch_event(&changed).await.unwrap();
        api.delete_event(&seed.external_id).await.unwrap();
        assert_eq!(api.events(), vec![changed.clone()]);
        assert_eq!(
            api.recorded_calls(),
            vec![
                RecordedCalendarCall::ListEvents,
                RecordedCalendarCall::InsertEvent { event: added },
                RecordedCalendarCall::PatchEvent { event: changed },
                RecordedCalendarCall::DeleteEvent {
                    external_id: seed.external_id
                }
            ]
        );
    }
    #[tokio::test]
    async fn failed_recorded_operations_are_atomic_and_not_retried() {
        let seed = event("aaaaa");
        let added = event("bbbbb");
        let api = RecordingCalendarApi::with_events([seed.clone()]);
        api.fail_for(&seed.external_id);
        api.fail_for(&added.external_id);
        assert!(api.insert_event(&added).await.is_err());
        let mut changed = seed.clone();
        changed.summary = "Must not land".into();
        assert!(api.patch_event(&changed).await.is_err());
        assert!(api.delete_event(&seed.external_id).await.is_err());
        assert_eq!(api.events(), vec![seed]);
        assert_eq!(api.recorded_calls().len(), 3);
    }
}
