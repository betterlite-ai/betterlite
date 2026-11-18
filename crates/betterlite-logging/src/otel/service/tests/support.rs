use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::SystemTime;

use opentelemetry::Context;
use opentelemetry::propagation::TextMapPropagator as _;
use opentelemetry::trace::{TraceContextExt as _, Tracer as _};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SpanData};

use crate::otel::legacy_exporters::tracer_cache::MAX_DYNAMIC_TRACER_PROVIDERS;
use crate::otel::legacy_lifecycle::context::instant_from_ns;
use crate::otel::legacy_lifecycle::lifecycle::{
    LegacyLifecycle, LegacyLifecycleOptions, LifecycleCall,
};
use crate::otel::legacy_lifecycle::tracer::{BaseDynamicOtel, DynamicOtel};
use crate::otel::legacy_policy::capture::LoggingFlags;
use crate::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig, Signals};
use crate::otel::legacy_policy::span_time::{SpanTime, to_ns};
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::events::Clock;
use crate::otel::plumbing::exporters::HttpClients;
use crate::otel::v2_callback::live_span::{LiveSpan, RootSpans};
use crate::otel_provider_assembly::legacy_otel_init::{
    InjectedProviders, LegacyOtelEnvironment, LegacyOtelInit, MetadataKeyOverrides,
    init_legacy_otel,
};
use crate::otel_provider_assembly::opentelemetry_legacy_provider::GlobalProviders;
use crate::otel_provider_assembly::proxy_registry::ProxyRegistry;

pub(super) const START: f64 = 1_704_067_200.0;
pub(super) const END: f64 = 1_704_067_202.0;
pub(super) const NOW: i64 = 1_704_067_203_000_000_000;

#[derive(Debug)]
struct NoServer;

#[async_trait::async_trait]
impl HttpClient for NoServer {
    async fn send_bytes(&self, _request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        Err(HttpError::from("no HTTP client is wired into this suite"))
    }
}

struct FixedClock(i64);

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        instant_from_ns(self.0)
    }
}

pub(super) struct Rig {
    env: BTreeMap<String, String>,
    globals: GlobalProviders,
    proxy: ProxyRegistry,
    exporter: InMemorySpanExporter,
    roots: Arc<RootSpans>,
}

impl Rig {
    pub(super) fn new() -> Self {
        Self {
            env: BTreeMap::new(),
            globals: GlobalProviders::default(),
            proxy: ProxyRegistry::default(),
            exporter: InMemorySpanExporter::default(),
            roots: Arc::new(RootSpans::default()),
        }
    }

    pub(super) fn lifecycle(&self) -> LegacyLifecycle {
        self.lifecycle_with_dynamic(Arc::new(BaseDynamicOtel))
    }

    pub(super) fn lifecycle_with_dynamic(&self, dynamic: Arc<dyn DynamicOtel>) -> LegacyLifecycle {
        let init = LegacyOtelInit {
            config: Some(Self::config()),
            callback_name: Some("opentelemetry".to_owned()),
            overrides: MetadataKeyOverrides::default(),
            injected: InjectedProviders::default(),
            max_dynamic_tracer_providers: MAX_DYNAMIC_TRACER_PROVIDERS,
        };
        let shared: Arc<dyn HttpClient> = Arc::new(NoServer);
        let otel = init_legacy_otel(
            init,
            &LegacyOtelEnvironment {
                env: &self.env,
                clients: HttpClients {
                    traces: Arc::clone(&shared),
                    metrics: Arc::clone(&shared),
                    logs: shared,
                },
                shared_exporter: self.exporter.clone(),
                globals: &self.globals,
                proxy: Some(&self.proxy),
            },
        )
        .expect("legacy otel init");
        LegacyLifecycle::from_otel(
            otel,
            LegacyLifecycleOptions {
                dynamic,
                live: Arc::clone(&self.roots),
                metrics: None,
                events: None,
                failure_counters: Vec::new(),
                handler_class_name: "OpenTelemetry".to_owned(),
                use_request_span: false,
                clock: Box::new(FixedClock(NOW)),
            },
        )
    }

