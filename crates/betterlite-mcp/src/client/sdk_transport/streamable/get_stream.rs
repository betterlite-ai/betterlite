use std::sync::Arc;

use bytes::Bytes;
use futures_util::StreamExt as _;
use http::header::{CACHE_CONTROL, HeaderValue};
use http::{HeaderMap, Method};
use sse_stream::Sse;
use tokio::time::sleep;
use tracing::warn;

use crate::client::cleanup::http_client::HttpResponse;
use crate::client::cleanup::recording::ResponseBody as _;
use crate::client::http_transport::request::McpHttpRequest;
use crate::client::http_transport::send::HttpBody;
use crate::client::sdk_transport::error::SdkError;
use crate::client::sdk_transport::event_stream::open_stream;
use crate::client::sdk_transport::origin::{redirect_note, send_within_origin, status_error};
use crate::client::sessions::stream::StreamFailureKind;

use super::post::push;
use super::sse_response::{event_message, track_event};
use super::{
    LAST_EVENT_ID, MAX_RECONNECTION_ATTEMPTS, Resume, StreamableCore, report_stream_failure,
    request_headers, retry_delay, session_id,
};
enum Outcome {
    Ended,
    Framing(SdkError),
    Invalid(SdkError),
    Closed,
}
pub(super) async fn run_get_stream(core: Arc<StreamableCore>) {
    let mut resume = Resume::default();
    let mut attempt = 0_usize;
    loop {
        let Some(outcome) = core
            .cancel
            .run_until_cancelled(get_once(&core, &mut resume))
            .await
        else {
            return;
        };
        match outcome {
            Outcome::Closed => return,
            Outcome::Ended => attempt = 0,
            Outcome::Invalid(error) => {
                report_stream_failure(&core, StreamFailureKind::ValueError, error);
                return;
            }
            Outcome::Framing(error) => {
                attempt = attempt.saturating_add(1);
                if attempt >= MAX_RECONNECTION_ATTEMPTS {
                    report_stream_failure(&core, StreamFailureKind::Http, error);
                    return;
                }
            }
        }
        let waited = core
            .cancel
            .run_until_cancelled(sleep(retry_delay(resume.retry_ms)))
            .await;
        if waited.is_none() {
            return;
        }
    }
}
async fn get_once(core: &StreamableCore, resume: &mut Resume) -> Outcome {
    if session_id(core).is_none() {
        return Outcome::Closed;
    }
    let request = get_request(core, resume);
    let mut response = match send_within_origin(&core.http, request).await {
        Ok(response) => response,
        Err(error) => return Outcome::Framing(error),
    };
    if let Some(note) = redirect_note(&core.url, response.status, &response.headers) {
        warn!("GET stream not opened: {note}");
        drop(response.body.close().await);
        return Outcome::Closed;
    }
    if !(200..=299).contains(&response.status) {
        return Outcome::Framing(status_error(response));
    }
    let HttpResponse { headers, body, .. } = response;
    read_get_events(core, body, &headers, resume).await
}
pub(super) fn get_request(core: &StreamableCore, resume: &Resume) -> McpHttpRequest {
    let mut headers = request_headers(core);
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Some(current) = resume.event_id.as_deref()
        && let Ok(value) = HeaderValue::from_str(current)
    {
        headers.insert(LAST_EVENT_ID, value);
    }
    McpHttpRequest {
        method: Method::GET,
        url: core.url.clone(),
        headers,
        body: Bytes::new(),
        read_timeout: None,
    }
}
async fn read_get_events(
    core: &StreamableCore,
    body: HttpBody,
    headers: &HeaderMap,
    resume: &mut Resume,
) -> Outcome {
    let mut events = open_stream(body, core.max_event_size, headers);
    loop {
        let Some(event) = core.cancel.run_until_cancelled(events.next()).await else {
            return Outcome::Closed;
        };
        match event {
            None => return Outcome::Ended,
            Some(Err(error)) => return Outcome::Framing(error),
            Some(Ok(sse)) => {
                if let Some(outcome) = handle_event(core, resume, sse).await {
                    return outcome;
                }
            }
        }
    }
}
async fn handle_event(core: &StreamableCore, resume: &mut Resume, sse: Sse) -> Option<Outcome> {
    track_event(resume, &sse);
    let Some(parsed) = event_message(&sse) else {
        return None;
    };
    let message = match parsed {
        Ok(message) => message,
        Err(error) => return Some(Outcome::Invalid(SdkError::Json(error))),
    };
    match push(core, message).await {
        Ok(()) => None,
        Err(error) => Some(Outcome::Framing(error)),
    }
}
