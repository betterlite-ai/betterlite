#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry::{Key, KeyValue};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};
use rstest::rstest;

use crate::otel::config::destination::{DestinationFields, OtelDestination, SpanScope};
use crate::otel::config::{CaptureMessageContent, ExporterOwner, ExporterSpec, Settings};
use crate::otel::fan_out::attributes::LANGFUSE_TRACE_NAME;
use crate::otel::fan_out::scope::{RequestOtelScope, RequestScopeSource};
use crate::otel::plumbing::exporters::{
    ExporterError, ExporterFactories, ExporterFactory, HttpClients,
};
use crate::otel::plumbing::processors::BaggageSpanProcessor;
use crate::otel::plumbing::sinks::SpanSink;
use crate::otel::vocabulary::constants::attribute_keys::gen_ai;
use crate::otel_provider_assembly::tracer_provider_assembly::{
    AssemblyOptions, build_tracer_provider, owner_name, provider_resource, wire_sink_key,
};

const COLLECTOR: &str = "http://collector:4318";

#[derive(Debug)]
struct NoServer;

#[async_trait::async_trait]
impl HttpClient for NoServer {
    async fn send_bytes(&self, _request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        Err(Box::from("the assembly tests never export"))
    }
}

#[derive(Debug)]
struct FixedScope(RequestOtelScope);

impl RequestScopeSource for FixedScope {
    fn scope(&self) -> RequestOtelScope {
        self.0.clone()
    }
}

fn factories_with(custom: BTreeMap<String, ExporterFactory>) -> ExporterFactories {
    let client: Arc<dyn HttpClient> = Arc::new(NoServer);
    ExporterFactories::new(
        HttpClients {
            traces: Arc::clone(&client),
            metrics: Arc::clone(&client),
            logs: client,
        },
        custom,
    )
}

fn no_request_scope() -> Arc<dyn RequestScopeSource> {
    Arc::new(FixedScope(RequestOtelScope::default()))
}

fn spec(kind: &str) -> ExporterSpec {
    ExporterSpec {
        kind: kind.to_owned(),
        endpoint: None,
        traces_endpoint: None,
        headers: None,
        owner: None,
        options: None,
        use_simple_processor: None,
        requires_headers: false,
    }
}

fn settings(exporters: Vec<ExporterSpec>) -> Settings {
    Settings {
        exporters,
        service_name: "betterlite".to_owned(),
        deployment_environment: None,
        enable_metrics: false,
        enable_events: false,
        capture_message_content: CaptureMessageContent::NoContent,
        legacy_compat: false,
        langfuse_span_scope: SpanScope::Full,
        excluded_services: BTreeSet::new(),
        mapper_names: Vec::new(),
        resource_attributes: BTreeMap::new(),
        baggage_promoted_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        baggage_team_metadata_keys: Vec::new(),
    }
}

fn assembled(config: &Settings, options: AssemblyOptions) -> String {
    let provider = build_tracer_provider(
        config,
        &factories_with(BTreeMap::new()),
        &no_request_scope(),
        options,
    )
    .expect("the provider builds");
    format!("{provider:?}")
}

fn owned_spec(owner: ExporterOwner) -> ExporterSpec {
    ExporterSpec {
        owner: Some(owner),
        ..spec("console")
    }
}

#[test]
fn the_baggage_processor_precedes_every_exporter_processor() {
    let text = assembled(
        &settings(vec![spec("console"), spec("console")]),
        AssemblyOptions::default(),
    );
    let baggage = text
        .find("BaggageSpanProcessor")
        .expect("a baggage processor");
    let first_exporter = text
        .find("SimpleSpanProcessor")
        .expect("an exporter processor");
    assert!(baggage < first_exporter);
    assert_eq!(text.matches("BaggageSpanProcessor").count(), 1);
    assert_eq!(text.matches("SimpleSpanProcessor").count(), 2);
}

