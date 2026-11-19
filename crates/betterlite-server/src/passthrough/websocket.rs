use betterlite_core::api::realtime_contracts::close::CloseFrame;
use serde_json::{Map, Value};

use super::{CLOSE_REASON_MAX_BYTES, SENDABLE_CLOSE_CODES};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebSocketState {
    Connecting,
    Connected,
    Disconnected,
}

// Starlette tracks the client and application halves separately, so both decide whether a close can still be sent
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebSocket {
    pub client_state: WebSocketState,
    pub application_state: WebSocketState,
}

#[must_use]
pub const fn client_socket_is_open(socket: WebSocket) -> bool {
    !matches!(socket.client_state, WebSocketState::Disconnected)
        && !matches!(socket.application_state, WebSocketState::Disconnected)
}

// LiteLLM's _truncated_close_reason: a close frame carries at most 123 reason bytes, and no character is split
#[must_use]
pub fn truncated_close_reason(reason: &str) -> String {
    if reason.len() <= CLOSE_REASON_MAX_BYTES {
        return reason.to_owned();
    }
    let end = reason.floor_char_boundary(CLOSE_REASON_MAX_BYTES);
    reason.get(..end).map_or_else(String::new, str::to_owned)
}

// LiteLLM's _upstream_close_to_relay: 1005, 1006 and 1015 never travel on the wire, so relaying them would build an invalid frame
#[must_use]
pub fn upstream_close_to_relay(upstream: Option<CloseFrame>) -> Option<CloseFrame> {
    let close = upstream?;
    if close.code == 1000 && close.reason.is_empty() {
        return None;
    }
    if !SENDABLE_CLOSE_CODES.contains(&close.code) && !(3000..5000).contains(&close.code) {
        return None;
    }
    Some(close)
}

// LiteLLM's _json_object_frame: only a JSON object is a frame the relay inspects
#[must_use]
pub fn json_object_frame(frame: &[u8]) -> Option<Map<String, Value>> {
    match serde_json::from_slice::<Value>(frame) {
        Ok(Value::Object(object)) => Some(object),
        Ok(_) | Err(_) => None,
    }
}

// LiteLLM's _rewrite_vertex_live_setup_model: every frame that is not a setup frame stays byte-identical
#[must_use]
pub fn rewrite_live_setup_model(text: &str, rewriter: Option<&dyn Fn(&str) -> String>) -> String {
    let Some(rewrite_with) = rewriter else {
        return text.to_owned();
    };
    let Ok(Value::Object(mut message)) = serde_json::from_str::<Value>(text) else {
        return text.to_owned();
    };
    let Some(Value::Object(setup)) = message.get("setup") else {
        return text.to_owned();
    };
    let Some(Value::String(model)) = setup.get("model") else {
        return text.to_owned();
    };
    let rewritten = rewrite_with(model);
    if rewritten == *model {
        return text.to_owned();
    }
    let mut updated_setup = setup.clone();
    updated_setup.insert("model".to_owned(), Value::String(rewritten));
    message.insert("setup".to_owned(), Value::Object(updated_setup));
    // serializing a Value cannot fail; keep the frame intact if it somehow does
    serde_json::to_string(&message).unwrap_or_else(|_error| text.to_owned())
}

// LiteLLM's _resolved_vertex_live_setup: the extractor reads the same model path the upstream will receive
#[must_use]
pub fn resolved_vertex_live_setup(
    setup_data: &Map<String, Value>,
    rewriter: Option<&dyn Fn(&str) -> String>,
) -> Map<String, Value> {
    let Some(rewrite_with) = rewriter else {
        return setup_data.clone();
    };
    let Some(Value::String(model)) = setup_data.get("model") else {
        return setup_data.clone();
    };
    let mut resolved = setup_data.clone();
    resolved.insert("model".to_owned(), Value::String(rewrite_with(model)));
    resolved
}
