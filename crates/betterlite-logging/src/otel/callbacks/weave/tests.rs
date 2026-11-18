#![cfg(test)]

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use opentelemetry::Context;
use opentelemetry::propagation::TextMapPropagator as _;
use opentelemetry::trace::{
    Span as _, SpanContext, SpanId, Status, TraceContextExt as _, Tracer as _,
};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SpanData};

use super::{CALLBACK_NAME, WeaveDynamicOtel, weave_lifecycle};
use crate::otel::legacy_exporters::tracer_cache::MAX_DYNAMIC_TRACER_PROVIDERS;
use crate::otel::legacy_lifecycle::lifecycle::{
    LegacyLifecycle, LegacyLifecycleOptions, LifecycleCall, LifecycleEvent,
};
use crate::otel::legacy_lifecycle::tracer::{DynamicOtel as _, TracerChoice};
use crate::otel::legacy_policy::capture::LoggingFlags;
use crate::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig};
use crate::otel::legacy_policy::span_time::SpanTime;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::exporters::HttpClients;
use crate::otel::v2_callback::live_span::{LiveSpan, RootSpans};
use crate::otel::vocabulary::span_names::PROXY_REQUEST_SPAN_NAME;
use crate::otel_provider_assembly::legacy_otel_init::{
    InjectedProviders, LegacyOtel, LegacyOtelEnvironment, LegacyOtelInit, MetadataKeyOverrides,
    init_legacy_otel,
};
use crate::otel_provider_assembly::opentelemetry_legacy_provider::GlobalProviders;
use crate::otel_provider_assembly::proxy_registry::ProxyRegistry;

const START: f64 = 1_704_067_200.0;
const END: f64 = 1_704_067_201.5;

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

fn in_memory_config() -> OpenTelemetryConfig {
    OpenTelemetryConfig {
        exporter: Exporter::InMemory,
        ..OpenTelemetryConfig::default()
    }
}

struct Rig {
    env: BTreeMap<String, String>,
    globals: GlobalProviders,
    proxy: ProxyRegistry,
    exporter: InMemorySpanExporter,
    roots: Arc<RootSpans>,
}

impl Rig {
    fn new() -> Self {
        Self {
            env: BTreeMap::from([("OTEL_EXPORTER".to_owned(), "in_memory".to_owned())]),
            globals: GlobalProviders::default(),
            proxy: ProxyRegistry::default(),
            exporter: InMemorySpanExporter::default(),
            roots: Arc::new(RootSpans::default()),
        }
    }

    fn weave(&self) -> LegacyLifecycle {
        weave_lifecycle(
            self.init(in_memory_config(), Some(CALLBACK_NAME)),
            LegacyLifecycleOptions {
                live: Arc::clone(&self.roots),
                ..LegacyLifecycleOptions::default()
            },
        )
    }

    fn base(&self) -> LegacyLifecycle {
        LegacyLifecycle::from_otel(
            self.init(in_memory_config(), None),
            LegacyLifecycleOptions {
                live: Arc::clone(&self.roots),
                ..LegacyLifecycleOptions::default()
            },
        )
    }

