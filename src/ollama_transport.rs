//! Executable-only HTTP shell; deliberately absent from the library and tests.
use std::time::Duration;
use ubu_core::worker::{AdvisoryTransport, LocalAdvisoryResult, LocalAdvisorySubmission};
use ubu_orchestrator::services::advisory_wire::{self as wire, Failure};

pub(crate) struct OllamaTransport {
    endpoint: String,
}
impl OllamaTransport {
    pub(crate) fn new(endpoint: &str) -> Self {
        Self {
            endpoint: endpoint.into(),
        }
    }
}
impl AdvisoryTransport for OllamaTransport {
    fn submit(&self, sub: &LocalAdvisorySubmission) -> ubu_core::Result<LocalAdvisoryResult> {
        if !ubu_orchestrator::services::setting_authoring::valid_advisory_endpoint(&self.endpoint) {
            return Ok(wire::failed(sub, Failure::Unavailable));
        }
        let body = match wire::request_body(sub) {
            Ok(body) => body,
            Err(reason) => return Ok(wire::failed(sub, reason)),
        };
        let endpoint = self.endpoint.clone();
        let submission = sub.clone();
        // The established trait is synchronous; a dedicated runtime thread keeps
        // async reqwest out of nested-runtime block_on and requires no new feature.
        let result = std::thread::Builder::new()
            .name("ollama-submit".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|_| Failure::Unavailable)?;
                runtime.block_on(async {
                    let send = async {
                        let client = reqwest::Client::builder()
                            .no_proxy()
                            .redirect(reqwest::redirect::Policy::none())
                            .timeout(Duration::from_millis(submission.timeout_ms))
                            .build()
                            .map_err(|_| Failure::Unavailable)?;
                        let mut response = client
                            .post(format!("{endpoint}/api/generate"))
                            .json(&body)
                            .send()
                            .await
                            .map_err(http_failure)?;
                        let status = response.status().as_u16();
                        if response
                            .content_length()
                            .is_some_and(|size| size > submission.result_size_limit_bytes)
                        {
                            return Err(Failure::TooLarge);
                        }
                        let mut bytes = Vec::new();
                        while let Some(chunk) = response.chunk().await.map_err(http_failure)? {
                            wire::append_chunk(
                                &mut bytes,
                                &chunk,
                                submission.result_size_limit_bytes,
                            )?;
                        }
                        Ok(wire::interpret(&submission, status, &bytes))
                    };
                    tokio::time::timeout(Duration::from_millis(submission.timeout_ms), send)
                        .await
                        .map_err(|_| Failure::Timeout)?
                })
            })
            .map(|thread| thread.join());
        Ok(match result {
            Ok(Ok(Ok(result))) => result,
            Ok(Ok(Err(reason))) => wire::failed(sub, reason),
            _ => wire::failed(sub, Failure::Unavailable),
        })
    }
}
fn http_failure(error: reqwest::Error) -> Failure {
    if error.is_timeout() {
        Failure::Timeout
    } else {
        Failure::Connection
    }
}
