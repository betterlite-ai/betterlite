#![cfg(test)]
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use rstest::rstest;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use super::{
    ExporterKind, ProcessorKind, SpanProcessorPlan, log_exporter_plan, metric_reader_plan,
    span_processor_plan,
};
use crate::otel::legacy_exporters::headers::HeadersError;
use crate::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig, Signals};

fn config(
    exporter: Exporter,
    endpoint: Option<&str>,
    headers: Option<&str>,
) -> OpenTelemetryConfig {
    OpenTelemetryConfig {
        exporter,
        endpoint: endpoint.map(str::to_owned),
        headers: headers.map(str::to_owned),
        signals: Signals::default(),
        service_name: "litellm".to_owned(),
        deployment_environment: "production".to_owned(),
        model_id: "litellm".to_owned(),
        ignore_context_propagation: false,
        skip_set_global: false,
        capture_message_content: None,
        semconv_stability_opt_in: BTreeSet::new(),
        baggage_team_metadata_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        attributes: None,
    }
}

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn named(text: &str) -> Exporter {
    Exporter::Named(text.to_owned())
}

fn span_plan(
    base: &OpenTelemetryConfig,
    dynamic_headers: Option<&BTreeMap<String, String>>,
    override_config: Option<&OpenTelemetryConfig>,
    callback_name: Option<&str>,
) -> SpanProcessorPlan {
    span_processor_plan(base, dynamic_headers, override_config, callback_name).expect("plan")
}

#[rstest]
#[case::in_memory(Exporter::InMemory, ExporterKind::Shared, ProcessorKind::Simple, None)]
#[case::console(Exporter::Console, ExporterKind::Console, ProcessorKind::Batch, None)]
#[case::named_console(named("console"), ExporterKind::Console, ProcessorKind::Batch, None)]
#[case::otlp_http(
    named("otlp_http"),
    ExporterKind::OtlpHttp,
    ProcessorKind::Batch,
    Some("http://collector:4318/v1/traces")
)]
#[case::http_protobuf(
    named("http/protobuf"),
    ExporterKind::OtlpHttp,
    ProcessorKind::Batch,
    Some("http://collector:4318/v1/traces")
)]
#[case::http_json(
    named("http/json"),
    ExporterKind::OtlpHttp,
    ProcessorKind::Batch,
    Some("http://collector:4318/v1/traces")
)]
#[case::otlp_grpc(
    named("otlp_grpc"),
    ExporterKind::OtlpGrpc,
    ProcessorKind::Batch,
    Some("http://collector:4318/v1/traces")
)]
#[case::grpc(
    named("grpc"),
    ExporterKind::OtlpGrpc,
    ProcessorKind::Batch,
    Some("http://collector:4318/v1/traces")
)]
#[case::unknown_text(named("jaeger"), ExporterKind::Console, ProcessorKind::Batch, None)]
#[case::case_sensitive_text(named("OTLP_HTTP"), ExporterKind::Console, ProcessorKind::Batch, None)]
#[case::grpc_uppercase(named("GRPC"), ExporterKind::Console, ProcessorKind::Batch, None)]
fn span_processor_plan_picks_the_exporter_by_its_exact_text(
    #[case] exporter: Exporter,
    #[case] kind: ExporterKind,
    #[case] processor: ProcessorKind,
    #[case] endpoint: Option<&str>,
) {
    let plan = span_plan(
        &config(exporter, Some("http://collector:4318/v1/logs"), None),
        None,
        None,
        None,
    );
    assert_eq!(plan.kind, kind);
    assert_eq!(plan.processor, processor);
    assert_eq!(plan.endpoint.as_deref(), endpoint);
}

#[test]
fn span_processor_plan_logs_only_the_unknown_exporter_text() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(Warnings(Arc::clone(&events)));
    with_default(subscriber, || {
        span_processor_plan(&config(named("console"), None, None), None, None, None)
            .expect("console plan");
        span_processor_plan(&config(named("jaeger"), None, None), None, None, None)
            .expect("unknown plan");
    });
    let captured = events.lock().expect("events");
    assert_eq!(captured.len(), 1);
    assert!(captured[0].contains("unknown OpenTelemetry exporter"));
    drop(captured);
}

#[test]
fn span_processor_plan_normalizes_a_missing_endpoint_to_none() {
    let plan = span_plan(&config(named("otlp_http"), None, None), None, None, None);
    assert_eq!(plan.kind, ExporterKind::OtlpHttp);
    assert_eq!(plan.endpoint, None);
}

