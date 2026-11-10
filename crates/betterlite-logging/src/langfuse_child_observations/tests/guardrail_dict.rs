#![cfg(test)]

use std::time::{Duration, UNIX_EPOCH};

use opentelemetry::Value;

use super::guardrails::log_guardrail_spans;
use super::support::child;
use super::support::{
    attribute, attribute_json, generation, payload, prompt_value, provider, sdk_tracer, span, spans,
};

const GUARDRAILS: &str = r#"[{
    "guardrail_name": "pii-detector",
    "guardrail_mode": "pre_call",
    "guardrail_request": "secret text",
    "guardrail_response": {"action": "masked", "count": 2},
    "start_time": 1704067200.5,
    "end_time": 1704067201.25,
    "masked_entity_count": {"PHONE": 1, "CREDIT_CARD": 2}
}]"#;

#[test]
fn guardrail_dict_entry_creates_a_child_span_with_its_metadata() {
    let (provider, exporter) = provider();
    let tracer = sdk_tracer(&provider);
    let parent = generation(&tracer);
    let observations = child(&tracer, &parent);
    let standard = payload(Some(GUARDRAILS)).expect("the payload parses");
    log_guardrail_spans(&observations, Some(&standard));
    let finished = spans(&exporter);
    assert_eq!(finished.len(), 1);
    let data = finished.first().expect("a span");
    assert_eq!(data.name, "guardrail");
    assert_eq!(data.parent_span_id, span("0102030405060708"));
    assert_eq!(
        attribute(data, "langfuse.observation.type"),
        Some(Value::from("span"))
    );
    assert_eq!(
        attribute(data, "langfuse.observation.input"),
        Some(Value::from("secret text"))
    );
    assert_eq!(
        attribute_json(data, "langfuse.observation.output"),
        Some(prompt_value(r#"{"action": "masked", "count": 2}"#))
    );
    assert_eq!(
        attribute(data, "langfuse.observation.metadata.guardrail_name"),
        Some(Value::from("pii-detector"))
    );
    assert_eq!(
        attribute(data, "langfuse.observation.metadata.guardrail_mode"),
        Some(Value::from("pre_call"))
    );
    assert_eq!(
        attribute_json(
            data,
            "langfuse.observation.metadata.guardrail_masked_entity_count"
        ),
        Some(prompt_value(r#"{"PHONE": 1, "CREDIT_CARD": 2}"#))
    );
}

#[test]
fn guardrail_dict_entry_keeps_its_start_and_end_time() {
    let (provider, exporter) = provider();
    let tracer = sdk_tracer(&provider);
    let parent = generation(&tracer);
    let observations = child(&tracer, &parent);
    let standard = payload(Some(GUARDRAILS)).expect("the payload parses");
    log_guardrail_spans(&observations, Some(&standard));
    let finished = spans(&exporter);
    let data = finished.first().expect("a span");
    assert_eq!(
        data.start_time,
        UNIX_EPOCH
            .checked_add(Duration::from_millis(1_704_067_200_500))
            .expect("a representable start")
    );
    assert_eq!(
        data.end_time,
        UNIX_EPOCH
            .checked_add(Duration::from_millis(1_704_067_201_250))
            .expect("a representable end")
    );
}
