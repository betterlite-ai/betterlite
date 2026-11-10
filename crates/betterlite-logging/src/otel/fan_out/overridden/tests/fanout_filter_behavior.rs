#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use opentelemetry::Context;
use opentelemetry::trace::{SpanId, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{SdkTracerProvider, SpanData, SpanProcessor};

use crate::otel::config::destination::{DestinationFields, OtelDestination, SpanScope};
use crate::otel::fan_out::attach::{SinkKey, sink_key};
use crate::otel::fan_out::overridden::OverriddenBackendFilter;
use crate::otel::fan_out::tests::support::{
    FixedScope, Recorder, SharedRecorder, destination_with, keys, root_span, span, string_attribute,
};

fn inner(recorder: &Arc<Recorder>) -> Box<dyn SpanProcessor> {
    Box::new(SharedRecorder(Arc::clone(recorder)))
}

fn filter(
    owner: Option<&str>,
    scope: SpanScope,
    sink: Option<SinkKey>,
    request: Arc<FixedScope>,
    recorder: &Arc<Recorder>,
) -> OverriddenBackendFilter {
    OverriddenBackendFilter {
        inner: inner(recorder),
        owner: owner.map(str::to_owned),
        scope,
        sink,
        scope_source: request,
    }
}

fn request_root() -> SpanData {
    root_span(vec![string_attribute("http.route", "/v1/chat")])
}

fn model_call() -> SpanData {
    span(
        vec![string_attribute("gen_ai.operation.name", "chat")],
        SpanId::from(0x0102_0304_0506_0708),
    )
}

fn account(endpoint: &str) -> SinkKey {
    sink_key(Some(endpoint), &BTreeMap::new()).expect("a sink key")
}

#[test]
fn full_scope_passes_every_span_through() {
    let recorder = Recorder::new();
    let request = FixedScope::new(vec![], false);
    let filter = filter(None, SpanScope::Full, None, request, &recorder);
    SpanProcessor::on_end(&filter, request_root());
    let delivered = recorder.delivered();
    assert_eq!(delivered.len(), 1);
    assert!(
        delivered[0]
            .attributes
            .iter()
            .any(|pair| pair.key.as_str() == "http.route")
    );
}

#[test]
fn a_suppressed_owner_backend_gets_nothing() {
    let recorder = Recorder::new();
    let request = FixedScope::new(
        vec![destination_with(
            "http://collector:4318",
            &[],
            Some("arize"),
        )],
        false,
    );
    let filter = filter(Some("arize"), SpanScope::Full, None, request, &recorder);
    SpanProcessor::on_end(&filter, request_root());
    assert!(recorder.delivered().is_empty());
}

#[test]
fn additive_mode_keeps_the_operator_copy() {
    let recorder = Recorder::new();
    let request = FixedScope::new(
        vec![destination_with(
            "http://collector:4318",
            &[],
            Some("arize"),
        )],
        true,
    );
    let filter = filter(Some("arize"), SpanScope::Full, None, request, &recorder);
    SpanProcessor::on_end(&filter, request_root());
    assert_eq!(recorder.delivered().len(), 1);
}

#[test]
fn an_anonymous_owner_is_never_suppressed() {
    let recorder = Recorder::new();
    let request = FixedScope::new(
        vec![destination_with("http://collector:4318", &[], None)],
        false,
    );
    let filter = filter(None, SpanScope::Full, None, request, &recorder);
    SpanProcessor::on_end(&filter, request_root());
    assert_eq!(recorder.delivered().len(), 1);
}

#[test]
fn llm_only_holds_the_rest_of_the_tree_back() {
    let recorder = Recorder::new();
    let request = FixedScope::new(vec![], false);
    let filter = filter(None, SpanScope::LlmOnly, None, request, &recorder);
    SpanProcessor::on_end(&filter, request_root());
    SpanProcessor::on_end(&filter, model_call());
    let delivered = recorder.delivered();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].parent_span_id, SpanId::INVALID);
    assert!(
        delivered[0]
            .attributes
            .iter()
            .any(|pair| pair.key.as_str() == "langfuse.trace.name")
    );
}

