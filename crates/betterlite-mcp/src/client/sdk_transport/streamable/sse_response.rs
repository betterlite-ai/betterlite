use bytes::Bytes;
use http::Method;
use http::header::HeaderValue;
use rmcp::RoleClient;
use rmcp::model::{ErrorCode, JsonRpcMessage, RequestId};
use rmcp::service::RxJsonRpcMessage;
use sse_stream::Sse;
use tokio::time::sleep;
use tracing::{debug, warn};

use crate::client::cleanup::http_client::HttpResponse;
use crate::client::cleanup::recording::ResponseBody as _;
use crate::client::http_transport::request::McpHttpRequest;
use crate::client::http_transport::send::HttpBody;
use crate::client::sdk_transport::error::SdkError;
use crate::client::sdk_transport::event_stream::EventStream;
use crate::client::sdk_transport::origin::send_within_origin;

use super::post::content_type;
use super::{CONNECTION_CLOSED, LAST_EVENT_ID, MAX_RECONNECTION_ATTEMPTS, StreamableCore};
enum Attempt {
    Complete,
    Ended,
    Retry,
}
pub(super) fn event_message(sse: &Sse) -> Option<Result<RxJsonRpcMessage<RoleClient>, SdkError>> {
    if !matches!(sse.event.as_deref(), None | Some("message")) {
        warn!("Unknown SSE event: {}", sse.event.as_deref().unwrap_or(""));
        return None;
    }
    let data = sse.data.as_deref().unwrap_or_default();
    if data.is_empty() {
        return None;
    }
    Some(serde_json::from_str::<RxJsonRpcMessage<RoleClient>>(data).map_err(SdkError::Json))
}
pub(super) fn track_event(event_id: &mut Option<String>, retry_ms: &mut Option<u64>, sse: &Sse) {
    if let Some(id) = sse.id.as_deref() {
        *event_id = Some(id.to_owned());
    }
    if let Some(retry) = sse.retry {
        *retry_ms = Some(retry);
    }
}
impl StreamableCore {
    pub(super) async fn read_events(&self, body: HttpBody, id: RequestId) -> Result<(), SdkError> {
        let mut events = EventStream::open(body, self.max_event_size);
        let mut event_id: Option<String> = None;
        let mut retry_ms: Option<u64> = None;
        loop {
            let Some(event) = events.next().await else {
                return self.reconnect(event_id, retry_ms, id).await;
            };
            match event {
                Err(error) => {
                    if !matches!(&error, SdkError::Sse(_) | SdkError::EventTooLarge { .. }) {
                        return self.reconnect(event_id, retry_ms, id).await;
                    }
                    return self
                        .push_error(CONNECTION_CLOSED, format!("{error}"), id)
                        .await;
                }
                Ok(sse) => {
                    track_event(&mut event_id, &mut retry_ms, &sse);
                    match self.handle_event(sse, &id).await {
                        Ok(true) => return Ok(()),
                        Ok(false) => {}
                        Err(SdkError::Json(source)) => {
                            let message = format!("Failed to parse SSE message: {source}");
                            return self.push_error(ErrorCode::PARSE_ERROR, message, id).await;
                        }
                        Err(other) => return Err(other),
                    }
                }
            }
        }
    }
    async fn reconnect(
        &self,
        last_event_id: Option<String>,
        retry: Option<u64>,
        id: RequestId,
    ) -> Result<(), SdkError> {
        let mut retry_ms = retry;
        let mut attempt = 0_usize;
        let mut event_id = last_event_id;
        if event_id.is_none() {
            return self
                .push_error(
                    CONNECTION_CLOSED,
                    "SSE stream ended without a response".to_owned(),
                    id,
                )
                .await;
        }
        loop {
            if attempt >= MAX_RECONNECTION_ATTEMPTS {
                return self
                    .push_error(
                        CONNECTION_CLOSED,
                        "SSE stream ended and reconnection attempts were exhausted".to_owned(),
                        id,
                    )
                    .await;
            }
            sleep(StreamableCore::retry_delay(retry_ms)).await;
            match self
                .reconnect_once(&mut event_id, &mut retry_ms, &id)
                .await?
            {
                Attempt::Complete => return Ok(()),
                Attempt::Ended => attempt = 0,
                Attempt::Retry => attempt = attempt.saturating_add(1),
            }
        }
    }
    async fn reconnect_once(
        &self,
        event_id: &mut Option<String>,
        retry_ms: &mut Option<u64>,
        id: &RequestId,
    ) -> Result<Attempt, SdkError> {
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
        let response = match send_within_origin(&self.http, request).await {
            Ok(response) => response,
            Err(error) => {
                debug!("Reconnection failed: {error}");
                return Ok(Attempt::Retry);
            }
        };
        let HttpResponse {
            status,
            headers,
            mut body,
            ..
        } = response;
        if !(200..=299).contains(&status) {
            drop(body.close().await);
            return Ok(Attempt::Retry);
        }
        let is_sse = content_type(&headers).starts_with("text/event-stream");
        let mut events = EventStream::open(body, self.max_event_size);
        loop {
            let Some(event) = events.next().await else {
                return Ok(Attempt::Ended);
            };
            match event {
                Err(error) => {
                    let framing =
                        matches!(&error, SdkError::Sse(_) | SdkError::EventTooLarge { .. });
                    if !framing || !is_sse {
                        return Ok(Attempt::Retry);
                    }
                    self.push_error(CONNECTION_CLOSED, format!("{error}"), id.clone())
                        .await?;
                    return Ok(Attempt::Complete);
                }
                Ok(sse) => {
                    track_event(event_id, retry_ms, &sse);
                    match self.handle_event(sse, id).await {
                        Ok(true) => return Ok(Attempt::Complete),
                        Ok(false) => {}
                        Err(SdkError::Json(source)) => {
                            let message = format!("Failed to parse SSE message: {source}");
                            self.push_error(ErrorCode::PARSE_ERROR, message, id.clone())
                                .await?;
                            return Ok(Attempt::Complete);
                        }
                        Err(other) => return Err(other),
                    }
                }
            }
        }
    }
    async fn handle_event(&self, sse: Sse, id: &RequestId) -> Result<bool, SdkError> {
        let Some(parsed) = event_message(&sse) else {
            return Ok(false);
        };
        let mut message = parsed?;
        let complete = match &mut message {
            JsonRpcMessage::Response(response) => {
                response.id = id.clone();
                true
            }
            JsonRpcMessage::Error(error) => {
                error.id = Some(id.clone());
                true
            }
            JsonRpcMessage::Request(_) | JsonRpcMessage::Notification(_) => false,
        };
        self.push(message).await?;
        Ok(complete)
    }
}
