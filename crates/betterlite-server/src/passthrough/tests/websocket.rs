use betterlite_core::api::realtime_contracts::close::CloseFrame;
use serde_json::{Map, Value};

use crate::passthrough::{CLOSE_REASON_MAX_BYTES, SENDABLE_CLOSE_CODES};

mod passthrough_client_socket_open;
mod passthrough_json_object_frame;
mod passthrough_resolved_setup;
mod passthrough_rewrite_setup_model;
mod passthrough_truncated_close_reason;
mod passthrough_upstream_close;

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other @ (Value::Null
        | Value::Bool(_)
        | Value::Number(_)
        | Value::String(_)
        | Value::Array(_)) => panic!("expected an object, got {other}"),
    }
}

fn frame(code: u16, reason: &str) -> CloseFrame {
    CloseFrame {
        code,
        reason: reason.to_owned(),
    }
}

fn rewriter(model: &str) -> String {
    format!("projects/p/locations/l/models/{model}")
}

#[test]
fn sendable_close_codes_are_the_wire_codes() {
    assert_eq!(
        SENDABLE_CLOSE_CODES,
        [
            1000, 1001, 1002, 1003, 1007, 1008, 1009, 1010, 1011, 1012, 1013, 1014
        ]
    );
    assert!(!SENDABLE_CLOSE_CODES.contains(&1004));
    assert!(!SENDABLE_CLOSE_CODES.contains(&1005));
    assert!(!SENDABLE_CLOSE_CODES.contains(&1006));
    assert!(!SENDABLE_CLOSE_CODES.contains(&1015));
}

#[test]
fn the_close_reason_budget_is_123_bytes() {
    assert_eq!(CLOSE_REASON_MAX_BYTES, 123);
}
