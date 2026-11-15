use std::sync::{Arc, PoisonError};

use bytes::Bytes;
use http::{HeaderMap, Method};
use rmcp::RoleClient;
use rmcp::model::{ClientNotification, ErrorCode, ErrorData, JsonRpcMessage, RequestId};
use rmcp::service::{RxJsonRpcMessage, TxJsonRpcMessage};
use tracing::warn;

use crate::client::cleanup::http_client::HttpResponse;
use crate::client::cleanup::recording::ResponseBody as _;
use crate::client::http_transport::request::McpHttpRequest;
use crate::client::http_transport::send::HttpBody;
use crate::client::sdk_transport::error::SdkError;
use crate::client::sdk_transport::event_stream::content_type;
use crate::client::sdk_transport::origin::{redirect_note, send_within_origin};

use super::get_stream::run_get_stream;
use super::sse_response::read_events;
use super::{Incoming, StreamableCore, note_session, request_headers, session_id};
pub(super) async fn send(
    core: Arc<StreamableCore>,
    item: TxJsonRpcMessage<RoleClient>,
) -> Result<(), SdkError> {
    let initialize = matches!(
        &item,
        JsonRpcMessage::Request(request) if request.request.method() == "initialize"
    );
    let initialized = matches!(
        &item,
        JsonRpcMessage::Notification(notification)
            if matches!(&notification.notification, ClientNotification::InitializedNotification(_))
    );
    if initialize {
        let mut state = core.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.protocol_version = None;
    }
    let body = serde_json::to_vec(&item).map_err(SdkError::Json)?;
    let request = McpHttpRequest {
        method: Method::POST,
        url: core.url.clone(),
        headers: request_headers(&core),
        body: Bytes::from(body),
        read_timeout: None,
    };
    let mut response = send_within_origin(&core.http, request).await?;
    match request_id(&item) {
        Some(id) => answer(&core, response, id, initialize).await?,
        None => drop(response.body.close().await),
    }
    if initialized {
        core.tasks.spawn(run_get_stream(Arc::clone(&core)));
    }
    Ok(())
}
fn request_id(item: &TxJsonRpcMessage<RoleClient>) -> Option<RequestId> {
    match item {
        JsonRpcMessage::Request(request) => Some(request.id.clone()),
        JsonRpcMessage::Response(_)
        | JsonRpcMessage::Notification(_)
        | JsonRpcMessage::Error(_) => None,
    }
}
async fn answer(
    core: &StreamableCore,
    response: HttpResponse<HttpBody>,
    id: RequestId,
    initialize: bool,
) -> Result<(), SdkError> {
    let HttpResponse {
        status,
        headers,
        mut body,
        ..
    } = response;
    if status == 202 {
        let message = "server answered a request with 202 Accepted".to_owned();
        push_error(core, ErrorCode::INVALID_REQUEST, message, id).await?;
        drop(body.close().await);
        return Ok(());
    }
    if let Some(note) = redirect_note(&core.url, status, &headers) {
        warn!("{note}");
        push_error(
            core,
            ErrorCode::INVALID_REQUEST,
            note.trim_start().to_owned(),
            id,
        )
        .await?;
        drop(body.close().await);
        return Ok(());
    }
    if status >= 400 {
        return answer_error(core, status, &headers, body, id).await;
    }
    if initialize {
        note_session(core, &headers);
    }
    answer_body(core, headers, body, id).await
}
async fn answer_body(
    core: &StreamableCore,
    headers: HeaderMap,
    mut body: HttpBody,
    id: RequestId,
) -> Result<(), SdkError> {
    let kind = content_type(&headers);
    if kind.starts_with("application/json") {
        return push_json(core, body, id).await;
    }
    if kind.starts_with("text/event-stream") {
        return read_events(core, body, &headers, id).await;
    }
    let message = format!("Unexpected content type: {kind}");
    push_error(core, ErrorCode::INVALID_REQUEST, message, id).await?;
    drop(body.close().await);
    Ok(())
}
// httpx2 reports a body that fails to arrive and a body that is not JSON-RPC the same way
async fn push_json(
    core: &StreamableCore,
    mut body: HttpBody,
    id: RequestId,
) -> Result<(), SdkError> {
    let content = match body.read_all().await {
        Ok(content) => content,
        Err(source) => {
            let message = format!("Failed to parse JSON response: {source}");
            return push_error(core, ErrorCode::PARSE_ERROR, message, id).await;
        }
    };
    match serde_json::from_slice::<RxJsonRpcMessage<RoleClient>>(&content) {
        Ok(message) => push(core, message).await,
        Err(source) => {
            let message = format!("Failed to parse JSON response: {source}");
            push_error(core, ErrorCode::PARSE_ERROR, message, id).await
        }
    }
}
async fn answer_error(
    core: &StreamableCore,
    status: u16,
    headers: &HeaderMap,
    mut body: HttpBody,
    id: RequestId,
) -> Result<(), SdkError> {
    if content_type(headers).starts_with("application/json")
        && let Ok(content) = body.read_all().await
        && let Ok(JsonRpcMessage::Error(error)) =
            serde_json::from_slice::<RxJsonRpcMessage<RoleClient>>(&content)
    {
        let reply: RxJsonRpcMessage<RoleClient> = JsonRpcMessage::error(error.error, Some(id));
        return push(core, reply).await;
    }
    let (code, message) = if status != 404 {
        (
            ErrorCode::INTERNAL_ERROR,
            "Server returned an error response",
        )
    } else if session_id(core).is_some() {
        (ErrorCode::INVALID_REQUEST, "Session terminated")
    } else {
        (ErrorCode::METHOD_NOT_FOUND, "Not Found")
    };
    push_error(core, code, message.to_owned(), id).await?;
    drop(body.close().await);
    Ok(())
}
pub(super) async fn push(
    core: &StreamableCore,
    message: RxJsonRpcMessage<RoleClient>,
) -> Result<(), SdkError> {
    core.incoming
        .send(Incoming::Message(Box::new(message)))
        .await
        .map_err(|_| SdkError::SessionClosed)
}
pub(super) async fn push_error(
    core: &StreamableCore,
    code: ErrorCode,
    message: String,
    id: RequestId,
) -> Result<(), SdkError> {
    let error = ErrorData::new(code, message, None);
    let reply: RxJsonRpcMessage<RoleClient> = JsonRpcMessage::error(error, Some(id));
    push(core, reply).await
}
