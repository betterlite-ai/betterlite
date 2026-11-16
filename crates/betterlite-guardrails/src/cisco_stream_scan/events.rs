use serde::Serialize;

use crate::cisco_conversation::Fields;
use crate::cisco_stream_scan::StreamItem;
use crate::cisco_verdict_enforcement::support::insert_serialized;

pub(super) const UNSUPPORTED_MESSAGE: &str =
    "Cisco AI Defense: unsupported streaming format — response withheld for safety";
pub(super) const SCAN_ERROR_MESSAGE: &str =
    "Cisco AI Defense streaming scan failed — response withheld.";

#[derive(Debug, Clone, Copy)]
pub(super) struct EventSpec {
    pub(super) message: &'static str,
    pub(super) kind: &'static str,
    pub(super) code: u16,
}

pub(super) const UNSUPPORTED: EventSpec = EventSpec {
    message: UNSUPPORTED_MESSAGE,
    kind: "guardrail_unsupported_stream",
    code: 400,
};

pub(super) const SCAN_ERROR: EventSpec = EventSpec {
    message: SCAN_ERROR_MESSAGE,
    kind: "guardrail_scan_error",
    code: 500,
};

#[derive(Serialize)]
struct ErrorEnvelope<'envelope> {
    error: &'envelope Fields,
}

pub(super) fn event(guardrail_name: &str, spec: EventSpec) -> Vec<StreamItem> {
    let mut error = Fields::new();
    insert_serialized(&mut error, "message", spec.message);
    insert_serialized(&mut error, "type", spec.kind);
    insert_serialized(&mut error, "code", &spec.code);
    insert_serialized(&mut error, "guardrail", guardrail_name);
    error_object(&error)
}

pub(super) fn error_object(error: &Fields) -> Vec<StreamItem> {
    let envelope = ErrorEnvelope { error };
    let json = serde_json::to_string(&envelope).unwrap_or_default();
    let line = format!("data: {json}\n\n");
    vec![StreamItem::Sse(line)]
}
