#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use opentelemetry::Key;
use opentelemetry::KeyValue;
use opentelemetry::trace::{SpanId, SpanKind, Status};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};

use super::{
    CALLBACK_NAME, DEFAULT_PROJECT, HEALTHY_MESSAGE, MISSING_API_KEY_MESSAGE,
    PROXY_PARENT_SPAN_NAME, PhoenixHealth, handle_trace, health_check, is_proxy_request,
    otel_config, phoenix_lifecycle, project_attributes, resolve_project_name,
};
use crate::otel::integrations::phoenix_settings::PhoenixSettingsError;
use crate::otel::legacy_exporters::tracer_cache::DynamicTracerCache;
use crate::otel::legacy_exporters::tracer_cache::project::{
    MAX_PROJECT_PROVIDERS, project_resource, tracer_for,
};
use crate::otel::legacy_exporters::tracer_cache::tests::project_names;
use crate::otel::legacy_lifecycle::lifecycle::{
    DEFAULT_HANDLER_CLASS_NAME, LegacyLifecycle, LegacyLifecycleOptions, LifecycleCall,
    LifecycleEvent,
};
use crate::otel::legacy_lifecycle::tracer::BaseDynamicOtel;
use crate::otel::legacy_policy::capture::LoggingFlags;
use crate::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig};
use crate::otel::legacy_policy::error_recording::RecordedException;
use crate::otel::legacy_policy::span_time::SpanTime;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::events::SystemClock;
use crate::otel::plumbing::exporters::HttpClients;
use crate::otel::v2_callback::live_span::RootSpans;
use crate::otel_provider_assembly::legacy_otel_init::{
    InjectedProviders, LegacyOtel, LegacyOtelEnvironment, LegacyOtelInit, MetadataKeyOverrides,
    init_legacy_otel,
};
use crate::otel_provider_assembly::opentelemetry_legacy_provider::GlobalProviders;
use crate::otel_provider_assembly::proxy_registry::{
    OtelRegistration, ProxyRegistry, ServiceCallback, register_on_proxy,
};

const START: f64 = 1_704_067_200.0;
const END: f64 = 1_704_067_201.5;
const TRACEPARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

#[derive(Debug)]
struct NoServer;

#[async_trait]
impl HttpClient for NoServer {
    async fn send_bytes(&self, _request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        Err(HttpError::from("no HTTP client is wired into this suite"))
    }
}

fn shared_clients() -> HttpClients {
    let shared: Arc<dyn HttpClient> = Arc::new(NoServer);
    HttpClients {
        traces: Arc::clone(&shared),
        metrics: Arc::clone(&shared),
        logs: shared,
    }
}

fn in_memory_env() -> BTreeMap<String, String> {
    BTreeMap::from([("OTEL_EXPORTER".to_owned(), "in_memory".to_owned())])
}

struct Rig {
    globals: GlobalProviders,
    proxy: ProxyRegistry,
    exporter: InMemorySpanExporter,
    roots: Arc<RootSpans>,
}

impl Rig {
    fn new() -> Self {
        Self {
            globals: GlobalProviders::default(),
            proxy: ProxyRegistry::default(),
            exporter: InMemorySpanExporter::default(),
            roots: Arc::new(RootSpans::default()),
        }
    }

    fn phoenix(&self) -> LegacyLifecycle {
        phoenix_lifecycle(self.init(), self.options(), &BTreeMap::new())
    }

    fn options(&self) -> LegacyLifecycleOptions {
        LegacyLifecycleOptions {
            dynamic: Arc::new(BaseDynamicOtel),
            live: Arc::clone(&self.roots),
            metrics: None,
            events: None,
            failure_counters: Vec::new(),
            handler_class_name: DEFAULT_HANDLER_CLASS_NAME.to_owned(),
            use_request_span: false,
            clock: Box::new(SystemClock),
        }
    }

    fn init(&self) -> LegacyOtel {
        self.init_with(OpenTelemetryConfig::from_env(&in_memory_env()))
    }

