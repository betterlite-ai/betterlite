#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use opentelemetry::trace::SpanId;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{SpanData, SpanProcessor};

use crate::otel::config::destination::{OtelDestination, SpanScope};
use crate::otel::fan_out::attach::{SinkKey, sink_key};
use crate::otel::fan_out::destination::DestinationProcessorFactory;
use crate::otel::fan_out::processor::{FanOutSettings, TenantFanOutSpanProcessor};
use crate::otel::fan_out::scope::RequestScopeSource;
use crate::otel::fan_out::tests::support::{
    FixedScope, RecordingFactory, attributes, destination, destination_with, keys, root_span, span,
    string_attribute,
};

fn settings(operator_sinks: BTreeMap<SinkKey, SpanScope>) -> FanOutSettings {
    FanOutSettings {
        operator_sinks,
        excluded_db_systems: BTreeSet::new(),
        shutdown_drain: Duration::from_millis(200),
        pending_drains: 64,
        drain_pool: None,
    }
}

fn fanout(
    destinations: Vec<OtelDestination>,
    additive: bool,
    factory: Arc<RecordingFactory>,
    settings: FanOutSettings,
) -> TenantFanOutSpanProcessor {
    let scope: Arc<dyn RequestScopeSource> = FixedScope::new(destinations, additive);
    let build: Arc<dyn DestinationProcessorFactory> = factory;
    TenantFanOutSpanProcessor::new(scope, build, settings)
}

fn operator_sink(
    endpoint: &str,
    headers: &BTreeMap<String, String>,
    scope: SpanScope,
) -> BTreeMap<SinkKey, SpanScope> {
    let key = sink_key(Some(endpoint), headers).expect("a sink key");
    BTreeMap::from([(key, scope)])
}

fn model_call() -> SpanData {
    root_span(vec![
        string_attribute("gen_ai.operation.name", "chat"),
        string_attribute("http.route", "/v1/chat"),
    ])
}

fn request_root() -> SpanData {
    root_span(vec![
        string_attribute("http.route", "/v1/chat"),
        string_attribute("error.message", "proxy failed"),
    ])
}

#[test]
fn every_destination_receives_the_filtered_span() {
    let factory = RecordingFactory::new();
    let fanout = fanout(
        vec![
            destination("http://first:4318", SpanScope::Full),
            destination("http://second:4318", SpanScope::Full),
        ],
        false,
        Arc::clone(&factory),
        settings(BTreeMap::new()),
    );
    SpanProcessor::on_end(&fanout, request_root());
    assert_one_filtered_span(&factory, "http://first:4318");
    assert_one_filtered_span(&factory, "http://second:4318");
}

fn assert_one_filtered_span(factory: &RecordingFactory, endpoint: &str) {
    let recorder = factory.recorder_for(endpoint).expect("built");
    let delivered = recorder.delivered();
    assert_eq!(delivered.len(), 1);
    let kept = keys(&delivered[0]);
    assert!(kept.contains("http.route"));
    assert!(!kept.contains("error.message"));
}

#[test]
fn the_operator_exporter_already_writing_an_account_keeps_its_only_copy() {
    let factory = RecordingFactory::new();
    let target = destination_with("http://collector:4318", &[], Some("arize"));
    let operator_sinks = operator_sink("http://collector:4318", &BTreeMap::new(), SpanScope::Full);
    let fanout = fanout(
        vec![target],
        true,
        Arc::clone(&factory),
        settings(operator_sinks),
    );
    SpanProcessor::on_end(&fanout, model_call());
    assert!(factory.endpoints().is_empty());
}

#[test]
fn a_suppressed_operator_exporter_does_not_hold_the_fan_out_back() {
    let factory = RecordingFactory::new();
    let target = destination_with("http://collector:4318", &[], Some("arize"));
    let operator_sinks = operator_sink("http://collector:4318", &BTreeMap::new(), SpanScope::Full);
    let fanout = fanout(
        vec![target],
        false,
        Arc::clone(&factory),
        settings(operator_sinks),
    );
    SpanProcessor::on_end(&fanout, model_call());
    assert_eq!(
        factory.endpoints(),
        vec!["http://collector:4318".to_owned()]
    );
}

#[test]
fn an_operator_exporter_narrowed_to_model_calls_gets_only_those() {
    let factory = RecordingFactory::new();
    let target = destination_with("http://collector:4318", &[], Some("arize"));
    let operator_sinks = operator_sink(
        "http://collector:4318",
        &BTreeMap::new(),
        SpanScope::LlmOnly,
    );
    let fanout = fanout(
        vec![target],
        true,
        Arc::clone(&factory),
        settings(operator_sinks),
    );
    SpanProcessor::on_end(&fanout, request_root());
    SpanProcessor::on_end(&fanout, model_call());
    let recorder = factory
        .recorder_for("http://collector:4318")
        .expect("built");
    let delivered = recorder.delivered();
    assert_eq!(delivered.len(), 1);
    let kept = keys(&delivered[0]);
    assert!(kept.contains("http.route"));
    assert!(!kept.contains("gen_ai.operation.name"));
}

