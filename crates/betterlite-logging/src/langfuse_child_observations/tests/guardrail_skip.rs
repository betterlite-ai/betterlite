#![cfg(test)]

use super::guardrails::log_guardrail_spans;
use super::support::child;
use super::support::{attribute, generation, payload, provider, sdk_tracer, span, spans};

const NON_DICT_ENTRY: &str = r#"[{"guardrail_name":"pii-detector"}, "not-a-dict"]"#;

#[test]
fn guardrail_skip_a_non_dict_entry_is_rejected_by_the_payload() {
    let rejected =
        payload(Some(NON_DICT_ENTRY)).expect_err("a text entry in the guardrail list is rejected");
    assert!(rejected.is_data());
}

#[test]
fn guardrail_skip_an_entry_without_fields_is_still_logged() {
    let (provider, exporter) = provider();
    let tracer = sdk_tracer(&provider);
    let parent = generation(&tracer);
    let observations = child(&tracer, &parent);
    let standard = payload(Some("[{}]")).expect("the payload parses");
    log_guardrail_spans(&observations, Some(&standard));
    let finished = spans(&exporter);
    assert_eq!(finished.len(), 1);
    assert_eq!(finished.first().expect("a span").name, "guardrail");
    assert_eq!(
        finished.first().expect("a span").parent_span_id,
        span("0102030405060708")
    );
    let without_timing = finished.first().expect("a span");
    assert!(attribute(without_timing, "langfuse.observation.input").is_none());
    assert!(
        attribute(
            without_timing,
            "langfuse.observation.metadata.guardrail_name"
        )
        .is_none()
    );
}