    fn init_with(&self, config: OpenTelemetryConfig) -> LegacyOtel {
        let env = in_memory_env();
        init_legacy_otel(
            LegacyOtelInit {
                config: Some(config),
                callback_name: Some(CALLBACK_NAME.to_owned()),
                overrides: MetadataKeyOverrides::default(),
                injected: InjectedProviders::default(),
                max_dynamic_tracer_providers: MAX_PROJECT_PROVIDERS,
            },
            &LegacyOtelEnvironment {
                env: &env,
                clients: shared_clients(),
                shared_exporter: self.exporter.clone(),
                globals: &self.globals,
                proxy: Some(&self.proxy),
            },
        )
        .expect("legacy otel init")
    }

    fn spans(&self) -> Vec<SpanData> {
        self.exporter.get_finished_spans().expect("finished spans")
    }
}

fn kwargs_fixture(json: &str) -> MetadataMap {
    serde_json::from_str(json).expect("kwargs fixture")
}

fn base_kwargs(call_id: &str) -> MetadataMap {
    kwargs_fixture(&format!(
        r#"{{
            "model": "gpt-4o",
            "messages": [{{"role": "user", "content": "hello"}}],
            "call_type": "completion",
            "optional_params": {{}},
            "litellm_params": {{"custom_llm_provider": "openai", "metadata": {{"litellm_call_id": "{call_id}"}}}},
            "standard_logging_object": {{
                "id": "chatcmpl-1",
                "call_type": "completion",
                "status": "success",
                "metadata": {{"litellm_call_id": "{call_id}"}},
                "hidden_params": {{}}
            }}
        }}"#
    ))
}

fn call(kwargs: &MetadataMap) -> LifecycleCall<'_> {
    LifecycleCall {
        kwargs,
        response: None,
        start_time: SpanTime::Seconds(START),
        end_time: SpanTime::Seconds(END),
        exception: None,
        flags: LoggingFlags {
            turn_off_message_logging: false,
            message_logging: false,
        },
    }
}

fn metadata_object<'map>(kwargs: &'map mut MetadataMap, bucket: &str) -> &'map mut MetadataMap {
    let Some(OtelModelValue::Object(fields)) = kwargs.get_mut(bucket) else {
        panic!("{bucket} must be an object");
    };
    let metadata = fields
        .entry("metadata".to_owned())
        .or_insert_with(|| OtelModelValue::Object(MetadataMap::default()));
    let OtelModelValue::Object(entries) = metadata else {
        panic!("{bucket}.metadata must be an object");
    };
    entries
}

fn with_project(kwargs: &mut MetadataMap, bucket: &str, key: &str, project: &str) {
    metadata_object(kwargs, bucket)
        .insert(key.to_owned(), OtelModelValue::String(project.to_owned()));
}

fn with_auth_project(kwargs: &mut MetadataMap, key: &str, project: &str) {
    let metadata = metadata_object(kwargs, "litellm_params");
    let auth = metadata
        .entry("user_api_key_auth_metadata".to_owned())
        .or_insert_with(|| OtelModelValue::Object(MetadataMap::default()));
    let OtelModelValue::Object(entries) = auth else {
        panic!("the auth metadata must be an object");
    };
    entries.insert(key.to_owned(), OtelModelValue::String(project.to_owned()));
}

fn with_proxy(kwargs: &mut MetadataMap) {
    let Some(OtelModelValue::Object(params)) = kwargs.get_mut("litellm_params") else {
        panic!("litellm_params must be an object");
    };
    params.insert(
        "proxy_server_request".to_owned(),
        serde_json::from_str(r#"{"headers": {}}"#).expect("the proxy request"),
    );
}

fn with_traceparent(kwargs: &mut MetadataMap, traceparent: &str) {
    let Some(OtelModelValue::Object(params)) = kwargs.get_mut("litellm_params") else {
        panic!("litellm_params must be an object");
    };
    let request = serde_json::from_str::<OtelModelValue>(&format!(
        r#"{{"headers": {{"traceparent": "{traceparent}"}}}}"#
    ))
    .expect("the proxy request");
    params.insert("proxy_server_request".to_owned(), request);
}

fn names(spans: &[SpanData]) -> Vec<String> {
    spans.iter().map(|span| span.name.to_string()).collect()
}

fn named<'spans>(spans: &'spans [SpanData], name: &str) -> &'spans SpanData {
    spans
        .iter()
        .find(|span| span.name == name)
        .expect("the named span")
}

fn attribute(span: &SpanData, name: &str) -> Option<String> {
    span.attributes
        .iter()
        .find(|pair| pair.key.as_str() == name)
        .map(|pair| pair.value.as_str().into_owned())
}

