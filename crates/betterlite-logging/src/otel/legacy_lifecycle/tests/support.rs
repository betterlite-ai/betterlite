use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use opentelemetry::Context;
use opentelemetry::propagation::TextMapPropagator as _;
use opentelemetry::trace::{SpanContext, TraceContextExt as _, Tracer as _};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SpanData};

use betterlite_logging::exporter_options::failure_counter::CallbackFailureCounter;
use betterlite_logging::otel::legacy_lifecycle::lifecycle::{
    LegacyLifecycle, LegacyLifecycleOptions, LifecycleCall,
};
use betterlite_logging::otel::legacy_lifecycle::tracer::DynamicOtel;
use betterlite_logging::otel::legacy_metrics::events::SemanticEventEmitter;
use betterlite_logging::otel::legacy_metrics::recorder::LegacyMetricRecorder;
use betterlite_logging::otel::legacy_policy::capture::LoggingFlags;
use betterlite_logging::otel::legacy_policy::config::OpenTelemetryConfig;
use betterlite_logging::otel::legacy_policy::span_time::SpanTime;
use betterlite_logging::otel::model::metadata::wire::MetadataMap;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::plumbing::events::{Clock, SystemClock};
use betterlite_logging::otel::plumbing::exporters::HttpClients;
use betterlite_logging::otel::v2_callback::live_span::{LiveSpan, RootSpans};
use betterlite_logging::otel_provider_assembly::legacy_otel_init::{
    LegacyOtelEnvironment, LegacyOtelInit, init_legacy_otel,
};
use betterlite_logging::otel_provider_assembly::opentelemetry_legacy_provider::GlobalProviders;
use betterlite_logging::otel_provider_assembly::proxy_registry::ProxyRegistry;

pub(super) const START: f64 = 1704067200.0;
pub(super) const END: f64 = 1704067201.5;
pub(super) const PROXY: &str = "Received Proxy Server Request";

#[derive(Debug)]
struct NoServer;

#[async_trait::async_trait]
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

pub(super) struct Rig {
    env: BTreeMap<String, String>,
    globals: GlobalProviders,
    proxy: ProxyRegistry,
    exporter: InMemorySpanExporter,
    pub(super) roots: Arc<RootSpans>,
}

impl Rig {
    pub(super) fn new() -> Self {
        Self {
            env: BTreeMap::from([("OTEL_EXPORTER".to_owned(), "in_memory".to_owned())]),
            globals: GlobalProviders::default(),
            proxy: ProxyRegistry::default(),
            exporter: InMemorySpanExporter::default(),
            roots: Arc::new(RootSpans::default()),
        }
    }

    pub(super) fn in_memory(&self, callback_name: Option<&str>) -> LegacyLifecycle {
        self.with_options(in_memory_config(), callback_name, None, false)
    }

    pub(super) fn with_options(
        &self,
        config: OpenTelemetryConfig,
        callback_name: Option<&str>,
        dynamic: Option<Arc<dyn DynamicOtel>>,
        use_request_span: bool,
    ) -> LegacyLifecycle {
        self.with_services(
            config,
            callback_name,
            dynamic,
            use_request_span,
            Box::new(SystemClock),
            None,
            None,
            Vec::new(),
        )
    }

    pub(super) fn with_services(
        &self,
        config: OpenTelemetryConfig,
        callback_name: Option<&str>,
        dynamic: Option<Arc<dyn DynamicOtel>>,
        use_request_span: bool,
        clock: Box<dyn Clock>,
        metrics: Option<LegacyMetricRecorder>,
        events: Option<SemanticEventEmitter>,
        failure_counters: Vec<Arc<dyn CallbackFailureCounter + Send + Sync>>,
    ) -> LegacyLifecycle {
        let init = LegacyOtelInit {
            config: Some(config),
            callback_name: callback_name.map(str::to_owned),
            overrides: Default::default(),
            injected: Default::default(),
            max_dynamic_tracer_providers: betterlite_logging::otel::legacy_exporters::tracer_cache::MAX_DYNAMIC_TRACER_PROVIDERS,
        };
        let otel = init_legacy_otel(
            init,
            &LegacyOtelEnvironment {
                env: &self.env,
                clients: shared_clients(),
                shared_exporter: self.exporter.clone(),
                globals: &self.globals,
                proxy: Some(&self.proxy),
            },
        )
        .expect("legacy otel init");
        LegacyLifecycle::from_otel(
            otel,
            LegacyLifecycleOptions {
                dynamic: dynamic.unwrap_or_else(|| Arc::new(BaseDynamic)),
                metrics,
                events,
                failure_counters,
                live: Arc::clone(&self.roots),
                use_request_span,
                clock,
                handler_class_name: "OpenTelemetry".to_owned(),
            },
        )
    }

    pub(super) fn spans(&self) -> Vec<SpanData> {
        self.exporter.get_finished_spans().expect("finished spans")
    }

    pub(super) fn names(&self) -> Vec<String> {
        self.spans()
            .iter()
            .map(|span| span.name.to_string())
            .collect()
    }

