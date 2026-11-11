use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::fields::payload::StandardLoggingPayload;
use crate::posthog_logging::event::create_event_payload;
use crate::posthog_logging::tests::support::{
    failed_payload, fixed_id, number, object, payload, text,
};

fn properties(payload: &StandardLoggingPayload) -> IndexMap<String, Json> {
    create_event_payload(payload, fixed_id)
        .expect("the event builds")
        .properties
}

#[test]
fn posthog_properties_project_the_builtin_set() {
    let properties = properties(&payload());
    assert_eq!(properties.get("$ai_model"), Some(&text("gpt-4o")));
    assert_eq!(properties.get("$ai_provider"), Some(&text("openai")));
    assert_eq!(
        properties.get("$ai_input"),
        Some(&Json::List(vec![object(&[
            ("role", text("user")),
            ("content", text("hi")),
        ])]))
    );
    assert_eq!(
        properties.get("$ai_output_choices"),
        Some(&object(&[("choices", Json::List(Vec::new()))]))
    );
    assert_eq!(properties.get("$ai_input_tokens"), Some(&number(10)));
    assert_eq!(properties.get("$ai_output_tokens"), Some(&number(5)));
    assert_eq!(properties.get("$ai_trace_id"), Some(&text("trace-1")));
    assert_eq!(properties.get("$ai_span_id"), Some(&text("span-1")));
    assert_eq!(properties.get("$ai_is_error"), None);
    assert_eq!(properties.get("$ai_error"), None);
}

#[test]
fn posthog_properties_keep_the_cost_and_the_latency() {
    let properties = properties(&payload());
    assert_eq!(
        properties.get("$ai_total_cost_usd"),
        Some(&Json::Number(
            serde_json::Number::from_f64(0.5).expect("the cost is finite")
        ))
    );
    assert_eq!(properties.get("$ai_latency"), Some(&number(1)));
}

#[test]
fn posthog_properties_keep_a_zero_cost() {
    let mut payload = payload();
    payload.response_cost = Decimal::ZERO;
    let properties = properties(&payload);
    assert_eq!(properties.get("$ai_total_cost_usd"), Some(&number(0)));
}

#[test]
fn posthog_properties_mark_a_failure() {
    let properties = properties(&failed_payload());
    assert_eq!(properties.get("$ai_is_error"), Some(&Json::Bool(true)));
    assert_eq!(properties.get("$ai_error"), Some(&text("upstream refused")));
}

#[test]
fn posthog_properties_leave_out_an_absent_error_text() {
    let mut payload = failed_payload();
    payload.error_str = None;
    let properties = properties(&payload);
    assert_eq!(properties.get("$ai_is_error"), Some(&Json::Bool(true)));
    assert_eq!(properties.get("$ai_error"), None);
}

#[test]
fn posthog_properties_omit_absent_input_and_response() {
    let mut payload = payload();
    payload.messages = None;
    payload.response = None;
    let properties = properties(&payload);
    assert!(!properties.contains_key("$ai_input"));
    assert!(!properties.contains_key("$ai_output_choices"));
}

#[test]
fn posthog_properties_keep_an_empty_message_list() {
    let mut payload = payload();
    payload.messages = Some(Json::List(Vec::new()));
    let properties = properties(&payload);
    assert_eq!(properties.get("$ai_input"), Some(&Json::List(Vec::new())));
}

#[test]
fn posthog_properties_default_an_absent_provider_to_an_empty_text() {
    let mut payload = payload();
    payload.custom_llm_provider = None;
    let properties = properties(&payload);
    assert_eq!(properties.get("$ai_provider"), Some(&text("")));
}

#[test]
fn posthog_properties_carry_the_parent_run_id() {
    let mut payload = payload();
    payload.metadata.requester_metadata = Some(object(&[
        ("parent_run_id", text("run-1")),
        ("parent_id", text("span-9")),
    ]));
    let properties = properties(&payload);
    assert_eq!(properties.get("$ai_parent_id"), Some(&text("run-1")));
}

#[test]
fn posthog_properties_fall_back_to_the_parent_id() {
    let mut payload = payload();
    payload.metadata.requester_metadata = Some(object(&[("parent_id", text("span-9"))]));
    let properties = properties(&payload);
    assert_eq!(properties.get("$ai_parent_id"), Some(&text("span-9")));
}

#[test]
fn posthog_properties_leave_out_an_empty_parent_id() {
    let mut payload = payload();
    payload.metadata.requester_metadata = Some(object(&[
        ("parent_run_id", text("")),
        ("parent_id", text("")),
    ]));
    let properties = properties(&payload);
    assert_eq!(properties.get("$ai_parent_id"), None);
}