fn env_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn fill_projects(cache: &DynamicTracerCache, count: usize) -> Vec<String> {
    let mut names = Vec::with_capacity(count);
    for index in 0..count {
        let project = format!("project-{index}");
        tracer_for(cache, &project, &project_attributes(&project)).expect("the project tracer");
        names.push(project);
    }
    names
}

#[test]
fn phoenix_answers_to_the_arize_phoenix_callback_name() {
    let lifecycle = Rig::new().phoenix();
    assert_eq!(lifecycle.callback_name.as_deref(), Some(CALLBACK_NAME));
    assert_eq!(CALLBACK_NAME, "arize_phoenix");
    assert_eq!(PROXY_PARENT_SPAN_NAME, "litellm_proxy_request");
}

// ArizePhoenixLogger._init_otel_logger_on_litellm_proxy is empty, so the primary otel callback keeps the proxy slot
#[test]
fn phoenix_coexists_with_the_primary_otel_registration() {
    let rig = Rig::new();
    register_on_proxy(
        &rig.proxy,
        &OtelRegistration {
            callback_name: Some("otel".to_owned()),
        },
    );
    let _lifecycle = rig.phoenix();
    let slot = rig.proxy.open_telemetry_logger.lock().expect("the slot");
    assert_eq!(
        slot.as_ref()
            .and_then(|entry| entry.callback_name.as_deref()),
        Some("otel")
    );
    drop(slot);
    let callbacks = rig.proxy.service_callbacks.lock().expect("the callbacks");
    assert_eq!(callbacks.len(), 1);
    assert!(matches!(
        callbacks.first(),
        Some(ServiceCallback::OpenTelemetry(entry)) if entry.callback_name.as_deref() == Some("otel")
    ));
}

// ArizePhoenixLogger._init_tracing builds private providers and never takes the process global
#[test]
fn phoenix_keeps_its_tracer_out_of_the_global_slot() {
    let rig = Rig::new();
    let mut config = otel_config(&in_memory_env()).expect("the phoenix config");
    config.exporter = Exporter::InMemory;
    let _lifecycle = phoenix_lifecycle(rig.init_with(config), rig.options(), &BTreeMap::new());
    assert!(
        rig.globals
            .tracer
            .current
            .lock()
            .expect("the global slot")
            .is_none(),
        "phoenix publishes no global tracer provider"
    );
}

#[test]
fn phoenix_routes_the_request_to_its_project_provider() {
    let rig = Rig::new();
    let lifecycle = rig.phoenix();
    let mut kwargs = base_kwargs("call-project");
    with_project(
        &mut kwargs,
        "litellm_params",
        "phoenix_project_name",
        "project-a",
    );
    handle_trace(&lifecycle, &call(&kwargs), LifecycleEvent::Success).expect("the phoenix trace");
    assert_eq!(
        project_names(&lifecycle.tracer_cache),
        vec!["project-a".to_owned()]
    );
    assert_eq!(rig.spans().len(), 1);
}

// an injected provider leaves the project routing on, unlike LiteLLM's injected branch
#[test]
fn phoenix_routes_to_the_project_even_with_an_injected_provider() {
    let rig = Rig::new();
    let env = in_memory_env();
    let otel = init_legacy_otel(
        LegacyOtelInit {
            config: Some(OpenTelemetryConfig::from_env(&env)),
            callback_name: Some(CALLBACK_NAME.to_owned()),
            overrides: MetadataKeyOverrides::default(),
            injected: InjectedProviders {
                tracer: Some(SdkTracerProvider::builder().build()),
                meter: None,
                logger: None,
            },
            max_dynamic_tracer_providers: MAX_PROJECT_PROVIDERS,
        },
        &LegacyOtelEnvironment {
            env: &env,
            clients: shared_clients(),
            shared_exporter: rig.exporter.clone(),
            globals: &rig.globals,
            proxy: Some(&rig.proxy),
        },
    )
    .expect("legacy otel init");
    let lifecycle = phoenix_lifecycle(otel, rig.options(), &BTreeMap::new());
    handle_trace(
        &lifecycle,
        &call(&base_kwargs("call-injected")),
        LifecycleEvent::Success,
    )
    .expect("the phoenix trace");
    assert_eq!(
        project_names(&lifecycle.tracer_cache),
        vec![DEFAULT_PROJECT.to_owned()]
    );
    assert_eq!(rig.spans().len(), 1);
}

