#![cfg(test)]

use crate::helpers::{HelperCall, invoke};

#[test]
fn span_names_decode_mcp_methods_from_their_wire_values() {
    let output = invoke(&HelperCall {
        operation: "otel_vocabulary".to_owned(),
        input: serde_json::json!({"function": "SpanName", "spans": [
            {"role": "mcp_list_tools", "method": "tools/list"},
            {"role": "service", "service_name": "redis", "call_type": null},
        ]}),
    });
    assert_eq!(output.value, Some(serde_json::json!(["tools/list", "redis"])));
}

#[test]
fn unknown_mcp_methods_are_refused() {
    let output = invoke(&HelperCall {
        operation: "otel_vocabulary".to_owned(),
        input: serde_json::json!({"function": "SpanName", "spans": [
            {"role": "mcp_list_tools", "method": "resources/list"},
        ]}),
    });
    assert_eq!(output.value, None);
}
