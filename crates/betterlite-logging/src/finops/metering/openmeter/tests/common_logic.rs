#![cfg(test)]

use rust_decimal::Decimal;

use super::support::settings_with;
use crate::finops::metering::openmeter::payload::build;
use crate::finops::metering::support::{EVENT_TIME, FixedClock, MODEL, event};

#[test]
fn common_logic_builds_the_cloud_event() {
    let payload = build(&settings_with(&[]), &FixedClock, &event()).unwrap();
    assert_eq!(payload.specversion, "1.0");
    assert_eq!(payload.event_type, "litellm_tokens");
    assert_eq!(payload.id.as_deref(), Some("chatcmpl-response-1"));
    assert_eq!(payload.time, EVENT_TIME);
    assert_eq!(payload.subject, "end-user-1");
    assert_eq!(payload.source, "litellm-proxy");
    assert_eq!(payload.data.model.as_deref(), Some(MODEL));
    assert_eq!(payload.data.cost, Some(Decimal::new(12, 4)));
    assert_eq!(payload.data.prompt_tokens, Some(10));
    assert_eq!(payload.data.completion_tokens, Some(20));
    assert_eq!(payload.data.total_tokens, Some(30));
}

#[test]
fn common_logic_falls_back_to_the_litellm_call_id() {
    let mut call = event();
    call.response_id = None;
    let payload = build(&settings_with(&[]), &FixedClock, &call).unwrap();
    assert_eq!(payload.id.as_deref(), Some("call-1"));
}

#[test]
fn common_logic_reads_the_event_type_from_the_environment() {
    let payload = build(
        &settings_with(&[("OPENMETER_EVENT_TYPE", "proxy_tokens")]),
        &FixedClock,
        &event(),
    )
    .unwrap();
    assert_eq!(payload.event_type, "proxy_tokens");
}

#[test]
fn common_logic_keeps_null_model_and_cost_when_the_callback_has_none() {
    let mut call = event();
    call.model = None;
    call.response_cost = None;
    let payload = build(&settings_with(&[]), &FixedClock, &call).unwrap();
    let body = serde_json::to_string(&payload).unwrap();
    assert!(body.contains(r#""model":null"#), "{body}");
    assert!(body.contains(r#""cost":null"#), "{body}");
}

#[test]
fn common_logic_omits_usage_when_the_response_has_none() {
    let mut call = event();
    call.usage = None;
    let payload = build(&settings_with(&[]), &FixedClock, &call).unwrap();
    let body = serde_json::to_string(&payload).unwrap();
    assert!(!body.contains("prompt_tokens"), "{body}");
    assert!(!body.contains("completion_tokens"), "{body}");
    assert!(!body.contains("total_tokens"), "{body}");
}

#[test]
fn common_logic_keeps_a_null_id_when_neither_id_is_present() {
    let mut call = event();
    call.response_id = None;
    call.litellm_call_id = None;
    let payload = build(&settings_with(&[]), &FixedClock, &call).unwrap();
    assert!(payload.id.is_none());
    assert!(
        serde_json::to_string(&payload)
            .unwrap()
            .contains(r#""id":null"#)
    );
}