#[test]
fn phoenix_reuses_the_provider_of_a_project() {
    let rig = Rig::new();
    let lifecycle = rig.phoenix();
    let first = base_kwargs("call-first");
    let second = base_kwargs("call-second");
    handle_trace(&lifecycle, &call(&first), LifecycleEvent::Success).expect("the first call");
    handle_trace(&lifecycle, &call(&second), LifecycleEvent::Success).expect("the second call");
    assert_eq!(project_names(&lifecycle.tracer_cache).len(), 1);
    assert_eq!(rig.spans().len(), 2);
}

#[test]
fn phoenix_keeps_the_last_sixty_four_project_providers() {
    let rig = Rig::new();
    let lifecycle = rig.phoenix();
    let names = fill_projects(&lifecycle.tracer_cache, 65);
    let cached = project_names(&lifecycle.tracer_cache);
    assert_eq!(cached.len(), MAX_PROJECT_PROVIDERS);
    assert_eq!(
        cached.first().map(String::as_str),
        names.get(1).map(String::as_str)
    );
    assert_eq!(
        cached.last().map(String::as_str),
        names.get(64).map(String::as_str)
    );
    // the evicted provider of an in-memory exporter owns no exporter, so nothing is queued for shutdown
    assert_eq!(lifecycle.tracer_cache.join_shutdowns(), 0);
}

// a project provider with an exporter of its own is flushed and shut down after its eviction
#[test]
fn phoenix_flushes_an_evicted_project_provider() {
    let rig = Rig::new();
    let env = env_map(&[("PHOENIX_COLLECTOR_ENDPOINT", "http://phoenix.test:6006")]);
    let config = otel_config(&env).expect("the phoenix config");
    let lifecycle = phoenix_lifecycle(rig.init_with(config), rig.options(), &BTreeMap::new());
    fill_projects(&lifecycle.tracer_cache, MAX_PROJECT_PROVIDERS + 1);
    assert_eq!(lifecycle.tracer_cache.join_shutdowns(), 1);
}

#[test]
fn phoenix_moves_a_hit_to_the_end_of_the_lru() {
    let rig = Rig::new();
    let lifecycle = rig.phoenix();
    let names = fill_projects(&lifecycle.tracer_cache, 64);
    let touched = names.first().expect("the first project");
    tracer_for(
        &lifecycle.tracer_cache,
        touched,
        &project_attributes(touched),
    )
    .expect("the cached tracer");
    tracer_for(
        &lifecycle.tracer_cache,
        "project-new",
        &project_attributes("project-new"),
    )
    .expect("the new tracer");
    let cached = project_names(&lifecycle.tracer_cache);
    assert!(cached.iter().any(|name| name == touched));
    assert!(cached.iter().all(|name| name != "project-1"));
    assert!(cached.iter().any(|name| name == "project-new"));
}

#[test]
fn phoenix_project_resolution_prefers_the_override() {
    let mut kwargs = base_kwargs("call-override");
    with_project(
        &mut kwargs,
        "litellm_params",
        "phoenix_project_name",
        "named",
    );
    with_project(
        &mut kwargs,
        "litellm_params",
        "phoenix_project_name_override",
        "overridden",
    );
    assert_eq!(
        resolve_project_name(&kwargs, &BTreeMap::new()),
        "overridden"
    );
}

#[test]
fn phoenix_project_resolution_reads_the_metadata_name() {
    let mut kwargs = base_kwargs("call-name");
    with_project(
        &mut kwargs,
        "litellm_params",
        "phoenix_project_name",
        "named",
    );
    assert_eq!(resolve_project_name(&kwargs, &BTreeMap::new()), "named");
}

#[test]
fn phoenix_project_resolution_reads_the_env_name() {
    let kwargs = base_kwargs("call-env");
    let env = env_map(&[("PHOENIX_PROJECT_NAME", "env-project")]);
    assert_eq!(resolve_project_name(&kwargs, &env), "env-project");
}

