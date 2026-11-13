use std::sync::Arc;

use bytes::Bytes;
use http::Method;
use http::header::HeaderValue;
use sse_stream::Sse;
use tokio::time::sleep;
use tracing::warn;

use crate::client::cleanup::http_client::HttpResponse;
use crate::client::cleanup::recording::ResponseBody as _;
use crate::client::http_transport::request::McpHttpRequest;
use crate::client::sdk_transport::error::SdkError;
use crate::client::sdk_transport::event_stream::EventStream;
use crate::client::sdk_transport::origin::{redirect_note, send_within_origin, status_error};
use crate::client::sessions::stream::StreamFailureKind;

use super::sse_response::{event_message, track_event};
use super::streamable::{LAST_EVENT_ID, MAX_RECONNECTION_ATTEMPTS, StreamableCore};
enum GetOutcome {
    Ended,
    Framing(SdkError),
    Invalid(SdkError),
    Closed,
}
impl StreamableCore {
    pub(super) async fn run_get_stream(self: Arc<Self>) {
        let mut event_id: Option<String> = None;
        let mut retry_ms: Option<u64> = None;
        let mut attempt = 0_usize;
        loop {
            match self.get_once(&mut event_id, &mut retry_ms).await {
                GetOutcome::Closed => return,
                GetOutcome::Invalid(error) => {
                    self.failure(StreamFailureKind::ValueError, error);
                    return;
                }
                GetOutcome::Framing(error) => {
                    attempt = attempt.saturating_add(1);
                    if attempt >= MAX_RECONNECTION_ATTEMPTS {
                        self.failure(StreamFailureKind::Http, error);
                        return;
                    }
                }
                GetOutcome::Ended => attempt = 0,
            }
            tokio::select! {
                () = self.cancel.cancelled() => return,
                () = sleep(StreamableCore::retry_delay(retry_ms)) => {}
            }
        }
    }
    async fn get_once(
        &self,
        event_id: &mut Option<String>,
        retry_ms: &mut Option<u64>,
    ) -> GetOutcome {
        if self.session_id().is_none() {
            return GetOutcome::Closed;
        }
        let mut headers = self.headers();
        if let Some(current) = event_id.as_deref()
            && let Ok(value) = HeaderValue::from_str(current)
        {
            headers.insert(LAST_EVENT_ID, value);
        }
        let request = McpHttpRequest {
            method: Method::GET,
            url: self.url.clone(),
            headers,
            body: Bytes::new(),
            read_timeout: None,
        };
        let mut response = match send_within_origin(&self.http, request).await {
            Ok(response) => response,
            Err(error) => return GetOutcome::Framing(error),
        };
        if let Some(note) = redirect_note(&self.url, response.status, &response.headers) {
            warn!("GET stream not opened: {note}");
            drop(response.body.close().await);
            return GetOutcome::Closed;
        }
        if !(200..=299).contains(&response.status) {
            return GetOutcome::Framing(status_error(response));
        }
        let HttpResponse { body, .. } = response;
        let mut events = EventStream::open(body, self.max_event_size);
        loop {
            let event = tokio::select! {
                () = self.cancel.cancelled() => return GetOutcome::Closed,
                event = events.next() => event,
            };
            let Some(event) = event else {
                return GetOutcome::Ended;
            };
            match event {
                Err(error) => return GetOutcome::Framing(error),
                Ok(sse) => {
                    track_event(event_id, retry_ms, &sse);
                    match self.on_event(sse).await {
                        Ok(()) => {}
                        Err(SdkError::Json(source)) => {
                            return GetOutcome::Invalid(SdkError::Json(source));
                        }
                        Err(other) => return GetOutcome::Framing(other),
                    }
                }
            }
        }
    }
    async fn on_event(&self, sse: Sse) -> Result<(), SdkError> {
        let Some(parsed) = event_message(&sse) else {
            return Ok(());
        };
        self.push(parsed?).await
    }
}