#[test]
fn the_default_baggage_processor_carries_the_promoted_keys_and_an_override_replaces_it() {
    let mut config = settings(vec![spec("console")]);
    config.baggage_promoted_keys = vec!["promoted.team".to_owned()];
    let default = assembled(&config, AssemblyOptions::default());
    assert!(default.contains("promoted.team"));
    let overridden = assembled(
        &config,
        AssemblyOptions {
            exporter: None,
            baggage_processor: Some(BaggageSpanProcessor::new(
                vec!["custom.key".to_owned()],
                None,
            )),
            use_simple_processor: None,
            tenant_overrides: false,
        },
    );
    assert!(overridden.contains("custom.key"));
    assert!(!overridden.contains("promoted.team"));
}

#[rstest]
#[case::required_without_headers(true, None, 0)]
#[case::required_with_headers(true, Some("x-api-key=secret"), 1)]
#[case::required_with_empty_headers(true, Some(""), 1)]
#[case::optional_without_headers(false, None, 1)]
fn a_headers_required_exporter_without_headers_is_skipped(
    #[case] requires_headers: bool,
    #[case] headers: Option<&str>,
    #[case] attached: usize,
) {
    let exporter = ExporterSpec {
        requires_headers,
        headers: headers.map(str::to_owned),
        ..spec("console")
    };
    let text = assembled(&settings(vec![exporter]), AssemblyOptions::default());
    assert_eq!(text.matches("SimpleSpanProcessor").count(), attached);
}

#[test]
fn a_skipped_exporter_does_not_stop_the_ones_after_it() {
    let skipped = ExporterSpec {
        requires_headers: true,
        ..spec("console")
    };
    let text = assembled(
        &settings(vec![skipped, spec("console")]),
        AssemblyOptions::default(),
    );
    assert_eq!(text.matches("SimpleSpanProcessor").count(), 1);
}

#[rstest]
#[case::default_for_console(None, true)]
#[case::batch_override(Some(false), false)]
fn an_explicit_exporter_replaces_the_configured_ones(
    #[case] use_simple_processor: Option<bool>,
    #[case] simple: bool,
) {
    let config = settings(vec![spec("console"), spec("console")]);
    let text = assembled(
        &config,
        AssemblyOptions {
            exporter: Some(SpanSink::Console(
                opentelemetry_stdout::SpanExporter::default(),
            )),
            baggage_processor: None,
            use_simple_processor,
            tenant_overrides: false,
        },
    );
    assert_eq!(text.matches("BaggageSpanProcessor").count(), 1);
    assert_eq!(
        text.matches("SimpleSpanProcessor").count(),
        usize::from(simple)
    );
    assert_eq!(
        text.matches("BatchSpanProcessor").count(),
        usize::from(!simple)
    );
}

#[rstest]
#[case::both_unset(None, None, true)]
#[case::spec_batch_beats_option(Some(false), Some(true), false)]
#[case::spec_simple_beats_option(Some(true), Some(false), true)]
#[case::option_batch_fills_the_gap(None, Some(false), false)]
#[case::option_simple_fills_the_gap(None, Some(true), true)]
fn the_spec_flag_wins_over_the_option_for_the_processor_kind(
    #[case] spec_flag: Option<bool>,
    #[case] option_flag: Option<bool>,
    #[case] simple: bool,
) {
    let exporter = ExporterSpec {
        use_simple_processor: spec_flag,
        ..spec("console")
    };
    let text = assembled(
        &settings(vec![exporter]),
        AssemblyOptions {
            exporter: None,
            baggage_processor: None,
            use_simple_processor: option_flag,
            tenant_overrides: false,
        },
    );
    assert_eq!(
        text.matches("SimpleSpanProcessor").count(),
        usize::from(simple)
    );
    assert_eq!(
        text.matches("BatchSpanProcessor").count(),
        usize::from(!simple)
    );
}

#[rstest]
#[case::tenant_overrides_on(true, 1)]
#[case::tenant_overrides_off(false, 0)]
fn an_owned_exporter_is_filtered_only_when_tenant_overrides_are_on(
    #[case] tenant_overrides: bool,
    #[case] filters: usize,
) {
    let text = assembled(
        &settings(vec![owned_spec(ExporterOwner::ArizeAx)]),
        AssemblyOptions {
            exporter: None,
            baggage_processor: None,
            use_simple_processor: None,
            tenant_overrides,
        },
    );
    assert_eq!(text.matches("OverriddenBackendFilter").count(), filters);
    assert_eq!(
        text.matches("SimpleSpanProcessor").count(),
        usize::from(filters == 0)
    );
}