    pub(super) fn span(&self, name: &str) -> SpanData {
        self.spans()
            .into_iter()
            .find(|span| span.name.to_string() == name)
            .unwrap_or_else(|| panic!("span {name} was not exported"))
    }

    pub(super) fn parent(&self, lifecycle: &LegacyLifecycle, name: &str) -> LiveSpan {
        let builder = lifecycle
            .tracer
            .span_builder(name.to_owned())
            .with_start_time(instant(START));
        let live = LiveSpan::from(
            lifecycle
                .tracer
                .build_with_context(builder, &Context::new()),
        );
        self.roots.register(&live);
        live
    }
}

pub(super) struct CountingFailureCounter(pub AtomicUsize);

impl CallbackFailureCounter for CountingFailureCounter {
    fn increment_callback_logging_failure(&self, _callback_name: &str) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

pub(super) struct ProbeDynamic {
    pub(super) config: Option<OpenTelemetryConfig>,
    pub(super) headers: Option<BTreeMap<String, String>>,
}

impl DynamicOtel for ProbeDynamic {
    fn construct_dynamic_otel_headers(
        &self,
        _dynamic_params: &MetadataMap,
    ) -> Option<BTreeMap<String, String>> {
        self.headers.clone()
    }

    fn construct_dynamic_otel_config(
        &self,
        _dynamic_params: &MetadataMap,
    ) -> Option<OpenTelemetryConfig> {
        self.config.clone()
    }
}

pub(super) fn with_dynamic(mut kwargs: MetadataMap, params: serde_json::Value) -> MetadataMap {
    kwargs.insert(
        "standard_callback_dynamic_params".to_owned(),
        serde_json::from_value(params).expect("dynamic params fixture"),
    );
    kwargs
}

struct BaseDynamic;

impl DynamicOtel for BaseDynamic {
    fn construct_dynamic_otel_headers(
        &self,
        _dynamic_params: &MetadataMap,
    ) -> Option<BTreeMap<String, String>> {
        None
    }

    fn construct_dynamic_otel_config(
        &self,
        _dynamic_params: &MetadataMap,
    ) -> Option<OpenTelemetryConfig> {
        None
    }
}

pub(super) fn in_memory_config() -> OpenTelemetryConfig {
    OpenTelemetryConfig {
        exporter: betterlite_logging::otel::legacy_policy::config::Exporter::InMemory,
        endpoint: None,
        headers: None,
        signals: betterlite_logging::otel::legacy_policy::config::Signals {
            enable_metrics: false,
            enable_events: false,
        },
        service_name: String::new(),
        deployment_environment: String::new(),
        model_id: String::new(),
        ignore_context_propagation: false,
        skip_set_global: false,
        capture_message_content: None,
        semconv_stability_opt_in: Default::default(),
        baggage_team_metadata_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        attributes: None,
    }
}

pub(super) fn instant(seconds: f64) -> std::time::SystemTime {
    betterlite_logging::otel::legacy_lifecycle::lifecycle::instant_from_ns(
        betterlite_logging::otel::legacy_policy::span_time::to_ns(SpanTime::Seconds(seconds), 0),
    )
}

pub(super) fn kwargs(json: &str) -> MetadataMap {
    serde_json::from_str(json).expect("kwargs fixture")
}

pub(super) fn simple_kwargs(call_id: &str) -> MetadataMap {
    kwargs(&format!(
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

pub(super) fn with_team(kwargs: &mut MetadataMap) {
    let Some(OtelModelValue::Object(payload)) = kwargs.get_mut("standard_logging_object") else {
        panic!("standard_logging_object must be an object");
    };
    let Some(OtelModelValue::Object(metadata)) = payload.get_mut("metadata") else {
        panic!("standard_logging_object.metadata must be an object");
    };
    metadata.insert(
        "user_api_key_team_id".to_owned(),
        OtelModelValue::String("team-a".to_owned()),
    );
    metadata.insert(
        "user_api_key_team_alias".to_owned(),
        OtelModelValue::String("team-alias".to_owned()),
    );
}

pub(super) fn call<'call>(kwargs: &'call MetadataMap) -> LifecycleCall<'call> {
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

pub(super) fn carry_in(kwargs: &mut MetadataMap, bucket: &str, span: &LiveSpan) {
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

pub(super) fn attach(context: &SpanContext) -> Context {
    Context::new().with_remote_span_context(context.clone())
}

pub(super) fn attribute<'span>(
    span: &'span SpanData,
    key: &str,
) -> Option<&'span opentelemetry::Value> {
    span.attributes
        .iter()
        .find(|pair| pair.key.as_str() == key)
        .map(|pair| &pair.value)
}

pub(super) fn text_attribute(span: &SpanData, key: &str) -> String {
    match attribute(span, key).unwrap_or_else(|| panic!("attribute {key} is missing")) {
        opentelemetry::Value::String(text) => text.as_str().to_owned(),
        other => panic!("attribute {key} is not text: {other:?}"),
    }
}