    fn init(&self, config: OpenTelemetryConfig, callback_name: Option<&str>) -> LegacyOtel {
        init_legacy_otel(
            LegacyOtelInit {
                config: Some(config),
                callback_name: callback_name.map(str::to_owned),
                overrides: MetadataKeyOverrides::default(),
                injected: InjectedProviders::default(),
                max_dynamic_tracer_providers: MAX_DYNAMIC_TRACER_PROVIDERS,
            },
            &LegacyOtelEnvironment {
                env: &self.env,
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

    fn parent(&self, lifecycle: &LegacyLifecycle, name: &str) -> LiveSpan {
        let builder = lifecycle.tracer.span_builder(name.to_owned());
        let live = LiveSpan::from(
            lifecycle
                .tracer
                .build_with_context(builder, &Context::new()),
        );
        self.roots.register(&live);
        live
    }
}

fn names(spans: &[SpanData]) -> Vec<String> {
    spans.iter().map(|span| span.name.to_string()).collect()
}

fn attach(context: &SpanContext) -> Context {
    Context::new().with_remote_span_context(context.clone())
}

fn kwargs_fixture(json: &str) -> MetadataMap {
    serde_json::from_str(json).expect("kwargs fixture")
}

fn simple_kwargs(call_id: &str) -> MetadataMap {
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
            message_logging: true,
        },
    }
}

fn carry_in(kwargs: &mut MetadataMap, bucket: &str, span: &LiveSpan) {
    let mut carrier = HashMap::new();
    TraceContextPropagator::new().inject_context(
        &Context::new().with_remote_span_context(span.context.clone()),
        &mut carrier,
    );
    let traceparent = carrier
        .remove("traceparent")
        .expect("the propagator writes traceparent");
    let Some(OtelModelValue::Object(params)) = kwargs.get_mut("litellm_params") else {
        panic!("litellm_params must be an object");
    };
    let fields = params
        .entry(bucket.to_owned())
        .or_insert_with(|| OtelModelValue::Object(MetadataMap::default()));
    let OtelModelValue::Object(metadata) = fields else {
        panic!("{bucket} must be an object");
    };
    metadata.insert(
        "litellm_parent_otel_span".to_owned(),
        OtelModelValue::String(traceparent),
    );
}

fn dynamic(pairs: &[(&str, &str)]) -> MetadataMap {
    pairs
        .iter()
        .map(|(name, value)| {
            (
                (*name).to_owned(),
                OtelModelValue::String((*value).to_owned()),
            )
        })
        .collect()
}

#[test]
fn weave_start_primary_span_creates_a_child_where_the_base_reuses_the_parent() {
    let rig = Rig::new();
    let lifecycle = rig.weave();
    let parent = rig.parent(&lifecycle, "wrapper");
    let kwargs = simple_kwargs("call-weave-child");
    let guard = attach(&parent.context).attach();
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("the weave success lifecycle");
    drop(guard);
    let spans = rig.spans();
    assert_eq!(names(&spans), vec!["litellm_request".to_owned()]);
    assert_eq!(spans[0].parent_span_id, parent.context.span_id());
    assert!(matches!(spans[0].status, Status::Ok));
    assert!(
        parent.lock().is_recording(),
        "the wrapper stays user-managed"
    );

    let base_rig = Rig::new();
    let base = base_rig.base();
    let base_parent = base_rig.parent(&base, "wrapper");
    let base_kwargs = simple_kwargs("call-base-parent");
    let guard = attach(&base_parent.context).attach();
    base.handle(&call(&base_kwargs), LifecycleEvent::Success)
        .expect("the base success lifecycle");
    drop(guard);
    assert_eq!(
        names(&base_rig.spans()),
        vec!["raw_gen_ai_request".to_owned()]
    );
}

#[test]
fn weave_maybe_log_raw_request_adds_no_child_span() {
    let rig = Rig::new();
    let lifecycle = rig.weave();
    let kwargs = simple_kwargs("call-raw");
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("the weave success lifecycle");
    let spans = rig.spans();
    assert_eq!(names(&spans), vec!["litellm_request".to_owned()]);
    assert!(matches!(spans[0].status, Status::Ok));
}

#[test]
fn weave_handle_success_does_not_end_an_externally_managed_parent_span() {
    let rig = Rig::new();
    let lifecycle = rig.weave();
    let parent = rig.parent(&lifecycle, "wrapper");
    let kwargs = simple_kwargs("call-parent");
    let guard = attach(&parent.context).attach();
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("the weave success lifecycle");
    drop(guard);
    assert!(
        parent.lock().is_recording(),
        "the wrapper span is never ended"
    );
    assert!(
        rig.spans()
            .iter()
            .all(|span| span.span_context.span_id() != parent.context.span_id())
    );
}

// docs/deviations/B3863.yaml: the Weave override skips the base success tail, which leaked this span
#[test]
fn weave_handle_success_closes_the_proxy_request_span_litellm_leaks() {
    let rig = Rig::new();
    let lifecycle = rig.weave();
    let proxy = rig.parent(&lifecycle, PROXY_REQUEST_SPAN_NAME);
    let mut kwargs = simple_kwargs("call-proxy");
    carry_in(&mut kwargs, "metadata", &proxy);
    let guard = attach(&proxy.context).attach();
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("the weave success lifecycle");
    drop(guard);
    assert!(!proxy.lock().is_recording(), "the proxy span is closed");
    let spans = rig.spans();
    assert!(
        spans
            .iter()
            .any(|span| span.span_context.span_id() == proxy.context.span_id())
    );
    assert_eq!(
        spans
            .iter()
            .filter(|span| span.name == "litellm_request")
            .count(),
        1
    );
}

fn with_guardrail(kwargs: &mut MetadataMap) {
    let Some(OtelModelValue::Object(payload)) = kwargs.get_mut("standard_logging_object") else {
        panic!("standard_logging_object must be an object");
    };
    payload.insert(
        "guardrail_information".to_owned(),
        OtelModelValue::Array(vec![
            serde_json::from_str(
                r#"{
                    "guardrail_name": "pii",
                    "guardrail_mode": "pre_call",
                    "guardrail_status": "success",
                    "start_time": 1704067200.5,
                    "end_time": 1704067200.75
                }"#,
            )
            .expect("guardrail entry"),
        ]),
    );
}

// docs/deviations/B3863.yaml: the Weave override parents the guardrail to ctx, the shared lifecycle to the new span
#[test]
fn weave_parents_the_guardrail_span_to_the_litellm_request_span() {
    let rig = Rig::new();
    let lifecycle = rig.weave();
    let mut kwargs = simple_kwargs("call-guardrail");
    with_guardrail(&mut kwargs);
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("the weave success lifecycle");
    let spans = rig.spans();
    let primary = spans
        .iter()
        .find(|span| span.name == "litellm_request")
        .expect("the primary span");
    let guardrail = spans
        .iter()
        .find(|span| span.name == "guardrail")
        .expect("the guardrail span");
    assert_eq!(guardrail.parent_span_id, primary.span_context.span_id());
}