#[test]
fn phoenix_project_resolution_reads_the_arize_env_name() {
    let kwargs = base_kwargs("call-arize-env");
    let env = env_map(&[("ARIZE_PROJECT_NAME", "arize-project")]);
    assert_eq!(resolve_project_name(&kwargs, &env), "arize-project");
}

#[test]
fn phoenix_project_resolution_defaults_to_default() {
    let mut kwargs = base_kwargs("call-default");
    with_project(&mut kwargs, "litellm_params", "phoenix_project_name", "   ");
    let env = env_map(&[("PHOENIX_PROJECT_NAME", "")]);
    assert_eq!(resolve_project_name(&kwargs, &env), DEFAULT_PROJECT);
}

#[test]
fn phoenix_project_resolution_reads_the_metadata_before_the_env() {
    let mut kwargs = base_kwargs("call-both");
    with_project(
        &mut kwargs,
        "litellm_params",
        "phoenix_project_name",
        "named",
    );
    let env = env_map(&[("PHOENIX_PROJECT_NAME", "env-project")]);
    assert_eq!(resolve_project_name(&kwargs, &env), "named");
}

#[test]
fn phoenix_project_resolution_reads_the_logging_metadata_first() {
    let mut kwargs = base_kwargs("call-logging");
    with_project(
        &mut kwargs,
        "standard_logging_object",
        "phoenix_project_name",
        "logged",
    );
    with_project(
        &mut kwargs,
        "litellm_params",
        "phoenix_project_name",
        "params",
    );
    assert_eq!(resolve_project_name(&kwargs, &BTreeMap::new()), "logged");
}

#[test]
fn phoenix_proxy_mode_ignores_the_client_metadata() {
    let mut kwargs = base_kwargs("call-client");
    with_proxy(&mut kwargs);
    with_project(
        &mut kwargs,
        "litellm_params",
        "phoenix_project_name",
        "client-project",
    );
    assert!(is_proxy_request(&kwargs));
    assert_eq!(
        resolve_project_name(&kwargs, &BTreeMap::new()),
        DEFAULT_PROJECT
    );
}

#[test]
fn phoenix_proxy_mode_reads_the_auth_metadata() {
    let mut kwargs = base_kwargs("call-auth");
    with_proxy(&mut kwargs);
    with_auth_project(&mut kwargs, "phoenix_project_name", "team-project");
    assert_eq!(
        resolve_project_name(&kwargs, &BTreeMap::new()),
        "team-project"
    );
}

#[test]
fn phoenix_proxy_mode_ignores_a_blank_auth_project() {
    let mut kwargs = base_kwargs("call-blank-auth");
    with_proxy(&mut kwargs);
    with_auth_project(&mut kwargs, "phoenix_project_name", "   ");
    let env = env_map(&[("PHOENIX_PROJECT_NAME", "env-project")]);
    assert_eq!(resolve_project_name(&kwargs, &env), "env-project");
}

#[test]
fn phoenix_sdk_mode_starts_a_root_span() {
    let rig = Rig::new();
    let lifecycle = rig.phoenix();
    let kwargs = base_kwargs("call-sdk");
    handle_trace(&lifecycle, &call(&kwargs), LifecycleEvent::Success).expect("the sdk trace");
    let spans = rig.spans();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].parent_span_id, SpanId::INVALID);
    assert_eq!(spans[0].span_kind, SpanKind::Internal);
}

#[test]
fn phoenix_proxy_mode_creates_its_own_parent_span() {
    let rig = Rig::new();
    let lifecycle = rig.phoenix();
    let mut kwargs = base_kwargs("call-proxy");
    with_proxy(&mut kwargs);
    handle_trace(&lifecycle, &call(&kwargs), LifecycleEvent::Success).expect("the proxy trace");
    let spans = rig.spans();
    assert_eq!(spans.len(), 2);
    assert!(names(&spans).contains(&PROXY_PARENT_SPAN_NAME.to_owned()));
    let parent = named(&spans, PROXY_PARENT_SPAN_NAME);
    assert_eq!(parent.span_kind, SpanKind::Server);
    assert_eq!(parent.parent_span_id, SpanId::INVALID);
    let request = spans
        .iter()
        .find(|span| span.name != PROXY_PARENT_SPAN_NAME)
        .expect("the request span");
    assert_eq!(request.parent_span_id, parent.span_context.span_id());
    assert!(matches!(parent.status, Status::Ok));
}

