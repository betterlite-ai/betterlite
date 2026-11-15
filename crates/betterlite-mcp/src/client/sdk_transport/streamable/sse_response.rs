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
enum Flow {
    Continue,
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
        let Some(flow) = core
            .cancel
            .run_until_cancelled(next_flow(core, &mut events, &mut resume, &id))
            .await
        else {
            return Ok(());
        };
        match flow? {
            Flow::Continue => {}
            Flow::Complete => return Ok(()),
            Flow::Ended | Flow::Retry => {
                if resume.event_id.is_none() {
                    let message = "SSE stream ended without a response".to_owned();
                    return push_error(core, CONNECTION_CLOSED, message, id.clone()).await;
                }
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
async fn next_flow(
    core: &StreamableCore,
    events: &mut EventStream,
    resume: &mut Resume,
    id: &RequestId,
) -> Result<Flow, SdkError> {
    let Some(event) = events.next().await else {
        return Ok(Flow::Ended);
    };
    let sse = match event {
        Ok(sse) => sse,
        Err(error)
            if matches!(
                error,
                SdkError::Sse(_) | SdkError::EventTooLarge { .. } | SdkError::ContentType { .. }
            ) =>
        {
            push_error(core, CONNECTION_CLOSED, format!("{error}"), id.clone()).await?;
            return Ok(Flow::Complete);
        }
        Err(_) => return Ok(Flow::Retry),
    };
    track_event(resume, &sse);
    if handle_event(core, sse, id).await? {
        return Ok(Flow::Complete);
    }
    Ok(Flow::Continue)
}
async fn handle_event(core: &StreamableCore, sse: Sse, id: &RequestId) -> Result<bool, SdkError> {
    let Some(parsed) = event_message(&sse) else {
        return Ok(false);
    };
    let mut message = match parsed {
        Ok(message) => message,
        Err(source) => {
            let message = format!("Failed to parse SSE message: {source}");
            push_error(core, ErrorCode::PARSE_ERROR, message, id.clone()).await?;
            return Ok(true);
        }
    };
    let complete = rewrite_id(&mut message, id);
    push(core, message).await?;
    Ok(complete)
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
    let mut attempt = 0_usize;
    loop {
        if attempt >= MAX_RECONNECTION_ATTEMPTS {
            let message = "SSE stream ended and reconnection attempts were exhausted".to_owned();
            return push_error(core, CONNECTION_CLOSED, message, id.clone()).await;
        }
        sleep(retry_delay(resume.retry_ms)).await;
        let Some(mut events) = reconnection_stream(core, resume).await else {
            attempt = attempt.saturating_add(1);
            continue;
        };
        loop {
            let flow = next_flow(core, &mut events, resume, id).await?;
            match flow {
                Flow::Continue => {}
                Flow::Complete => return Ok(()),
                Flow::Ended => {
                    attempt = 0;
                    break;
                }
                Flow::Retry => {
                    attempt = attempt.saturating_add(1);
                    break;
                }
            }
        }
    }
}
async fn reconnection_stream(core: &StreamableCore, resume: &Resume) -> Option<EventStream> {
    let mut response = match send_within_origin(&core.http, get_request(core, resume)).await {
        Ok(response) => response,
        Err(error) => {
            debug!("Reconnection failed: {error}");
            return None;
        }
    };
    if !(200..=299).contains(&response.status) {
        drop(response.body.close().await);
        return None;
    }
    if !is_sse_response(&response.headers) {
        drop(response.body.close().await);
        return None;
    }
    let HttpResponse { headers, body, .. } = response;
    Some(open_stream(body, core.max_event_size, &headers))
}
