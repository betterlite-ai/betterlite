use std::io;
use std::sync::Arc;

use bytes::Bytes;
use http::header::CONTENT_TYPE;
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
use crate::client::sdk_transport::origin::{redirect_note, send_within_origin};

use super::streamable::{Incoming, StreamableCore};

pub(super) fn content_type(headers: &HeaderMap) -> String {
    let value = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    match value {
        Some(value) => value.to_lowercase(),
        None => String::new(),
    }
}

impl StreamableCore {
    pub(super) async fn send(
        self: Arc<Self>,
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
            self.clear_protocol_version();
        }
        let body = serde_json::to_vec(&item).map_err(SdkError::Json)?;
        let request = McpHttpRequest {
            method: Method::POST,
            url: self.url.clone(),
            headers: self.headers(),
            body: Bytes::from(body),
            read_timeout: None,
        };
        let mut response = send_within_origin(&self.http, request).await?;
        let request_id = match &item {
            JsonRpcMessage::Request(request) => Some(request.id.clone()),
            JsonRpcMessage::Response(_)
            | JsonRpcMessage::Notification(_)
            | JsonRpcMessage::Error(_) => None,
        };
        match request_id {
            Some(id) => self.answer(response, id, initialize).await?,
            None => drop(response.body.close().await),
        }
        if initialized {
            let core = Arc::clone(&self);
            self.tasks.spawn(async move { core.run_get_stream().await });
        }
        Ok(())
    }

    async fn answer(
        &self,
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
            self.push_error(
                ErrorCode::INVALID_REQUEST,
                "server answered a request with 202 Accepted".to_owned(),
                id,
            )
            .await?;
            drop(body.close().await);
            return Ok(());
        }
        if let Some(note) = redirect_note(&self.url, status, &headers) {
            warn!("{note}");
            self.push_error(ErrorCode::INVALID_REQUEST, note.trim_start().to_owned(), id)
                .await?;
            drop(body.close().await);
            return Ok(());
        }
        if status >= 400 {
            return self.answer_error(status, &headers, body, id).await;
        }
        if initialize {
            self.note_session(&headers);
        }
        let kind = content_type(&headers);
        if kind.starts_with("application/json") {
            return self.answer_json(body, id).await;
        }
        if kind.starts_with("text/event-stream") {
            return self.read_events(body, id).await;
        }
        self.push_error(
            ErrorCode::INVALID_REQUEST,
            format!("Unexpected content type: {kind}"),
            id,
        )
        .await?;
        drop(body.close().await);
        Ok(())
    }

    async fn answer_json(&self, mut body: HttpBody, id: RequestId) -> Result<(), SdkError> {
        let content = body.read_all().await?;
        match serde_json::from_slice::<RxJsonRpcMessage<RoleClient>>(&content) {
            Ok(message) => self.push(message).await,
            Err(source) => {
                self.push_error(
                    ErrorCode::PARSE_ERROR,
                    format!("Failed to parse JSON response: {source}"),
                    id,
                )
                .await
            }
        }
    }

    async fn answer_error(
        &self,
        status: u16,
        headers: &HeaderMap,
        mut body: HttpBody,
        id: RequestId,
    ) -> Result<(), SdkError> {
        let kind = content_type(headers);
        if kind.starts_with("application/json") {
            let content = body.read_all().await?;
            let parsed = serde_json::from_slice::<RxJsonRpcMessage<RoleClient>>(&content);
            if let Ok(JsonRpcMessage::Error(error)) = parsed {
                let reply: RxJsonRpcMessage<RoleClient> =
                    JsonRpcMessage::error(error.error, Some(id));
                return self.push(reply).await;
            }
        }
        let (code, message) = if status == 404 {
            match self.session_id() {
                Some(_) => (ErrorCode::INVALID_REQUEST, "Session terminated"),
                None => (ErrorCode::METHOD_NOT_FOUND, "Not Found"),
            }
        } else {
            (
                ErrorCode::INTERNAL_ERROR,
                "Server returned an error response",
            )
        };
        self.push_error(code, message.to_owned(), id).await?;
        drop(body.close().await);
        Ok(())
    }

    pub(super) async fn push(&self, message: RxJsonRpcMessage<RoleClient>) -> Result<(), SdkError> {
        match self.incoming.send(Incoming::Message(message)).await {
            Ok(()) => Ok(()),
            Err(_) => Err(SdkError::Io(Arc::new(io::Error::other(
                "the MCP session is closed",
            )))),
        }
    }

    pub(super) async fn push_error(
        &self,
        code: ErrorCode,
        message: String,
        id: RequestId,
    ) -> Result<(), SdkError> {
        let error = ErrorData::new(code, message, None);
        let reply: RxJsonRpcMessage<RoleClient> = JsonRpcMessage::error(error, Some(id));
        self.push(reply).await
    }
}
