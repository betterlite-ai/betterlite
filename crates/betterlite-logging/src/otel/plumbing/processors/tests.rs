#![cfg(test)]
use std::collections::BTreeMap;
use std::sync::Arc;

use opentelemetry::baggage::BaggageExt as _;
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry::{Context, KeyValue};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SimpleSpanProcessor};
use rstest::rstest;

use super::{BaggageSpanProcessor, ExportProcessor, processor_for};
use crate::otel::config::ExporterSpec;
use crate::otel::plumbing::exporters::ExporterFactories;
use crate::otel::plumbing::exporters::tests::{RecordingClient, clients};
use crate::otel::plumbing::sinks::SpanSink;
use crate::otel::vocabulary::constants::attribute_keys::litellm::METADATA_PREFIX;

fn keys(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[rstest]
#[case::exact_key("user.id", true)]
#[case::exact_key_other("team.id", true)]
#[case::metadata_prefix("litellm.metadata.requester_ip", true)]
#[case::prefix_alone("litellm.metadata.", true)]
#[case::other_litellm_key("litellm.call_id", false)]
#[case::prefix_substring("x.litellm.metadata.a", false)]
#[case::key_prefix_only("user.id.extra", false)]
#[case::unrelated("session", false)]
#[case::empty("", false)]
fn baggage_allowlist_matches_a_key_or_a_prefix(#[case] key: &str, #[case] expected: bool) {
    let processor =
        BaggageSpanProcessor::new(keys(&["user.id", "team.id"]), keys(&[METADATA_PREFIX]));
    assert_eq!(processor.is_allowed(key), expected);
}

#[rstest]
#[case::custom_prefix("tenant.region", true)]
#[case::default_prefix_replaced("litellm.metadata.a", false)]
#[case::exact("exact", true)]
fn baggage_custom_prefixes_replace_the_default(#[case] key: &str, #[case] expected: bool) {
    let processor = BaggageSpanProcessor::new(keys(&["exact"]), keys(&["tenant."]));
    assert_eq!(processor.is_allowed(key), expected);
}

#[test]
fn baggage_processor_stamps_only_allowed_entries_on_the_span() {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_span_processor(BaggageSpanProcessor::new(
            keys(&["user.id"]),
            keys(&[METADATA_PREFIX]),
        ))
        .with_span_processor(SimpleSpanProcessor::new(exporter.clone()))
        .build();
    let parent = Context::new().with_baggage(vec![
        KeyValue::new("user.id", "u-1"),
        KeyValue::new("litellm.metadata.team", "blue"),
        KeyValue::new("secret", "hidden"),
    ]);
    provider
        .tracer("baggage")
        .start_with_context("operation", &parent)
        .end();
    let spans = exporter.get_finished_spans().expect("spans");
    let mut attributes: Vec<(String, String)> = spans
        .first()
        .expect("one span")
        .attributes
        .iter()
        .map(|pair| (pair.key.to_string(), pair.value.to_string()))
        .collect();
    attributes.sort();
    assert_eq!(
        attributes,
        vec![
            ("litellm.metadata.team".to_owned(), "blue".to_owned()),
            ("user.id".to_owned(), "u-1".to_owned()),
        ]
    );
}

#[test]
fn baggage_processor_leaves_a_span_without_baggage_untouched() {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_span_processor(BaggageSpanProcessor::new(
            keys(&["user.id"]),
            keys(&[METADATA_PREFIX]),
        ))
        .with_span_processor(SimpleSpanProcessor::new(exporter.clone()))
        .build();
    provider.tracer("baggage").start("operation").end();
    let spans = exporter.get_finished_spans().expect("spans");
    assert!(spans.first().expect("one span").attributes.is_empty());
}

fn sink(kind: &str) -> SpanSink {
    let recorder = Arc::new(RecordingClient::default());
    let spec = ExporterSpec {
        kind: kind.to_owned(),
        endpoint: Some("http://collector:4318".to_owned()),
        traces_endpoint: None,
        headers: None,
        owner: None,
        options: None,
        use_simple_processor: None,
        requires_headers: false,
    };
    ExporterFactories::new(clients(&recorder), BTreeMap::new())
        .exporter_from_spec(&spec)
        .expect("sink")
}

#[rstest]
#[case::console_default("console", None, true)]
#[case::in_memory_default("in_memory", None, true)]
#[case::otlp_default("otlp_http", None, false)]
#[case::otlp_json_default("http/json", None, false)]
#[case::console_forced_batch("console", Some(false), false)]
#[case::in_memory_forced_batch("in_memory", Some(false), false)]
#[case::otlp_forced_simple("otlp_http", Some(true), true)]
#[case::console_forced_simple("console", Some(true), true)]
fn processor_choice_follows_the_sink_unless_overridden(
    #[case] kind: &str,
    #[case] use_simple: Option<bool>,
    #[case] expect_simple: bool,
) {
    let processor = processor_for(sink(kind), use_simple);
    assert_eq!(
        matches!(processor, ExportProcessor::Simple(_)),
        expect_simple
    );
}

#[test]
fn processor_simple_choice_exports_a_finished_span_immediately() {
    let exporter = InMemorySpanExporter::default();
    let processor = processor_for(SpanSink::InMemory(exporter.clone()), None);
    let provider = SdkTracerProvider::builder()
        .with_span_processor(processor)
        .build();
    provider.tracer("simple").start("operation").end();
    assert_eq!(exporter.get_finished_spans().expect("spans").len(), 1);
}

#[test]
fn processor_batch_choice_exports_on_flush() {
    let exporter = InMemorySpanExporter::default();
    let processor = processor_for(SpanSink::InMemory(exporter.clone()), Some(false));
    let provider = SdkTracerProvider::builder()
        .with_span_processor(processor)
        .build();
    provider.tracer("batch").start("operation").end();
    provider.force_flush().expect("flush");
    assert_eq!(exporter.get_finished_spans().expect("spans").len(), 1);
}
