use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::api::messages_chat_output::records::ResponseRecord;

use crate::guardrail_message_scope::{Object, StreamItem};

pub(crate) const MESSAGE_START: &str = "event: message_start\ndata: {\"type\": \"message_start\", \"message\": {\"id\": \"msg_1\", \"model\": \"claude-x\"}}\n\n";
pub(crate) const PING: &str = "event: ping\ndata: {\"type\": \"ping\"}\n\n";
pub(crate) const ERROR_EVENT: &str =
    "event: error\ndata: {\"type\": \"error\", \"error\": {\"message\": \"blocked\"}}\n\n";
pub(crate) const CHAT_ERROR: &str = "data: {\"error\": {\"message\": \"blocked\"}}\n\n";
pub(crate) const GOOGLE_FRAME: &str = "data: {\"candidates\": [{\"index\": 0}]}\n\n";

pub(crate) fn text_chunk(text: &str) -> StreamItem {
    StreamItem::Json(GuardrailValue::Text(text.to_owned()))
}

pub(crate) fn bytes_chunk(bytes: &[u8]) -> StreamItem {
    StreamItem::Bytes(bytes.to_vec())
}

pub(crate) fn object_chunk() -> StreamItem {
    StreamItem::Json(GuardrailValue::Object(Object::new()))
}

pub(crate) fn record(source: &str) -> ResponseRecord {
    serde_json::from_str(source).unwrap()
}