// docs/deviations/B3863.yaml: the Weave override has no _emit_once, the shared lifecycle keeps it
#[test]
fn weave_suppresses_a_duplicate_success_for_one_request() {
    let rig = Rig::new();
    let lifecycle = rig.weave();
    let kwargs = simple_kwargs("call-duplicate");
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("the first success");
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("the repeated success");
    assert_eq!(names(&rig.spans()), vec!["litellm_request".to_owned()]);
}

// docs/deviations/B3863.yaml: the Weave override never checks ignore_context_propagation
#[test]
fn weave_honors_ignore_context_propagation() {
    let rig = Rig::new();
    let lifecycle = weave_lifecycle(
        rig.init(
            OpenTelemetryConfig {
                ignore_context_propagation: true,
                ..in_memory_config()
            },
            Some(CALLBACK_NAME),
        ),
        LegacyLifecycleOptions {
            live: Arc::clone(&rig.roots),
            ..LegacyLifecycleOptions::default()
        },
    );
    let parent = rig.parent(&lifecycle, "wrapper");
    let kwargs = simple_kwargs("call-ignore");
    let guard = attach(&parent.context).attach();
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("the weave success lifecycle");
    drop(guard);
    let spans = rig.spans();
    assert_eq!(names(&spans), vec!["litellm_request".to_owned()]);
    assert_eq!(spans[0].parent_span_id, SpanId::INVALID);
}

#[test]
fn weave_dynamic_headers_carry_the_authorization_and_project_id() {
    let headers = WeaveDynamicOtel
        .construct_dynamic_otel_headers(&dynamic(&[
            ("wandb_api_key", "key-1"),
            ("weave_project_id", "entity/project"),
        ]))
        .expect("the headers are built");
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Basic YXBpOmtleS0x")
    );
    assert_eq!(
        headers.get("project_id").map(String::as_str),
        Some("entity/project")
    );
}

#[test]
fn weave_dynamic_headers_need_an_api_key_for_the_authorization() {
    let headers = WeaveDynamicOtel
        .construct_dynamic_otel_headers(&dynamic(&[("wandb_api_key", "key-2")]))
        .expect("the headers are built");
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Basic YXBpOmtleS0y")
    );
    assert!(!headers.contains_key("project_id"));
}

#[test]
fn weave_dynamic_headers_carry_the_project_id_without_an_api_key() {
    let headers = WeaveDynamicOtel
        .construct_dynamic_otel_headers(&dynamic(&[
            ("wandb_api_key", ""),
            ("weave_project_id", "entity/project"),
        ]))
        .expect("the headers are built");
    assert!(!headers.contains_key("Authorization"));
    assert_eq!(
        headers.get("project_id").map(String::as_str),
        Some("entity/project")
    );
}

#[test]
fn weave_dynamic_headers_are_absent_without_a_weave_credential() {
    assert!(
        WeaveDynamicOtel
            .construct_dynamic_otel_headers(&MetadataMap::new())
            .is_none()
    );
    assert!(
        WeaveDynamicOtel
            .construct_dynamic_otel_headers(&dynamic(&[
                ("wandb_api_key", ""),
                ("weave_project_id", ""),
            ]))
            .is_none()
    );
}

#[test]
fn weave_dynamic_headers_ignore_a_non_text_value() {
    let params = MetadataMap::from([("wandb_api_key".to_owned(), OtelModelValue::Bool(true))]);
    assert!(
        WeaveDynamicOtel
            .construct_dynamic_otel_headers(&params)
            .is_none()
    );
}

#[test]
fn weave_reads_the_dynamic_headers_from_the_standard_callback_dynamic_params() {
    let rig = Rig::new();
    let lifecycle = rig.weave();
    let mut kwargs = simple_kwargs("call-dynamic");
    kwargs.insert(
        "standard_callback_dynamic_params".to_owned(),
        OtelModelValue::Object(dynamic(&[
            ("wandb_api_key", "key-2"),
            ("weave_project_id", "entity/project"),
        ])),
    );
    let TracerChoice::DynamicHeaders(headers) =
        lifecycle.tracer_choice(&kwargs).expect("the tracer choice")
    else {
        panic!("the weave callback builds dynamic headers");
    };
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Basic YXBpOmtleS0y")
    );
    assert_eq!(
        headers.get("project_id").map(String::as_str),
        Some("entity/project")
    );
}

#[test]
fn weave_answers_to_the_weave_otel_callback_name() {
    let rig = Rig::new();
    let lifecycle = rig.weave();
    assert_eq!(lifecycle.callback_name.as_deref(), Some(CALLBACK_NAME));
    assert_eq!(CALLBACK_NAME, "weave_otel");
}
