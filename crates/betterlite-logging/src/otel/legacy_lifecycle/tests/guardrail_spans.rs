use crate::support::instant;
use betterlite_logging::otel::legacy_lifecycle::lifecycle::LifecycleEvent;
use betterlite_logging::otel::model::metadata::wire::MetadataMap;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_models::decisions::Json;
use opentelemetry::trace::Status;
use opentelemetry_sdk::trace::SpanData;
use std::time::Duration;

use crate::support::{Rig, attribute, call, simple_kwargs, text_attribute, with_team};

fn entry() -> OtelModelValue {
    serde_json::from_str(
        r#"{
        "guardrail_name": "pii",
        "guardrail_mode": "pre_call",
        "guardrail_status": "guardrail_intervened",
        "start_time": 1704067200.5,
        "end_time": 1704067200.75,
        "masked_entity_count": {"EMAIL": 2},
        "guardrail_response": {"blocked": true, "reason": "pii"},
        "violation_categories": ["pii", "hate"],
        "guardrail_action": "GUARDRAIL_INTERVENED",
        "guardrail_usage": {"units": 3},
        "guardrail_cost": 0.25,
        "guardrail_cost_in_spend": true
    }"#,
    )
    .expect("guardrail entry")
}

fn with_guardrails(kwargs: &mut MetadataMap, entries: Vec<OtelModelValue>) {
    let Some(OtelModelValue::Object(payload)) = kwargs.get_mut("standard_logging_object") else {
        panic!("standard_logging_object must be an object");
    };
    payload.insert(
        "guardrail_information".to_owned(),
        OtelModelValue::Array(entries),
    );
}

fn guardrails(rig: &Rig) -> Vec<SpanData> {
    rig.spans()
        .into_iter()
        .filter(|span| span.name == "guardrail")
        .collect()
}

#[test]
fn guardrail_spans_are_named_and_stamped_with_their_identity() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let mut kwargs = simple_kwargs("call-1");
    with_guardrails(&mut kwargs, vec![entry()]);
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("success lifecycle");
    let spans = guardrails(&rig);
    assert_eq!(spans.len(), 1);
    let span = &spans[0];
    assert_eq!(text_attribute(span, "openinference.span.kind"), "GUARDRAIL");
    assert_eq!(text_attribute(span, "guardrail_name"), "pii");
    assert_eq!(text_attribute(span, "guardrail_mode"), "pre_call");
    assert_eq!(
        text_attribute(span, "guardrail_status"),
        "guardrail_intervened"
    );
    assert_eq!(
        text_attribute(span, "guardrail_action"),
        "GUARDRAIL_INTERVENED"
    );
    assert_eq!(
        span.parent_span_id,
        rig.span("litellm_request").span_context.span_id(),
        "the guardrail span hangs off the request span"
    );
}

#[test]
fn guardrail_spans_dedupe_each_entry_and_keep_distinct_ones() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let mut kwargs = simple_kwargs("call-2");
    let other = serde_json::from_str(
        r#"{
        "guardrail_name": "toxicity",
        "guardrail_mode": "pre_call",
        "start_time": 1704067200.5,
        "violation_categories": []
    }"#,
    )
    .expect("second guardrail");
    with_guardrails(&mut kwargs, vec![entry(), other, entry()]);
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("success lifecycle");
    let spans = guardrails(&rig);
    assert_eq!(spans.len(), 2, "one span per real guardrail invocation");
    let names: Vec<String> = spans
        .iter()
        .map(|span| text_attribute(span, "guardrail_name"))
        .collect();
    assert!(names.contains(&"pii".to_owned()));
    assert!(names.contains(&"toxicity".to_owned()));
}

#[test]
fn guardrail_spans_record_the_execution_window() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let mut kwargs = simple_kwargs("call-3");
    with_guardrails(&mut kwargs, vec![entry()]);
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("success lifecycle");
    let span = &guardrails(&rig)[0];
    assert_eq!(
        span.start_time,
        instant(1_704_067_200.5),
        "start comes from the entry, not the request"
    );
    assert_eq!(
        span.end_time,
        instant(1_704_067_200.75),
        "end comes from the entry, not the request"
    );
    // Epoch seconds become nanoseconds through the same f64 conversion as LiteLLM.
    assert_eq!(
        span.end_time
            .duration_since(span.start_time)
            .expect("ordered"),
        Duration::from_nanos(250_000_128)
    );
}

