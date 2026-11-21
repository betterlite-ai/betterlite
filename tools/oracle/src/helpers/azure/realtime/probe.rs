use async_trait::async_trait;
use betterlite_core::api::realtime_beta_protocol::headers::{
    HeaderField, HeaderPair, HeaderScope, openai_beta_realtime_header,
};
use betterlite_core::api::realtime_contracts::close::realtime_error_event;
use betterlite_core::api::realtime_contracts::error::{RealtimeContractError, WebSocketError};
use betterlite_core::api::realtime_contracts::transport::{
    ClientWebSocket, ScopedWebSocket, WebSocketScope,
};
use betterlite_openai::realtime::failure::RealtimeClose;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::error::HelperError;

pub(super) struct CloseRecord {
    pub(super) code: u16,
    pub(super) reason: Option<String>,
}

pub(super) struct RecordingSocket {
    pub(super) scope: WebSocketScope,
    pub(super) sent: Vec<String>,
    pub(super) closed: Option<CloseRecord>,
    pub(super) send_error: Option<String>,
    pub(super) close_error: Option<String>,
}

impl ScopedWebSocket for RecordingSocket {
    fn scope(&self) -> &WebSocketScope {
        &self.scope
    }
}

#[async_trait]
impl ClientWebSocket for RecordingSocket {
    async fn send_text(&mut self, data: &str) -> Result<(), WebSocketError> {
        if let Some(message) = &self.send_error {
            return Err(WebSocketError {
                message: message.clone(),
            });
        }
        self.sent.push(data.to_owned());
        Ok(())
    }

    async fn receive_text(&mut self) -> Result<String, WebSocketError> {
        Err(WebSocketError {
            message: String::from("this helper never reads a client message"),
        })
    }

    async fn close(&mut self, code: u16, reason: Option<&str>) -> Result<(), WebSocketError> {
        self.closed = Some(CloseRecord {
            code,
            reason: reason.map(str::to_owned),
        });
        self.close_error.as_ref().map_or(Ok(()), |message| {
            Err(WebSocketError {
                message: message.clone(),
            })
        })
    }
}

pub(super) async fn send_and_close(
    socket: &mut RecordingSocket,
    error_type: &str,
    message: &str,
    close: &RealtimeClose,
) -> Result<(), RealtimeContractError> {
    let event = realtime_error_event(message, error_type)?;
    drop(socket.send_text(&event).await);
    drop(socket.close(close.code, Some(&close.reason)).await);
    Ok(())
}

pub(super) fn helper_error(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("RealtimeError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

pub(super) fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| helper_error(&error.to_string()))
}

pub(super) fn beta_from_scope(headers: &[(String, String)]) -> bool {
    let pairs: Vec<HeaderPair> = headers
        .iter()
        .map(|(name, value)| {
            (
                HeaderField::Text(name.clone()),
                HeaderField::Text(value.clone()),
            )
        })
        .collect();
    openai_beta_realtime_header(Some(&HeaderScope { headers: &pairs }))
}

pub(super) fn closed_json(socket: &RecordingSocket) -> Value {
    socket.closed.as_ref().map_or(
        Value::Null,
        |frame| json!({"code": frame.code, "reason": frame.reason}),
    )
}

pub(super) fn sent_json(socket: &RecordingSocket) -> Result<Value, Box<HelperError>> {
    let sent = socket
        .sent
        .iter()
        .map(|text| {
            serde_json::from_str::<Value>(text).map_err(|error| helper_error(&error.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Array(sent))
}
