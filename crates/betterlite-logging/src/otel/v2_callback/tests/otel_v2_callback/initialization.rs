#![cfg(test)]

use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SimpleSpanProcessor};

use betterlite_logging::otel::v2_callback::callback::V2Callback;
use betterlite_logging::otel::v2_callback::error::CallbackError;
use betterlite_logging::otel::v2_callback::registry::V2Registry;

use super::support::{built_harness, construction_parts, exporter_spec, harness, settings};

#[test]
fn injected_provider_receives_the_emitted_span() {
    let harness = harness(settings());
    let phase = harness.callback.start_phase_span("auth");
    drop(phase);
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name.as_ref(), "auth");
}

#[test]
fn tracer_provider_is_the_injected_one() {
    let harness = harness(settings());
    let tracer = harness.callback.tracer_provider.tracer("probe");
    let mut span = tracer.start("probe");
    span.end();
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name.as_ref(), "probe");
}

#[test]
fn builds_a_provider_when_none_is_injected() {
    let mut config = settings();
    config.exporters = vec![exporter_spec("b3853_memory")];
    let harness = built_harness(config);
    let phase = harness.callback.start_phase_span("auth");
    drop(phase);
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name.as_ref(), "auth");
}

#[test]
fn unknown_mapper_names_fail_construction() {
    let spans = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_span_processor(SimpleSpanProcessor::new(spans.clone()))
        .build();
    let mut config = settings();
    config.mapper_names = vec!["nope".to_owned()];
    let registry = V2Registry::default();
    let result = V2Callback::new(
        construction_parts(config, &spans, Some(provider)),
        &registry,
    );
    assert!(matches!(result, Err(CallbackError::Mapper(_))));
    assert!(registry.registered_v2_logger().is_none());
}
