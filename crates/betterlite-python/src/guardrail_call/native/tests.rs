#![cfg(test)]

use super::NativeGuardrail;

const SETTINGS: &str = r#"{"guardrail_name": "pii", "event_hook": "pre_call"}"#;
const REQUESTED: &str = r#"{"guardrails": ["pii"]}"#;

#[test]
fn mode_is_null_until_params_are_updated() {
    let mut guardrail = NativeGuardrail::new(SETTINGS, true).unwrap();
    assert_eq!(guardrail.mode().unwrap(), "null");
    guardrail
        .update_params(r#"{"guardrail": "pii", "mode": "post_call"}"#)
        .unwrap();
    assert_eq!(guardrail.mode().unwrap(), "\"post_call\"");
}

#[test]
fn settings_carry_every_field() {
    let guardrail = NativeGuardrail::new(SETTINGS, true).unwrap();
    let text = guardrail.settings().unwrap();
    assert!(text.contains("\"guardrail_name\":\"pii\""), "{text}");
    assert!(text.contains("\"sticky_session_routing\":true"), "{text}");
    assert!(text.contains("\"event_hook\":\"pre_call\""), "{text}");
}

#[test]
fn violation_messages_fill_the_context() {
    let templated = NativeGuardrail::new(
        r#"{"violation_message_template": "blocked for {reason}"}"#,
        true,
    )
    .unwrap();
    assert_eq!(
        templated
            .render_violation_message("default", r#"{"reason": "pii"}"#)
            .unwrap(),
        "blocked for pii"
    );
    assert_eq!(
        templated.render_violation_message("default", "{}").unwrap(),
        "default"
    );
}

#[test]
fn advisory_messages_land_in_the_body() {
    let (written, text) =
        NativeGuardrail::inject_advisory_message(r#"{"messages": []}"#, "note").unwrap();
    assert!(written);
    assert!(text.contains("note"), "{text}");
    let (refused, unchanged) =
        NativeGuardrail::inject_advisory_message(r#"{"input": [{"role": "user"}]}"#, "note")
            .unwrap();
    assert!(!refused);
    assert!(!unchanged.contains("note"), "{unchanged}");
}

#[test]
fn the_gate_answers_by_hook_and_request() {
    let guardrail = NativeGuardrail::new(SETTINGS, true).unwrap();
    assert!(
        guardrail
            .should_run_guardrail(REQUESTED, "pre_call", "[]")
            .unwrap()
    );
    assert!(
        !guardrail
            .should_run_guardrail(REQUESTED, "post_call", "[]")
            .unwrap()
    );
    assert!(
        !guardrail
            .should_run_guardrail(REQUESTED, "pre_call", "[\"pii\"]")
            .unwrap()
    );
}

#[test]
fn event_hook_membership_follows_the_hook() {
    let guardrail = NativeGuardrail::new(SETTINGS, true).unwrap();
    assert!(guardrail.event_hook_is_event_type("pre_call").unwrap());
    assert!(!guardrail.event_hook_is_event_type("during_call").unwrap());
    assert_eq!(guardrail.event_hook_is_event_type("not_a_hook").ok(), None);
}

#[test]
fn dynamic_params_come_from_the_requested_entry() {
    let guardrail = NativeGuardrail::new(SETTINGS, true).unwrap();
    let requested = r#"{"guardrails": [{"pii": {"extra_body": {"threshold": 0.5}}}]}"#;
    let params = guardrail.dynamic_request_body_params(requested).unwrap();
    assert_eq!(params, r#"{"threshold":0.5}"#);
    assert_eq!(
        guardrail.dynamic_request_body_params(REQUESTED).unwrap(),
        "{}"
    );
}
