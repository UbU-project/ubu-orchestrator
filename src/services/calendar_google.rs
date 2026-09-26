//! Thin OAuth/HTTP shell. Request and response policy lives in calendar_wire.
use super::calendar_range::CalendarTimeRange;
use std::{collections::BTreeSet, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::{Mutex, Notify, OnceCell};
use yup_oauth2::{
    authenticator::DefaultAuthenticator, InstalledFlowAuthenticator, InstalledFlowReturnMethod,
};

use super::{
    calendar_client::{CalendarApi, CalendarApiFuture},
    calendar_projection::DesiredEvent,
    calendar_wire::{
        self as wire, Operation, ResponseAction, WireRequest, CALENDAR_API_BASE, CALENDAR_SCOPE,
    },
};
use crate::{api::planning::DiagnosticBody, config::ServerConfig};

pub struct GoogleCalendarApi {
    client: reqwest::Client,
    calendar_id: String,
    api_base: String,
    credentials_path: PathBuf,
    token_cache_path: PathBuf,
    authenticator: OnceCell<DefaultAuthenticator>,
    consent_failed: Arc<Notify>,
    diagnostics: Mutex<Vec<DiagnosticBody>>,
}

impl GoogleCalendarApi {
    /// Construction performs no OAuth or credential I/O. The apply gate must
    /// grant a permit before the first CalendarApi method is called.
    pub fn new(config: &ServerConfig) -> Result<Self, String> {
        let credentials_path = config
            .google_credentials_path()
            .ok_or("initialize event `*`: UBU_GOOGLE_CREDENTIALS_PATH is required")?
            .to_owned();
        let token_cache_path = config
            .google_token_cache_path()
            .ok_or("initialize event `*`: UBU_GOOGLE_TOKEN_CACHE_PATH is required")?
            .to_owned();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "initialize event `*`: could not initialize Calendar HTTP client")?;
        Ok(Self {
            client,
            calendar_id: config.google_calendar_id().into(),
            api_base: CALENDAR_API_BASE.into(),
            credentials_path,
            token_cache_path,
            authenticator: OnceCell::new(),
            consent_failed: Arc::new(Notify::new()),
            diagnostics: Mutex::new(Vec::new()),
        })
    }

    pub async fn take_diagnostics(&self) -> Vec<DiagnosticBody> {
        std::mem::take(&mut *self.diagnostics.lock().await)
    }

    async fn access_token(&self, request: &WireRequest) -> Result<String, String> {
        let fail =
            |reason: &str| wire::operation_error(request.operation, &request.event_id, reason);
        let auth = self
            .authenticator
            .get_or_try_init(|| async {
                let secret = yup_oauth2::read_application_secret(&self.credentials_path)
                    .await
                    .map_err(|_| "UBU_GOOGLE_CREDENTIALS_PATH is unreadable or invalid")?;
                InstalledFlowAuthenticator::builder(secret, InstalledFlowReturnMethod::HTTPRedirect)
                    .flow_delegate(Box::new(BrowserDelegate {
                        failed: self.consent_failed.clone(),
                    }))
                    .persist_tokens_to_disk(&self.token_cache_path)
                    .build()
                    .await
                    .map_err(|_| "UBU_GOOGLE_TOKEN_CACHE_PATH is unreadable or invalid")
            })
            .await
            .map_err(fail)?;
        // yup-oauth2 9's HTTPRedirect flow ignores delegate errors. Race an
        // explicit failure notification against it, so headless/missing-browser
        // consent fails immediately instead of waiting forever for a callback.
        let token = tokio::select! {
            biased;
            _ = self.consent_failed.notified() => return Err(fail("OAuth consent requires a working desktop browser; no prompt was opened")),
            result = tokio::time::timeout(Duration::from_secs(300), auth.token(&[CALENDAR_SCOPE])) => {
                result.map_err(|_| fail("OAuth consent or token refresh timed out"))?
                    .map_err(|_| fail("OAuth failed; check UBU_GOOGLE_CREDENTIALS_PATH and writable UBU_GOOGLE_TOKEN_CACHE_PATH"))?
            }
        };
        token
            .token()
            .map(str::to_owned)
            .ok_or_else(|| fail("OAuth returned no access token"))
    }

    async fn send(&self, request: &WireRequest) -> Result<(u16, String), String> {
        let token = self.access_token(request).await?;
        let fail =
            |reason: &str| wire::operation_error(request.operation, &request.event_id, reason);
        let method = reqwest::Method::from_bytes(request.operation.method().as_bytes())
            .map_err(|_| fail("invalid HTTP method"))?;
        let mut builder = self.client.request(method, &request.url);
        for (name, value) in request.headers(&token) {
            builder = builder.header(name, value);
        }
        if let Some(body) = &request.body {
            builder = builder.json(body);
        }
        let response = builder
            .send()
            .await
            .map_err(|_| fail("Calendar HTTP request failed"))?;
        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|_| fail("Calendar HTTP response could not be read"))?;
        Ok((status, body))
    }

    async fn write(
        &self,
        mut request: WireRequest,
        event: Option<&DesiredEvent>,
    ) -> Result<(), String> {
        loop {
            let (status, body) = self.send(&request).await?;
            match wire::response_action(request.operation, &request.event_id, status, &body) {
                ResponseAction::Done => return Ok(()),
                ResponseAction::Failed(message) => return Err(message),
                ResponseAction::Patch => {
                    self.diagnostics.lock().await.push(DiagnosticBody {
                        code: "calendar_insert_converted_to_patch".into(),
                        message: wire::operation_error(
                            Operation::Insert,
                            &request.event_id,
                            "HTTP 409; converting insert to patch",
                        ),
                    });
                    let event = event.ok_or_else(|| {
                        wire::operation_error(
                            Operation::Insert,
                            &request.event_id,
                            "missing event for patch",
                        )
                    })?;
                    request = wire::event_request(
                        Operation::Patch,
                        &self.api_base,
                        &self.calendar_id,
                        event,
                    );
                }
            }
        }
    }
}

