#![cfg(test)]

use rstest::rstest;

use super::guardrails::log_guardrail_spans;
use super::support::child;
use super::support::{generation, payload, provider, sdk_tracer, spans};

#[rstest]
#[case::an_empty_list(Some("[]"))]
#[case::a_null_field(Some("null"))]
#[case::a_missing_field(None)]
fn guardrail_empty_information_logs_no_span(#[case] guardrails: Option<&str>) {
    let (provider, exporter) = provider();
    let tracer = sdk_tracer(&provider);
    let parent = generation(&tracer);
    let observations = child(&tracer, &parent);
    let standard = payload(guardrails).expect("the payload parses");
    log_guardrail_spans(&observations, Some(&standard));
    assert!(spans(&exporter).is_empty());
}