#[test]
fn phoenix_proxy_mode_keeps_the_incoming_traceparent() {
    let rig = Rig::new();
    let lifecycle = rig.phoenix();
    let mut kwargs = base_kwargs("call-traceparent");
    with_traceparent(&mut kwargs, TRACEPARENT);
    handle_trace(&lifecycle, &call(&kwargs), LifecycleEvent::Success).expect("the traced call");
    let spans = rig.spans();
    let parent = named(&spans, PROXY_PARENT_SPAN_NAME);
    assert_eq!(
        parent.parent_span_id,
        SpanId::from_hex("b7ad6b7169203331").expect("the parent id")
    );
}

#[test]
fn phoenix_proxy_mode_closes_the_parent_on_failure() {
    let rig = Rig::new();
    let lifecycle = rig.phoenix();
    let mut kwargs = base_kwargs("call-failure");
    with_proxy(&mut kwargs);
    handle_trace(&lifecycle, &call(&kwargs), LifecycleEvent::Failure).expect("the failed call");
    let spans = rig.spans();
    let parent = named(&spans, PROXY_PARENT_SPAN_NAME);
    assert!(matches!(parent.status, Status::Error { .. }));
}

#[test]
fn phoenix_proxy_mode_records_the_failure_on_the_parent() {
    let rig = Rig::new();
    let lifecycle = rig.phoenix();
    let mut kwargs = base_kwargs("call-parent-error");
    with_proxy(&mut kwargs);
    let exception = RecordedException {
        type_name: "ValueError".to_owned(),
        message: "bad input".to_owned(),
        stacktrace: String::new(),
    };
    let mut failed = call(&kwargs);
    failed.exception = Some(&exception);
    handle_trace(&lifecycle, &failed, LifecycleEvent::Failure).expect("the failed call");
    let spans = rig.spans();
    let parent = named(&spans, PROXY_PARENT_SPAN_NAME);
    assert!(
        parent.events.iter().any(|event| event.name == "exception"),
        "the parent span records the exception"
    );
}

#[test]
fn phoenix_suppresses_a_duplicate_success() {
    let rig = Rig::new();
    let lifecycle = rig.phoenix();
    let kwargs = base_kwargs("call-duplicate");
    handle_trace(&lifecycle, &call(&kwargs), LifecycleEvent::Success).expect("the first success");
    handle_trace(&lifecycle, &call(&kwargs), LifecycleEvent::Success)
        .expect("the repeated success");
    assert_eq!(rig.spans().len(), 1);
}

#[test]
fn phoenix_ignores_a_base_provider_parent() {
    let rig = Rig::new();
    let lifecycle = rig.phoenix();
    let mut kwargs = base_kwargs("call-base-parent");
    with_project(
        &mut kwargs,
        "litellm_params",
        "litellm_parent_otel_span",
        TRACEPARENT,
    );
    handle_trace(&lifecycle, &call(&kwargs), LifecycleEvent::Success).expect("the phoenix trace");
    let spans = rig.spans();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].parent_span_id, SpanId::INVALID);
}

// an exception inside _handle_phoenix_trace leaves LiteLLM's parent unended and lost
#[test]
fn phoenix_closes_the_parent_when_the_lifecycle_errors() {
    let rig = Rig::new();
    let lifecycle = rig.phoenix();
    let mut kwargs = base_kwargs("call-error");
    with_proxy(&mut kwargs);
    {
        let Some(OtelModelValue::Object(payload)) = kwargs.get_mut("standard_logging_object")
        else {
            panic!("standard_logging_object must be an object");
        };
        payload.insert(
            "guardrail_information".to_owned(),
            serde_json::from_str(
                r#"[{"guardrail_name": "pii", "guardrail_mode": "pre_call", "start_time": "not-a-clock"}]"#,
            )
            .expect("guardrail entry"),
        );
    }
    assert!(handle_trace(&lifecycle, &call(&kwargs), LifecycleEvent::Success).is_err());
    assert!(names(&rig.spans()).contains(&PROXY_PARENT_SPAN_NAME.to_owned()));
}