impl CalendarApi for GoogleCalendarApi {
    fn list_events<'a>(&'a self, range: &'a CalendarTimeRange) -> CalendarApiFuture<'a, Vec<DesiredEvent>> {
        Box::pin(async move {
            let mut events = Vec::new();
            let mut page = None;
            let mut seen = BTreeSet::new();
            loop {
                let request =
                    wire::list_request(&self.api_base, &self.calendar_id, range, page.as_deref());
                let (status, body) = self.send(&request).await?;
                match wire::response_action(Operation::List, "*", status, &body) {
                    ResponseAction::Done => {}
                    ResponseAction::Failed(message) => return Err(message),
                    ResponseAction::Patch => {
                        unreachable!("wire policy only patches conflicting inserts")
                    }
                }
                let value = serde_json::from_str(&body)
                    .map_err(|_| "list event `*`: invalid Google Calendar JSON")?;
                let (parsed, skipped) = wire::parse_event_list(&value);
                events.extend(parsed);
                self.diagnostics
                    .lock()
                    .await
                    .extend(skipped.into_iter().map(|message| DiagnosticBody {
                        code: "calendar_event_skipped".into(),
                        message,
                    }));
                page = wire::next_page(&value, &mut seen)?;
                if page.is_none() {
                    return Ok(events);
                }
            }
        })
    }

    fn insert_event<'a>(&'a self, event: &'a DesiredEvent) -> CalendarApiFuture<'a, ()> {
        Box::pin(self.write(
            wire::event_request(Operation::Insert, &self.api_base, &self.calendar_id, event),
            Some(event),
        ))
    }

    fn patch_event<'a>(&'a self, event: &'a DesiredEvent) -> CalendarApiFuture<'a, ()> {
        Box::pin(self.write(
            wire::event_request(Operation::Patch, &self.api_base, &self.calendar_id, event),
            Some(event),
        ))
    }

    fn delete_event<'a>(&'a self, external_id: &'a str) -> CalendarApiFuture<'a, ()> {
        Box::pin(self.write(
            wire::delete_request(&self.api_base, &self.calendar_id, external_id),
            None,
        ))
    }
}

struct BrowserDelegate {
    failed: Arc<Notify>,
}
impl yup_oauth2::authenticator_delegate::InstalledFlowDelegate for BrowserDelegate {
    fn present_user_url<'a>(
        &'a self,
        url: &'a str,
        need_code: bool,
    ) -> CalendarApiFuture<'a, String> {
        Box::pin(async move {
            let desktop = cfg!(target_os = "macos")
                || cfg!(target_os = "windows")
                || std::env::var_os("DISPLAY").is_some_and(|value| !value.is_empty())
                || std::env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty());
            if need_code || !desktop {
                self.failed.notify_one();
                return Err("interactive consent unavailable".into());
            }
            let url = url.to_owned();
            let opened = tokio::task::spawn_blocking(move || {
                use std::process::{Command, Stdio};
                #[cfg(target_os = "macos")]
                let mut command = Command::new("open");
                #[cfg(target_os = "windows")]
                let mut command = {
                    let mut c = Command::new("rundll32.exe");
                    c.arg("url.dll,FileProtocolHandler");
                    c
                };
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                let mut command = Command::new("xdg-open");
                command
                    .arg(url)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|status| status.success())
            })
            .await
            .unwrap_or(false);
            if opened {
                Ok(String::new())
            } else {
                self.failed.notify_one();
                Err("browser could not be opened".into())
            }
        })
    }
}
