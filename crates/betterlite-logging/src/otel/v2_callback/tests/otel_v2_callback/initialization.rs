#![cfg(test)]

use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::metrics::data::{ResourceMetrics, ScopeMetrics};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SimpleSpanProcessor};

use betterlite_logging::callback_logger::CallbackLogger as _;
use betterlite_logging::logging_pre_post::hooks::{ApiCallLogger as _, PreCallEvent};
use betterlite_logging::otel::model::metadata::wire::MetadataMap;
use betterlite_logging::otel::plumbing::context::context_slots::RequestTraceContext;
use betterlite_logging::otel::plumbing::providers::ProviderState;
use betterlite_logging::otel::v2_callback::callback::V2Callback;
use betterlite_logging::otel::v2_callback::error::CallbackError;
use betterlite_logging::otel::v2_callback::phase;
use betterlite_logging::otel::v2_callback::registry::V2Registry;
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use super::support::{
    CALLBACK, built_harness, construction_parts, exporter_spec, harness, settings,
};

#[test]
fn injected_provider_receives_the_emitted_span() {
    let harness = harness(settings());
    let phase = phase::start_phase_span(&harness.callback, "auth", &RequestTraceContext::default());
    drop(phase);
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name.as_ref(), "auth");
    assert_eq!(spans[0].instrumentation_scope.name(), "litellm");
    assert_eq!(
        spans[0].instrumentation_scope.version(),
        Some(env!("CARGO_PKG_VERSION"))
    );
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
    let phase = phase::start_phase_span(&harness.callback, "auth", &RequestTraceContext::default());
    drop(phase);
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name.as_ref(), "auth");
}

#[test]
fn callback_name_is_the_configured_one() {
    let harness = harness(settings());
    assert_eq!(harness.callback.callback_name(), CALLBACK);
}

#[test]
fn metrics_and_events_stay_off_until_the_config_asks() {
    let harness = harness(settings());
    assert!(harness.callback.logger.metrics.is_none());
    assert!(harness.callback.logger.emitter.event_recorder.is_none());
}

#[test]
fn enabling_metrics_and_events_builds_their_recorders() {
    let mut config = settings();
    config.enable_metrics = true;
    config.enable_events = true;
    let harness = harness(config);
    assert!(harness.callback.logger.metrics.is_some());
    assert!(harness.callback.logger.emitter.event_recorder.is_some());
}

#[test]
fn a_built_metric_provider_keeps_exporting() {
    let mut config = settings();
    config.enable_metrics = true;
    let spans = InMemorySpanExporter::default();
    let tracer_provider = SdkTracerProvider::builder()
        .with_span_processor(SimpleSpanProcessor::new(spans.clone()))
        .build();
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter.clone()).build())
        .build();
    let registry = V2Registry::default();
    let mut parts = construction_parts(config, &spans, Some(tracer_provider));
    parts.providers.meter_state = ProviderState::Configured(provider.clone());
    let callback = V2Callback::new(parts, &registry).expect("callback");
    drop(provider);
    callback
        .logger
        .metrics
        .as_ref()
        .expect("metric recorder")
        .record_failure(
            &MetadataMap::new(),
            Decimal::from(10_i64),
            Decimal::from(13_i64),
        )
        .expect("record");
    callback
        .recorder_providers
        .0
        .as_ref()
        .expect("held meter provider")
        .force_flush()
        .expect("flush");
    assert!(
        exporter
            .get_finished_metrics()
            .expect("finished metrics")
            .iter()
            .flat_map(ResourceMetrics::scope_metrics)
            .flat_map(ScopeMetrics::metrics)
            .any(|metric| metric.name() == "gen_ai.client.operation.duration")
    );
}

#[test]
fn pre_api_call_stores_the_open_call() {
    let harness = harness(settings());
    let messages = Json::Null;
    let details: Json = serde_json::from_value(serde_json::json!({
        "litellm_call_id": "call-1",
        "model": "gpt-4o",
    }))
    .expect("details");
    let event = PreCallEvent {
        model: "gpt-4o",
        messages: &messages,
        details: &details,
    };
    harness.callback.log_pre_api_call(&event).expect("pre call");
    let ids: Vec<String> = harness
        .callback
        .logger
        .open_calls()
        .calls
        .keys()
        .cloned()
        .collect();
    assert_eq!(ids, vec!["call-1".to_owned()]);
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
