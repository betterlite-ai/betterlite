#![cfg(test)]

use super::guardrails::log_guardrail_spans;
use super::support::{generation, provider, sdk_tracer, spans};

#[test]
fn guardrail_none_payload_logs_no_span() {
    let (provider, exporter) = provider();
    let tracer = sdk_tracer(&provider);
    let parent = generation(&tracer);
    log_guardrail_spans(&tracer, &parent, None);
    assert!(spans(&exporter).is_empty());
}