#[test]
fn an_exporter_without_an_owner_is_never_filtered_by_tenant_overrides() {
    let text = assembled(
        &settings(vec![spec("console")]),
        AssemblyOptions {
            exporter: None,
            baggage_processor: None,
            use_simple_processor: None,
            tenant_overrides: true,
        },
    );
    assert_eq!(text.matches("OverriddenBackendFilter").count(), 0);
    assert_eq!(text.matches("SimpleSpanProcessor").count(), 1);
}

#[rstest]
#[case::arize(ExporterOwner::ArizeAx, "arize")]
#[case::phoenix(ExporterOwner::ArizePhoenix, "arize_phoenix")]
#[case::langfuse(ExporterOwner::LangfuseOtel, "langfuse_otel")]
#[case::weave(ExporterOwner::WeaveOtel, "weave_otel")]
#[case::levo(ExporterOwner::Levo, "levo")]
#[case::agentops(ExporterOwner::Agentops, "agentops")]
#[case::newrelic(ExporterOwner::Newrelic, "newrelic")]
#[case::signoz(ExporterOwner::Signoz, "signoz")]
fn a_filtered_exporter_carries_its_owner_callback_name(
    #[case] owner: ExporterOwner,
    #[case] name: &str,
) {
    assert_eq!(owner_name(owner), name);
    let text = assembled(
        &settings(vec![owned_spec(owner)]),
        AssemblyOptions {
            exporter: None,
            baggage_processor: None,
            use_simple_processor: None,
            tenant_overrides: true,
        },
    );
    assert!(text.contains(&format!("owner: Some(\"{name}\")")));
}

#[rstest]
#[case::langfuse_llm_only(
    ExporterOwner::LangfuseOtel,
    SpanScope::LlmOnly,
    false,
    Some("scope: LlmOnly")
)]
#[case::langfuse_full(ExporterOwner::LangfuseOtel, SpanScope::Full, false, None)]
#[case::other_owner_keeps_the_full_tree(ExporterOwner::ArizeAx, SpanScope::LlmOnly, false, None)]
#[case::other_owner_filtered_for_tenants_only(
    ExporterOwner::ArizeAx,
    SpanScope::LlmOnly,
    true,
    Some("scope: Full")
)]
fn the_langfuse_span_scope_narrows_only_the_langfuse_exporter(
    #[case] owner: ExporterOwner,
    #[case] langfuse_span_scope: SpanScope,
    #[case] tenant_overrides: bool,
    #[case] filtered_with: Option<&str>,
) {
    let mut config = settings(vec![owned_spec(owner)]);
    config.langfuse_span_scope = langfuse_span_scope;
    let text = assembled(
        &config,
        AssemblyOptions {
            exporter: None,
            baggage_processor: None,
            use_simple_processor: None,
            tenant_overrides,
        },
    );
    assert_eq!(
        text.contains("OverriddenBackendFilter"),
        filtered_with.is_some()
    );
    if let Some(scope) = filtered_with {
        assert!(text.contains(scope));
    }
}

#[test]
fn an_unbuildable_exporter_spec_fails_the_assembly() {
    let error = build_tracer_provider(
        &settings(vec![spec("no_such_exporter")]),
        &factories_with(BTreeMap::new()),
        &no_request_scope(),
        AssemblyOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(error, ExporterError::UnknownKind(kind) if kind == "no_such_exporter"));
}

#[test]
fn a_console_exporter_has_no_sink_identity() {
    let console = ExporterSpec {
        endpoint: Some(COLLECTOR.to_owned()),
        ..spec("console")
    };
    assert_eq!(
        wire_sink_key(&console, &factories_with(BTreeMap::new())),
        None
    );
}

#[test]
fn an_otlp_exporter_names_its_traces_endpoint_and_credentials() {
    let otlp = ExporterSpec {
        endpoint: Some(COLLECTOR.to_owned()),
        headers: Some("x-api-key=secret".to_owned()),
        ..spec("otlp_http")
    };
    let key = wire_sink_key(&otlp, &factories_with(BTreeMap::new())).expect("a sink key");
    assert_eq!(key.0, "http://collector:4318/v1/traces");
    assert_eq!(key.1, vec![("x_api_key".to_owned(), "secret".to_owned())]);
}