    pub(super) fn parent(&self, lifecycle: &LegacyLifecycle, name: &str, start: f64) -> LiveSpan {
        let builder = lifecycle
            .tracer
            .span_builder(name.to_owned())
            .with_start_time(instant(start));
        let live = LiveSpan::from(
            lifecycle
                .tracer
                .build_with_context(builder, &Context::new()),
        );
        self.roots.register(&live);
        live
    }

    // a parent that only travels as a traceparent header, which no live registry knows
    pub(super) fn detached(lifecycle: &LegacyLifecycle) -> LiveSpan {
        let builder = lifecycle
            .tracer
            .span_builder("detached".to_owned())
            .with_start_time(instant(START));
        LiveSpan::from(
            lifecycle
                .tracer
                .build_with_context(builder, &Context::new()),
        )
    }

    pub(super) fn spans(&self) -> Vec<SpanData> {
        self.exporter.get_finished_spans().expect("finished spans")
    }

    pub(super) fn named(&self, name: &str) -> Vec<SpanData> {
        self.spans()
            .into_iter()
            .filter(|span| span.name == name)
            .collect()
    }

    pub(super) fn span(&self, name: &str) -> SpanData {
        self.named(name)
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("span {name} was not exported"))
    }

    fn config() -> OpenTelemetryConfig {
        OpenTelemetryConfig {
            exporter: Exporter::InMemory,
            endpoint: None,
            headers: None,
            signals: Signals {
                enable_metrics: false,
                enable_events: false,
            },
            service_name: String::new(),
            deployment_environment: String::new(),
            model_id: String::new(),
            ignore_context_propagation: false,
            skip_set_global: true,
            capture_message_content: None,
            semconv_stability_opt_in: BTreeSet::new(),
            baggage_team_metadata_keys: Vec::new(),
            baggage_metadata_keys: Vec::new(),
            attributes: None,
        }
    }
}

pub(super) fn instant(seconds: f64) -> SystemTime {
    instant_from_ns(to_ns(SpanTime::Seconds(seconds), 0))
}

pub(super) fn now() -> SystemTime {
    instant_from_ns(NOW)
}

pub(super) fn kwargs(json: &str) -> MetadataMap {
    serde_json::from_str(json).expect("kwargs fixture")
}

pub(super) fn call(kwargs: &MetadataMap) -> LifecycleCall<'_> {
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

// the parent span travels as a live object, so a real request hands it to the hook through the metadata
pub(super) fn carry_in(kwargs: &mut MetadataMap, span: &LiveSpan) {
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
        .entry("metadata".to_owned())
        .or_insert_with(|| OtelModelValue::Object(MetadataMap::default()));
    let OtelModelValue::Object(metadata) = fields else {
        panic!("litellm_params.metadata must be an object");
    };
    metadata.insert(
        "litellm_parent_otel_span".to_owned(),
        OtelModelValue::String(traceparent),
    );
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
    let value = attribute(span, key).unwrap_or_else(|| panic!("attribute {key} is missing"));
    let opentelemetry::Value::String(text) = value else {
        panic!("attribute {key} is not text: {value:?}");
    };
    text.as_str().to_owned()
}

pub(super) fn i64_attribute(span: &SpanData, key: &str) -> i64 {
    let value = attribute(span, key).unwrap_or_else(|| panic!("attribute {key} is missing"));
    let opentelemetry::Value::I64(number) = value else {
        panic!("attribute {key} is not an integer: {value:?}");
    };
    *number
}

pub(super) fn f64_attribute(span: &SpanData, key: &str) -> f64 {
    let value = attribute(span, key).unwrap_or_else(|| panic!("attribute {key} is missing"));
    let opentelemetry::Value::F64(number) = value else {
        panic!("attribute {key} is not a float: {value:?}");
    };
    *number
}

pub(super) fn event_attributes(span: &SpanData) -> Vec<(String, String)> {
    let event = span
        .events
        .first()
        .expect("the recorded exception is one event");
    assert_eq!(event.name.as_ref(), "exception");
    event
        .attributes
        .iter()
        .map(|pair| (pair.key.to_string(), pair.value.to_string()))
        .collect()
}

pub(super) fn event_attribute(span: &SpanData, key: &str) -> Option<String> {
    event_attributes(span)
        .into_iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
}
