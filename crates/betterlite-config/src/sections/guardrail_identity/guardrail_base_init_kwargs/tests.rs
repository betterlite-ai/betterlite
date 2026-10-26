#![cfg(test)]

use super::super::item::GuardrailBaseInitKwargs;

#[test]
fn has_thirteen_fields() {
    let value = "{\"guardrail_name\":null,\"default_on\":false,\"mask_request_content\":false,\"mask_response_content\":false,\"violation_message_template\":null,\"end_session_after_n_fails\":null,\"on_violation\":null,\"realtime_violation_message\":null,\"on_sensitive_data\":null,\"sensitive_data_route_to_model\":null,\"sticky_session_routing\":false,\"run_in_parallel\":false,\"only_scan_new_messages\":false}";
    let _unused: GuardrailBaseInitKwargs = serde_json::from_str(value).unwrap();
}