fn capture_factory(exporter: &InMemorySpanExporter) -> BTreeMap<String, ExporterFactory> {
    let shared = exporter.clone();
    let factory: ExporterFactory = Arc::new(move |_spec| Ok(SpanSink::InMemory(shared.clone())));
    BTreeMap::from([("capture".to_owned(), factory)])
}

fn export_llm_call(request: RequestOtelScope) -> SpanData {
    let exporter = InMemorySpanExporter::default();
    let langfuse = ExporterSpec {
        endpoint: Some(COLLECTOR.to_owned()),
        headers: Some("x-api-key=secret".to_owned()),
        owner: Some(ExporterOwner::LangfuseOtel),
        ..spec("capture")
    };
    let mut config = settings(vec![langfuse]);
    config.langfuse_span_scope = SpanScope::LlmOnly;
    let scope_source: Arc<dyn RequestScopeSource> = Arc::new(FixedScope(request));
    let provider: SdkTracerProvider = build_tracer_provider(
        &config,
        &factories_with(capture_factory(&exporter)),
        &scope_source,
        AssemblyOptions::default(),
    )
    .expect("the provider builds");
    let tracer = provider.tracer("assembly-test");
    let mut span = tracer
        .span_builder("llm call")
        .with_attributes(vec![KeyValue::new(gen_ai::OPERATION_NAME, "chat")])
        .start(&tracer);
    span.end();
    let finished = exporter.get_finished_spans().expect("finished spans");
    finished.into_iter().next().expect("one exported span")
}

fn destination_at_the_langfuse_account(scope: SpanScope) -> RequestOtelScope {
    let destination = OtelDestination::from(DestinationFields {
        endpoint: COLLECTOR.to_owned(),
        headers: BTreeMap::from([("x-api-key".to_owned(), "secret".to_owned())]),
        resource_attributes: BTreeMap::new(),
        callback_name: None,
        protocol: None,
        span_scope: scope,
    });
    RequestOtelScope {
        destinations: vec![destination],
        additive: true,
    }
}

#[test]
fn the_sink_identity_lets_a_request_destination_widen_the_langfuse_scope() {
    let narrowed = export_llm_call(RequestOtelScope::default());
    assert!(
        narrowed
            .attributes
            .iter()
            .any(|pair| pair.key == Key::new(LANGFUSE_TRACE_NAME))
    );
    let widened = export_llm_call(destination_at_the_langfuse_account(SpanScope::Full));
    assert!(
        widened
            .attributes
            .iter()
            .all(|pair| pair.key != Key::new(LANGFUSE_TRACE_NAME))
    );
}

#[rstest]
#[case::with_environment(Some("prod"), Some("prod"))]
#[case::without_environment(None, None)]
#[case::empty_environment(Some(""), None)]
fn the_resource_names_the_service_and_an_environment_only_when_set(
    #[case] environment: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let mut config = settings(Vec::new());
    config.deployment_environment = environment.map(str::to_owned);
    let resource = provider_resource(&config);
    assert_eq!(
        resource
            .get(&Key::new("service.name"))
            .map(|value| value.to_string()),
        Some("betterlite".to_owned())
    );
    assert_eq!(
        resource
            .get(&Key::new("deployment.environment"))
            .map(|value| value.to_string()),
        expected.map(str::to_owned)
    );
}

#[test]
fn configured_resource_attributes_are_added_and_win_over_the_service_name() {
    let mut config = settings(Vec::new());
    config.resource_attributes = BTreeMap::from([
        ("team".to_owned(), "platform".to_owned()),
        ("service.name".to_owned(), "renamed".to_owned()),
    ]);
    let resource = provider_resource(&config);
    assert_eq!(
        resource
            .get(&Key::new("team"))
            .map(|value| value.to_string()),
        Some("platform".to_owned())
    );
    assert_eq!(
        resource
            .get(&Key::new("service.name"))
            .map(|value| value.to_string()),
        Some("renamed".to_owned())
    );
}
