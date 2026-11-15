use futures_util::StreamExt as _;
use http::HeaderMap;
use rmcp::RoleClient;
use rmcp::model::{ErrorCode, JsonRpcMessage, RequestId};
use rmcp::service::RxJsonRpcMessage;
use sse_stream::Sse;
use tokio::time::sleep;
use tracing::{debug, warn};

use crate::client::cleanup::http_client::HttpResponse;
use crate::client::cleanup::recording::ResponseBody as _;
use crate::client::http_transport::send::HttpBody;
use crate::client::sdk_transport::error::SdkError;
use crate::client::sdk_transport::event_stream::{EventStream, is_sse_response, open_stream};
use crate::client::sdk_transport::origin::send_within_origin;

use super::get_stream::get_request;
use super::post::{push, push_error};
use super::{MAX_RECONNECTION_ATTEMPTS, Resume, StreamableCore, retry_delay};
const CONNECTION_CLOSED: ErrorCode = ErrorCode(-32000);
enum Step {
    Continue,
    Complete,
    Reconnect,
}
enum Attempt {
    Complete,
    Ended,
    Retry,
}
pub(super) fn event_message(
    sse: &Sse,
) -> Option<Result<RxJsonRpcMessage<RoleClient>, serde_json::Error>> {
    match sse.event.as_deref() {
        None | Some("" | "message") => {}
        Some(other) => {
            warn!("Unknown SSE event: {other}");
            return None;
        }
    }
    let data = sse.data.as_deref().unwrap_or_default();
    if data.is_empty() {
        return None;
    }
    Some(serde_json::from_str(data))
}
pub(super) fn track_event(resume: &mut Resume, sse: &Sse) {
    if let Some(id) = sse.id.as_deref()
        && !id.is_empty()
    {
        resume.event_id = Some(id.to_owned());
    }
    if let Some(retry) = sse.retry {
        resume.retry_ms = Some(retry);
    }
}
// Nothing here is a stream that belongs to no request, so a failure answers the request instead
pub(super) async fn read_events(
    core: &StreamableCore,
    body: HttpBody,
    headers: &HeaderMap,
    id: RequestId,
) -> Result<(), SdkError> {
    let mut events = open_stream(body, core.max_event_size, headers);
    let mut resume = Resume::default();
    loop {
        let Some(step) = core
            .cancel
            .run_until_cancelled(next_event(core, &mut events, &mut resume, &id))
            .await
        else {
            return Ok(());
        };
        match step? {
            Step::Continue => {}
            Step::Complete => return Ok(()),
            Step::Reconnect => {
                let Some(resumed) = core
                    .cancel
                    .run_until_cancelled(reconnect(core, &mut resume, &id))
                    .await
                else {
                    return Ok(());
                };
                return resumed;
            }
        }
    }
}
async fn next_event(
    core: &StreamableCore,
    events: &mut EventStream,
    resume: &mut Resume,
    id: &RequestId,
) -> Result<Step, SdkError> {
    match events.next().await {
        None => Ok(Step::Reconnect),
        Some(Err(error)) if framing(&error) => {
            push_error(core, CONNECTION_CLOSED, format!("{error}"), id.clone()).await?;
            Ok(Step::Complete)
        }
        Some(Err(_)) => Ok(Step::Reconnect),
        Some(Ok(sse)) => {
            track_event(resume, &sse);
            handle_event(core, sse, id).await
        }
    }
}
const fn framing(error: &SdkError) -> bool {
    matches!(
        error,
        SdkError::Sse(_) | SdkError::EventTooLarge { .. } | SdkError::ContentType { .. }
    )
}
async fn handle_event(core: &StreamableCore, sse: Sse, id: &RequestId) -> Result<Step, SdkError> {
    let Some(parsed) = event_message(&sse) else {
        return Ok(Step::Continue);
    };
    let mut message = match parsed {
        Ok(message) => message,
        Err(source) => {
            let message = format!("Failed to parse SSE message: {source}");
            push_error(core, ErrorCode::PARSE_ERROR, message, id.clone()).await?;
            return Ok(Step::Complete);
        }
    };
    let complete = rewrite_id(&mut message, id);
    push(core, message).await?;
    if complete {
        return Ok(Step::Complete);
    }
    Ok(Step::Continue)
}
fn rewrite_id(message: &mut RxJsonRpcMessage<RoleClient>, id: &RequestId) -> bool {
    match message {
        JsonRpcMessage::Response(response) => {
            response.id = id.clone();
            true
        }
        JsonRpcMessage::Error(error) => {
            error.id = Some(id.clone());
            true
        }
        JsonRpcMessage::Request(_) | JsonRpcMessage::Notification(_) => false,
    }
}
async fn reconnect(
    core: &StreamableCore,
    resume: &mut Resume,
    id: &RequestId,
) -> Result<(), SdkError> {
    if resume.event_id.is_none() {
        let message = "SSE stream ended without a response".to_owned();
        return push_error(core, CONNECTION_CLOSED, message, id.clone()).await;
    }
    let mut attempt = 0_usize;
    loop {
        if attempt >= MAX_RECONNECTION_ATTEMPTS {
            let message = "SSE stream ended and reconnection attempts were exhausted".to_owned();
            return push_error(core, CONNECTION_CLOSED, message, id.clone()).await;
        }
        sleep(retry_delay(resume.retry_ms)).await;
        match reconnect_once(core, resume, id).await? {
            Attempt::Complete => return Ok(()),
            Attempt::Ended => attempt = 0,
            Attempt::Retry => attempt = attempt.saturating_add(1),
        }
    }
}
async fn reconnect_once(
    core: &StreamableCore,
    resume: &mut Resume,
    id: &RequestId,
) -> Result<Attempt, SdkError> {
    let mut response = match send_within_origin(&core.http, get_request(core, resume)).await {
        Ok(response) => response,
        Err(error) => {
            debug!("Reconnection failed: {error}");
            return Ok(Attempt::Retry);
        }
    };
    if !(200..=299).contains(&response.status) {
        drop(response.body.close().await);
        return Ok(Attempt::Retry);
    }
    let is_sse = is_sse_response(&response.headers);
    let HttpResponse { headers, body, .. } = response;
    let mut events = open_stream(body, core.max_event_size, &headers);
    loop {
        let event = match events.next().await {
            None => return Ok(Attempt::Ended),
            Some(Err(error)) if framing(&error) && is_sse => {
                push_error(core, CONNECTION_CLOSED, format!("{error}"), id.clone()).await?;
                return Ok(Attempt::Complete);
            }
            Some(Err(_)) => return Ok(Attempt::Retry),
            Some(Ok(event)) => event,
        };
        track_event(resume, &event);
        match handle_event(core, event, id).await? {
            Step::Complete => return Ok(Attempt::Complete),
            Step::Continue | Step::Reconnect => {}
        }
    }
}