#[test]
fn span_processor_plan_lets_the_config_override_win() {
    let base = config(
        Exporter::Console,
        Some("http://base:4318"),
        Some("x-base=1"),
    );
    let override_config = config(
        named("otlp_http"),
        Some("http://override:4318"),
        Some("x-override=2"),
    );
    let plan = span_plan(&base, None, Some(&override_config), None);
    assert_eq!(plan.kind, ExporterKind::OtlpHttp);
    assert_eq!(
        plan.endpoint.as_deref(),
        Some("http://override:4318/v1/traces")
    );
    assert_eq!(plan.headers, env(&[("x-override", "2")]));
}

#[test]
fn span_processor_plan_ignores_the_configured_headers_when_a_config_override_sets_none() {
    let base = config(Exporter::Console, None, Some("x-base=1"));
    let override_config = config(named("otlp_http"), None, None);
    let plan = span_plan(&base, None, Some(&override_config), None);
    assert!(plan.headers.is_empty());
}

#[test]
fn span_processor_plan_prefers_non_empty_dynamic_headers() {
    let dynamic = env(&[("x-dynamic", "3")]);
    let plan = span_plan(
        &config(Exporter::Console, None, Some("x-config=1")),
        Some(&dynamic),
        None,
        None,
    );
    assert_eq!(plan.headers, dynamic);
}

#[test]
fn span_processor_plan_falls_back_from_empty_dynamic_headers_to_the_config_text() {
    let dynamic = BTreeMap::new();
    let plan = span_plan(
        &config(Exporter::Console, None, Some("x-config=1")),
        Some(&dynamic),
        None,
        None,
    );
    assert_eq!(plan.headers, env(&[("x-config", "1")]));
    assert!(dynamic.is_empty());
}

#[rstest]
#[case::console(Exporter::Console)]
#[case::shared(Exporter::InMemory)]
fn span_processor_plan_parses_headers_before_choosing_the_exporter(#[case] exporter: Exporter) {
    let error = span_processor_plan(&config(exporter, None, Some("oops")), None, None, None)
        .map(|plan| plan.kind)
        .expect_err("the headers text has no equals sign");
    assert_eq!(error, HeadersError::MissingSeparator { position: 1 });
}

#[test]
fn span_processor_plan_never_parses_the_config_headers_behind_dynamic_headers() {
    let dynamic = env(&[("x-dynamic", "3")]);
    let plan = span_plan(
        &config(Exporter::Console, None, Some("oops")),
        Some(&dynamic),
        None,
        None,
    );
    assert_eq!(plan.headers, dynamic);
}

#[test]
fn span_processor_plan_leaves_the_endpoint_unset_for_an_otlp_override_without_one() {
    let base = config(Exporter::Console, Some("http://base:4318"), None);
    let override_config = config(named("otlp_grpc"), None, None);
    let plan = span_plan(&base, None, Some(&override_config), None);
    assert_eq!(plan.kind, ExporterKind::OtlpGrpc);
    assert_eq!(plan.endpoint, None);
}

#[test]
fn log_exporter_plan_uses_the_shared_exporter_for_the_in_memory_kind() {
    let plan = log_exporter_plan(
        &config(Exporter::InMemory, Some("http://collector:4318"), None),
        None,
        &env(&[("OTEL_LOGS_EXPORTER", "console")]),
    )
    .expect("plan");
    assert_eq!(plan.kind, ExporterKind::Shared);
    assert_eq!(plan.endpoint, None);
}

#[rstest]
#[case::console(Exporter::Console, None)]
#[case::named_console(named("console"), None)]
#[case::env_console(named("otlp_http"), Some("console"))]
#[case::env_console_beats_grpc(named("grpc"), Some("console"))]
#[case::env_console_beats_unknown(named("jaeger"), Some("console"))]
fn log_exporter_plan_picks_console(
    #[case] exporter: Exporter,
    #[case] logs_exporter: Option<&str>,
) {
    let environment =
        logs_exporter.map_or_else(BTreeMap::new, |value| env(&[("OTEL_LOGS_EXPORTER", value)]));
    let plan = log_exporter_plan(
        &config(exporter, Some("http://collector:4318"), None),
        None,
        &environment,
    )
    .expect("plan");
    assert_eq!(plan.kind, ExporterKind::Console);
    assert_eq!(plan.endpoint, None);
}

