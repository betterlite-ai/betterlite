#![cfg(test)]

use super::{Input, Op};

#[test]
fn provider_passthrough_matches_helper_contract() {
    let input = Input::Provider {
        custom_llm_provider: Some("vendor_new".to_owned()),
    };
    let output = <Op as crate::helpers::Helper>::run(&input).unwrap();
    assert_eq!(output, serde_json::json!("vendor_new"));
}

#[test]
fn metrics_only_role_is_null() {
    let input = Input::SpanRole {
        service_name: "router".to_owned(),
    };
    let output = <Op as crate::helpers::Helper>::run(&input).unwrap();
    assert_eq!(output, serde_json::Value::Null);
    let invoked = crate::helpers::invoke(&crate::helpers::HelperCall {
        operation: "otel_vocabulary".to_owned(),
        input: serde_json::json!({"function": "SpanRole", "service_name": "router"}),
    });
    assert_eq!(invoked.value, Some(serde_json::Value::Null));
}