#[test]
fn phoenix_routes_the_guardrail_to_the_project_provider() {
    let rig = Rig::new();
    let lifecycle = rig.phoenix();
    let mut kwargs = base_kwargs("call-guardrail");
    with_project(
        &mut kwargs,
        "litellm_params",
        "phoenix_project_name",
        "guard-project",
    );
    {
        let Some(OtelModelValue::Object(payload)) = kwargs.get_mut("standard_logging_object")
        else {
            panic!("standard_logging_object must be an object");
        };
        payload.insert(
            "guardrail_information".to_owned(),
            serde_json::from_str(
                r#"[{
                    "guardrail_name": "pii",
                    "guardrail_mode": "pre_call",
                    "guardrail_status": "success",
                    "start_time": 1704067200.5,
                    "end_time": 1704067200.75
                }]"#,
            )
            .expect("guardrail entry"),
        );
    }
    handle_trace(&lifecycle, &call(&kwargs), LifecycleEvent::Success).expect("the guardrail call");
    assert_eq!(
        project_names(&lifecycle.tracer_cache),
        vec!["guard-project".to_owned()]
    );
    assert!(names(&rig.spans()).contains(&"guardrail".to_owned()));
}

#[test]
fn phoenix_writes_openinference_attributes() {
    let rig = Rig::new();
    let lifecycle = rig.phoenix();
    let kwargs = base_kwargs("call-attributes");
    handle_trace(&lifecycle, &call(&kwargs), LifecycleEvent::Success).expect("the phoenix trace");
    let span = &rig.spans()[0];
    assert_eq!(
        attribute(span, "openinference.span.kind").as_deref(),
        Some("LLM")
    );
}

#[test]
fn phoenix_project_attributes_override_the_resource() {
    let base = Resource::builder_empty()
        .with_attributes([
            KeyValue::new("service.name", "gateway"),
            KeyValue::new("openinference.project.name", "base"),
        ])
        .build();
    let merged = project_resource(&base, &project_attributes("project-a"));
    let service = merged
        .get(&Key::new("service.name"))
        .map(|value| value.as_str().into_owned());
    let project = merged
        .get(&Key::new("openinference.project.name"))
        .map(|value| value.as_str().into_owned());
    assert_eq!(service.as_deref(), Some("project-a"));
    assert_eq!(project.as_deref(), Some("project-a"));
    let model = merged
        .get(&Key::new("model_id"))
        .map(|value| value.as_str().into_owned());
    assert_eq!(model.as_deref(), Some("project-a"));
}

#[test]
fn phoenix_health_check_accepts_an_api_key() {
    let env = env_map(&[("PHOENIX_API_KEY", "api-key")]);
    assert_eq!(
        health_check(&env).expect("the health answer"),
        PhoenixHealth::Healthy {
            message: HEALTHY_MESSAGE
        }
    );
}

#[test]
fn phoenix_health_check_reports_the_missing_api_key() {
    let env = env_map(&[]);
    assert_eq!(
        health_check(&env).expect("the health answer"),
        PhoenixHealth::Unhealthy {
            error_message: MISSING_API_KEY_MESSAGE.to_owned()
        }
    );
}

#[test]
fn phoenix_health_check_propagates_the_cloud_error() {
    let env = env_map(&[(
        "PHOENIX_COLLECTOR_ENDPOINT",
        "https://app.phoenix.arize.com",
    )]);
    assert!(matches!(
        health_check(&env),
        Err(PhoenixSettingsError::CloudApiKeyMissing)
    ));
}

#[test]
fn phoenix_otel_config_points_at_the_configured_endpoint() {
    let env = env_map(&[
        ("PHOENIX_COLLECTOR_ENDPOINT", "http://phoenix.test:6006"),
        ("PHOENIX_API_KEY", "api-key"),
    ]);
    let config = otel_config(&env).expect("the phoenix config");
    assert_eq!(config.exporter, Exporter::Named("otlp_http".to_owned()));
    assert_eq!(
        config.endpoint.as_deref(),
        Some("http://phoenix.test:6006/v1/traces")
    );
    assert_eq!(
        config.headers.as_deref(),
        Some("Authorization=Bearer api-key")
    );
}

#[test]
fn phoenix_otel_config_propagates_the_cloud_error() {
    let env = env_map(&[(
        "PHOENIX_COLLECTOR_ENDPOINT",
        "https://app.phoenix.arize.com",
    )]);
    assert!(matches!(
        otel_config(&env),
        Err(PhoenixSettingsError::CloudApiKeyMissing)
    ));
}