#[rstest]
#[case::otlp_http(
    named("http/json"),
    ExporterKind::OtlpHttp,
    "http://collector:4318/v1/logs"
)]
#[case::otlp_grpc(
    named("otlp_grpc"),
    ExporterKind::OtlpGrpc,
    "http://collector:4318/v1/logs"
)]
#[case::env_console_uppercase_ignored(
    named("otlp_http"),
    ExporterKind::OtlpHttp,
    "http://collector:4318/v1/logs"
)]
fn log_exporter_plan_normalizes_the_endpoint_for_logs(
    #[case] exporter: Exporter,
    #[case] kind: ExporterKind,
    #[case] endpoint: &str,
) {
    let plan = log_exporter_plan(
        &config(exporter, Some("http://collector:4318/v1/traces"), None),
        None,
        &env(&[("OTEL_LOGS_EXPORTER", "Console")]),
    )
    .expect("plan");
    assert_eq!(plan.kind, kind);
    assert_eq!(plan.endpoint.as_deref(), Some(endpoint));
}

#[test]
fn log_exporter_plan_warns_for_an_unknown_text() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(Warnings(Arc::clone(&events)));
    let plan = with_default(subscriber, || {
        log_exporter_plan(&config(named("jaeger"), None, None), None, &BTreeMap::new())
    })
    .expect("plan");
    assert_eq!(plan.kind, ExporterKind::Console);
    let captured = events.lock().expect("events");
    assert_eq!(captured.len(), 1);
    assert!(captured[0].contains("Unknown log exporter 'jaeger'"));
    assert!(captured[0].contains("Supported: console, otlp_http, otlp_grpc"));
    drop(captured);
}

#[test]
fn log_exporter_plan_parses_headers_before_choosing_the_exporter() {
    let error = log_exporter_plan(
        &config(Exporter::InMemory, None, Some("a=b,,c=d")),
        None,
        &BTreeMap::new(),
    )
    .map(|plan| plan.kind)
    .expect_err("the third part has no equals sign");
    assert_eq!(error, HeadersError::MissingSeparator { position: 2 });
}

#[rstest]
#[case::console(Exporter::Console, ExporterKind::Console)]
#[case::in_memory_is_unknown(Exporter::InMemory, ExporterKind::Console)]
#[case::unknown_text(named("jaeger"), ExporterKind::Console)]
#[case::otlp_http(named("http/json"), ExporterKind::OtlpHttp)]
#[case::otlp_grpc(named("grpc"), ExporterKind::OtlpGrpc)]
fn metric_reader_plan_has_no_shared_exporter(
    #[case] exporter: Exporter,
    #[case] kind: ExporterKind,
) {
    let plan = metric_reader_plan(&config(exporter, Some("http://collector:4318"), None), None)
        .expect("plan");
    assert_eq!(plan.kind, kind);
}

#[test]
fn metric_reader_plan_normalizes_the_endpoint_for_metrics() {
    let plan = metric_reader_plan(
        &config(
            named("otlp_http"),
            Some("http://collector:4318/v1/traces"),
            None,
        ),
        None,
    )
    .expect("plan");
    assert_eq!(
        plan.endpoint.as_deref(),
        Some("http://collector:4318/v1/metrics")
    );
}

#[test]
fn metric_reader_plan_warns_for_an_in_memory_exporter() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(Warnings(Arc::clone(&events)));
    with_default(subscriber, || {
        metric_reader_plan(&config(Exporter::InMemory, None, None), None)
    })
    .expect("plan");
    let captured = events.lock().expect("events");
    assert_eq!(captured.len(), 1);
    assert!(captured[0].contains("Unknown metric exporter 'in_memory'"));
    drop(captured);
}

#[test]
fn metric_reader_plan_parses_headers_before_choosing_the_exporter() {
    let error = metric_reader_plan(&config(Exporter::Console, None, Some("oops")), None)
        .map(|plan| plan.kind)
        .expect_err("the headers text has no equals sign");
    assert_eq!(error, HeadersError::MissingSeparator { position: 1 });
}

#[derive(Clone, Default)]
struct Warnings(Arc<Mutex<Vec<String>>>);

impl<S: Subscriber> Layer<S> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        if let Ok(mut events) = self.0.lock() {
            events.push(fields.0.join(" "));
        }
    }
}

#[derive(Default)]
struct Fields(Vec<String>);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.0.push(format!("{}={value:?}", field.name()));
    }
}

#[test]
fn plans_carry_the_parsed_headers() {
    let log = log_exporter_plan(
        &config(Exporter::Console, None, Some("x-key=a,b=c")),
        None,
        &BTreeMap::new(),
    )
    .expect("log plan");
    let metric = metric_reader_plan(&config(Exporter::Console, None, Some("m=1")), None)
        .expect("metric plan");
    assert_eq!(log.headers, env(&[("x-key", "a"), ("b", "c")]));
    assert_eq!(metric.headers, env(&[("m", "1")]));
}