#[test]
fn an_llm_only_destination_receives_only_model_calls_as_roots() {
    let factory = RecordingFactory::new();
    let fanout = fanout(
        vec![destination("http://collector:4318", SpanScope::LlmOnly)],
        false,
        Arc::clone(&factory),
        settings(BTreeMap::new()),
    );
    SpanProcessor::on_end(&fanout, request_root());
    SpanProcessor::on_end(
        &fanout,
        span(
            vec![string_attribute("gen_ai.operation.name", "chat")],
            SpanId::INVALID,
        ),
    );
    let recorder = factory
        .recorder_for("http://collector:4318")
        .expect("built");
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
fn an_excluded_database_span_is_never_delivered() {
    let mut settings = settings(BTreeMap::new());
    settings.excluded_db_systems = BTreeSet::from(["postgresql".to_owned()]);
    let factory = RecordingFactory::new();
    let fanout = fanout(
        vec![destination("http://collector:4318", SpanScope::Full)],
        false,
        Arc::clone(&factory),
        settings,
    );
    SpanProcessor::on_end(
        &fanout,
        root_span(attributes(&[(
            "db.system.name",
            opentelemetry::Value::from("postgresql".to_owned()),
        )])),
    );
    SpanProcessor::on_end(
        &fanout,
        root_span(attributes(&[(
            "db.system.name",
            opentelemetry::Value::from("mysql".to_owned()),
        )])),
    );
    let recorder = factory
        .recorder_for("http://collector:4318")
        .expect("built");
    assert_eq!(recorder.delivered().len(), 1);
}

#[test]
fn an_unbuildable_destination_does_not_stop_the_others() {
    let factory = Arc::new(RecordingFactory {
        built: Mutex::new(Vec::new()),
        unbuildable: BTreeSet::from(["http://broken:4318".to_owned()]),
        recorders: Mutex::new(Vec::new()),
        flush_fails: false,
    });
    let fanout = fanout(
        vec![
            destination("http://broken:4318", SpanScope::Full),
            destination("http://working:4318", SpanScope::Full),
        ],
        false,
        Arc::clone(&factory),
        settings(BTreeMap::new()),
    );
    SpanProcessor::on_end(&fanout, model_call());
    assert_eq!(factory.endpoints(), vec!["http://working:4318".to_owned()]);
    assert_eq!(
        factory
            .recorder_for("http://working:4318")
            .expect("built")
            .delivered()
            .len(),
        1
    );
}

#[test]
fn force_flush_reaches_every_processor_and_reports_a_failure() {
    let factory = Arc::new(RecordingFactory {
        built: Mutex::new(Vec::new()),
        unbuildable: BTreeSet::new(),
        recorders: Mutex::new(Vec::new()),
        flush_fails: true,
    });
    let fanout = fanout(
        vec![
            destination("http://first:4318", SpanScope::Full),
            destination("http://second:4318", SpanScope::Full),
        ],
        false,
        Arc::clone(&factory),
        settings(BTreeMap::new()),
    );
    SpanProcessor::on_end(&fanout, model_call());
    assert!(SpanProcessor::force_flush(&fanout).is_err());
    let recorders = factory.recorders.lock().unwrap().clone();
    assert_eq!(recorders.len(), 2);
    assert!(recorders.iter().all(|recorder| recorder.flush_count() == 1));
}

#[test]
fn shutdown_closes_every_destination_and_refuses_new_spans() {
    let factory = RecordingFactory::new();
    let fanout = fanout(
        vec![destination("http://collector:4318", SpanScope::Full)],
        false,
        Arc::clone(&factory),
        settings(BTreeMap::new()),
    );
    SpanProcessor::on_end(&fanout, model_call());
    SpanProcessor::shutdown_with_timeout(&fanout, Duration::from_secs(5)).expect("closed");
    let recorder = factory
        .recorder_for("http://collector:4318")
        .expect("built");
    assert_eq!(recorder.shutdown_count(), 1);
    SpanProcessor::on_end(&fanout, model_call());
    assert_eq!(recorder.delivered().len(), 1);
    assert_eq!(factory.build_count("http://collector:4318"), 1);
}

#[test]
fn the_settings_debug_names_itself() {
    let text = format!("{:?}", settings(BTreeMap::new()));
    assert!(text.contains("FanOutSettings"));
    assert!(text.contains("pending_drains"));
}

#[test]
fn the_fan_out_debug_keeps_credentials_out() {
    let mut headers = BTreeMap::new();
    headers.insert("authorization".to_owned(), "Bearer sk-secret".to_owned());
    let fanout = fanout(
        vec![],
        false,
        RecordingFactory::new(),
        settings(operator_sink(
            "http://collector:4318",
            &headers,
            SpanScope::Full,
        )),
    );
    let text = format!("{fanout:?}");
    assert!(text.contains("FanOutInner"));
    assert!(text.contains("shutdown_drain"));
    assert!(!text.contains("sk-secret"));
}

#[test]
fn the_provider_resource_reaches_every_destination_processor() {
    let factory = RecordingFactory::new();
    let mut fanout = fanout(
        vec![destination("http://collector:4318", SpanScope::Full)],
        false,
        Arc::clone(&factory),
        settings(BTreeMap::new()),
    );
    let resource = Resource::builder_empty()
        .with_attribute(opentelemetry::KeyValue::new("service.name", "betterlite"))
        .build();
    SpanProcessor::set_resource(&mut fanout, &resource);
    SpanProcessor::on_end(&fanout, model_call());
    let built = factory.built.lock().unwrap();
    assert_eq!(built.len(), 1);
    assert_eq!(
        built[0].1.get(&opentelemetry::Key::new("service.name")),
        Some(opentelemetry::Value::from("betterlite".to_owned()))
    );
}
