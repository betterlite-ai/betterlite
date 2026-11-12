use super::failure;
use super::hooks::ErrorInput;
use crate::helpers::HelperResult;
use betterlite_core::api::realtime_contracts::close::{
    BackendClose, CloseFrame, ConnectionClosed, backend_close_from, client_close_code,
    realtime_error_event, upstream_handshake_close_code, websocket_close_reason,
};
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Debug, Deserialize)]
pub(crate) struct CloseHelpersInput {
    error: ErrorInput,
    reason_messages: Vec<ReasonInput>,
    close_codes: Vec<u16>,
    handshake_statuses: Vec<u16>,
    backend_closes: Vec<FrameInput>,
    backend_errors: Vec<BackendErrorInput>,
}
#[derive(Debug, Deserialize)]
struct ReasonInput {
    message: String,
    fallback: String,
}
#[derive(Debug, Deserialize)]
struct FrameInput {
    code: u16,
    reason: String,
}
#[derive(Debug, Deserialize)]
struct BackendErrorInput {
    received: Option<FrameInput>,
}

pub(super) fn close_helpers(input: &CloseHelpersInput) -> HelperResult {
    let event = realtime_error_event(&input.error.message, "server_error")
        .map_err(|error| failure(error.to_string()))?;
    let parsed: Value = serde_json::from_str(&event).map_err(|error| failure(error.to_string()))?;
    Ok(json!({
        "error_event": parsed,
        "close_reasons": input
            .reason_messages
            .iter()
            .map(|item| websocket_close_reason(&item.message, &item.fallback))
            .collect::<Vec<String>>(),
        "client_close_codes": input
            .close_codes
            .iter()
            .map(|code| client_close_code(*code))
            .collect::<Vec<u16>>(),
        "handshake_close_codes": input
            .handshake_statuses
            .iter()
            .map(|status| upstream_handshake_close_code(*status))
            .collect::<Vec<u16>>(),
        "backend_messages": input
            .backend_closes
            .iter()
            .map(|item| BackendClose {
                code: item.code,
                reason: item.reason.clone(),
            })
            .map(|close| close.message())
            .collect::<Vec<String>>(),
        "backend_closes": backend_closes(input),
    }))
}
fn backend_closes(input: &CloseHelpersInput) -> Vec<Value> {
    input
        .backend_errors
        .iter()
        .map(|document| {
            let received = document.received.as_ref().map(|frame| CloseFrame {
                code: frame.code,
                reason: frame.reason.clone(),
            });
            let error = ConnectionClosed {
                received,
                message: "the upstream websocket closed".to_owned(),
            };
            let close = backend_close_from(&error);
            json!({
                "code": close.code,
                "reason": error.received.as_ref().map(|_| close.reason.clone()),
                "reason_is_error_text": close.reason == error.to_string(),
            })
        })
        .collect()
}
