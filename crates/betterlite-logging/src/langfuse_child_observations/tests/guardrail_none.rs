#![cfg(test)]

use super::guardrails::log_guardrail_spans;
use super::support::child;
use super::support::{generation, provider, sdk_tracer, spans};

#[test]
fn guardrail_none_payload_logs_no_span() {
    let (provider, exporter) = provider();
    let tracer = sdk_tracer(&provider);
    let parent = generation(&tracer);
    let observations = child(&tracer, &parent);
    log_guardrail_spans(&observations, None);
    assert!(spans(&exporter).is_empty());
}
