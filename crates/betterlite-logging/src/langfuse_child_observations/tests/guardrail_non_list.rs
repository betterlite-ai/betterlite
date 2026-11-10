#![cfg(test)]

use super::guardrails::log_guardrail_spans;
use super::support::child;
use super::support::{generation, payload, provider, sdk_tracer, spans};

#[test]
fn guardrail_non_list_guardrail_information_is_rejected_by_the_payload() {
    let non_list = payload(Some(r#""not-a-list""#)).expect_err("a text field is rejected");
    assert!(non_list.is_data());
    let mapping =
        payload(Some(r#"{"guardrail_name":"pii"}"#)).expect_err("a mapping field is rejected");
    assert!(mapping.is_data());
}

#[test]
fn guardrail_non_list_guardrail_information_logs_no_span() {
    let (provider, exporter) = provider();
    let tracer = sdk_tracer(&provider);
    let parent = generation(&tracer);
    let observations = child(&tracer, &parent);
    log_guardrail_spans(&observations, None);
    assert!(spans(&exporter).is_empty());
}