#[test]
fn a_wider_sibling_destination_widens_the_account_scope() {
    let recorder = Recorder::new();
    let request = FixedScope::new(
        vec![destination_with("http://collector:4318", &[], None)],
        false,
    );
    let filter = filter(
        None,
        SpanScope::LlmOnly,
        Some(account("http://collector:4318")),
        request,
        &recorder,
    );
    SpanProcessor::on_end(&filter, model_call());
    let delivered = recorder.delivered();
    assert_eq!(delivered.len(), 1);
    assert_eq!(
        delivered[0].parent_span_id,
        SpanId::from(0x0102_0304_0506_0708)
    );
    assert!(!keys(&delivered[0]).contains("langfuse.trace.name"));
}

#[test]
fn an_llm_only_sibling_does_not_widen_the_account_scope() {
    let recorder = Recorder::new();
    let narrow = OtelDestination::from(DestinationFields {
        endpoint: "http://collector:4318".to_owned(),
        headers: BTreeMap::new(),
        resource_attributes: BTreeMap::new(),
        callback_name: None,
        protocol: None,
        span_scope: SpanScope::LlmOnly,
    });
    let request = FixedScope::new(vec![narrow], false);
    let filter = filter(
        None,
        SpanScope::LlmOnly,
        Some(account("http://collector:4318")),
        request,
        &recorder,
    );
    SpanProcessor::on_end(&filter, model_call());
    let delivered = recorder.delivered();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].parent_span_id, SpanId::INVALID);
    assert!(keys(&delivered[0]).contains("langfuse.trace.name"));
}

#[test]
fn a_filter_without_a_sink_keeps_its_scope() {
    let recorder = Recorder::new();
    let request = FixedScope::new(
        vec![destination_with("http://collector:4318", &[], None)],
        false,
    );
    let filter = filter(None, SpanScope::LlmOnly, None, request, &recorder);
    SpanProcessor::on_end(&filter, request_root());
    assert!(recorder.delivered().is_empty());
}

#[test]
fn a_different_account_is_not_widened_by_a_full_sibling() {
    let recorder = Recorder::new();
    let request = FixedScope::new(
        vec![destination_with("http://other:4318", &[], None)],
        false,
    );
    let filter = filter(
        None,
        SpanScope::LlmOnly,
        Some(account("http://collector:4318")),
        request,
        &recorder,
    );
    SpanProcessor::on_end(&filter, request_root());
    assert!(recorder.delivered().is_empty());
}

#[test]
fn the_filter_debug_names_its_owner_and_scope_without_the_sink() {
    let recorder = Recorder::new();
    let filter = filter(
        Some("arize"),
        SpanScope::Full,
        Some(account("http://collector:4318")),
        FixedScope::new(vec![], false),
        &recorder,
    );
    let text = format!("{filter:?}");
    assert!(text.contains("OverriddenBackendFilter"));
    assert!(text.contains("arize"));
    assert!(text.contains("Full"));
    assert!(!text.contains("collector"));
}

#[test]
fn on_start_and_set_resource_reach_the_wrapped_processor() {
    let recorder = Recorder::new();
    let mut filter = filter(
        None,
        SpanScope::Full,
        None,
        FixedScope::new(vec![], false),
        &recorder,
    );
    let provider = SdkTracerProvider::builder().build();
    let mut span = provider.tracer("fan-out-tests").start("call model");
    SpanProcessor::on_start(&filter, &mut span, &Context::new());
    let resource = Resource::builder_empty()
        .with_attribute(opentelemetry::KeyValue::new("service.name", "betterlite"))
        .build();
    SpanProcessor::set_resource(&mut filter, &resource);
    assert_eq!(recorder.start_count(), 1);
    let resources = recorder.resources();
    assert_eq!(resources.len(), 1);
    assert_eq!(
        resources[0].get(&opentelemetry::Key::new("service.name")),
        Some(opentelemetry::Value::from("betterlite".to_owned()))
    );
}

#[test]
fn flush_and_shutdown_reach_the_wrapped_processor() {
    let recorder = Recorder::new();
    let request = FixedScope::new(vec![], false);
    let filter = filter(None, SpanScope::Full, None, request, &recorder);
    SpanProcessor::force_flush(&filter).unwrap();
    SpanProcessor::shutdown_with_timeout(&filter, Duration::from_secs(1)).unwrap();
    assert_eq!(recorder.flush_count(), 1);
    assert_eq!(recorder.shutdown_count(), 1);
}