#[test]
fn guardrail_spans_serialize_the_payload_attributes() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let mut kwargs = simple_kwargs("call-4");
    with_guardrails(&mut kwargs, vec![entry()]);
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("success lifecycle");
    let span = &guardrails(&rig)[0];
    assert_json_attribute(span, "masked_entity_count", r#"{"EMAIL":2}"#);
    assert_json_attribute(
        span,
        "guardrail_response",
        r#"{"blocked":true,"reason":"pii"}"#,
    );
    assert_json_attribute(span, "guardrail_violation_categories", r#"["pii","hate"]"#);
    assert_json_attribute(span, "guardrail_usage", r#"{"units":3}"#);
    assert_eq!(
        attribute(span, "guardrail_cost"),
        Some(&opentelemetry::Value::F64(0.25))
    );
    assert_eq!(
        attribute(span, "guardrail_cost_in_spend"),
        Some(&opentelemetry::Value::Bool(true))
    );
}

fn assert_json_attribute(span: &SpanData, key: &str, expected: &str) {
    let actual: Json = serde_json::from_str(&text_attribute(span, key)).expect("JSON attribute");
    let expected_value: Json = serde_json::from_str(expected).expect("expected JSON");
    assert_eq!(actual, expected_value, "attribute {key}");
}

#[test]
fn guardrail_spans_carry_the_team_attributes() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let mut kwargs = simple_kwargs("call-5");
    with_guardrails(&mut kwargs, vec![entry()]);
    with_team(&mut kwargs);
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("success lifecycle");
    let span = &guardrails(&rig)[0];
    assert_eq!(
        text_attribute(span, "metadata.user_api_key_team_id"),
        "team-a"
    );
    assert_eq!(
        text_attribute(span, "metadata.user_api_key_team_alias"),
        "team-alias"
    );
}

#[test]
fn guardrail_spans_from_request_data_share_the_lifecycle_dedupe_state() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let mut kwargs = simple_kwargs("call-6");
    with_guardrails(&mut kwargs, vec![entry()]);
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("success lifecycle");
    let request_data = request_data("call-6");
    lifecycle
        .emit_request_guardrails(&request_data, None)
        .expect("request data guardrails");
    assert_eq!(
        guardrails(&rig).len(),
        1,
        "the pre-call entry is not emitted twice"
    );
}

#[test]
fn guardrail_spans_are_skipped_without_guardrail_information() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    lifecycle
        .handle(&call(&simple_kwargs("call-7")), LifecycleEvent::Success)
        .expect("success lifecycle");
    assert!(guardrails(&rig).is_empty());
}

#[test]
fn guardrail_spans_come_from_the_request_data_bucket() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let request_data = request_data("call-8");
    lifecycle
        .emit_request_guardrails(&request_data, None)
        .expect("request data guardrails");
    let spans = guardrails(&rig);
    assert_eq!(spans.len(), 1);
    assert_eq!(text_attribute(&spans[0], "guardrail_name"), "pii");
}

#[rstest::rstest]
#[case::masked_entities("masked_entity_count")]
#[case::response("guardrail_response")]
#[case::usage("guardrail_usage")]
#[case::cost("guardrail_cost")]
fn guardrail_spans_skip_null_payload_attributes(#[case] key: &str) {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let mut kwargs = simple_kwargs("call-null");
    with_guardrails(
        &mut kwargs,
        vec![
            serde_json::from_str(
                r#"{
            "guardrail_name": "pii-null",
            "guardrail_mode": "pre_call",
            "start_time": 1704067200.5,
            "end_time": 1704067200.75,
            "masked_entity_count": null,
            "guardrail_response": null,
            "guardrail_usage": null,
            "guardrail_cost": null
        }"#,
            )
            .expect("null fields"),
        ],
    );
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("success lifecycle");
    let span = &guardrails(&rig)[0];
    assert_eq!(attribute(span, key), None);
}

#[test]
fn guardrail_spans_on_failure_are_children_of_the_error_span() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let mut kwargs = simple_kwargs("failed-guardrail");
    with_guardrails(&mut kwargs, vec![entry()]);

    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Failure)
        .expect("failure lifecycle");

    let primary = rig.span("litellm_request");
    assert!(matches!(primary.status, Status::Error { .. }));
    let spans = guardrails(&rig);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].parent_span_id, primary.span_context.span_id());
    assert_eq!(text_attribute(&spans[0], "guardrail_name"), "pii");
}

fn request_data(call_id: &str) -> MetadataMap {
    let metadata = MetadataMap::from([
        (
            "litellm_call_id".to_owned(),
            OtelModelValue::String(call_id.to_owned()),
        ),
        (
            "standard_logging_guardrail_information".to_owned(),
            OtelModelValue::Array(vec![entry()]),
        ),
    ]);
    MetadataMap::from([("metadata".to_owned(), OtelModelValue::Object(metadata))])
}
