use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_config::sections::otel_settings;
use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_error::normalize::Normalizer;
use betterlite_logging::otel::config::destination::SpanScope;
use betterlite_logging::otel::config::{CaptureMessageContent, Settings};
use betterlite_logging::otel::emitter::span_emission::MapperKind;
use betterlite_logging::otel::fan_out::scope::{RequestOtelScope, RequestScopeSource};
use betterlite_logging::otel::legacy_metrics::filter::CallbackSettingsView;
use betterlite_logging::otel::logger::v2::RequestTraceSource;
use betterlite_logging::otel::mappers::genai::GenAIMapper;
use betterlite_logging::otel::mappers::utils::tool_attr_budget;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::plumbing::context::context_slots::RequestTraceContext;
use betterlite_logging::otel::plumbing::exporters::{
    ExporterFactories, ExporterFactory, HttpClients,
};
use betterlite_logging::otel::plumbing::providers::ProviderState;
use betterlite_logging::otel::plumbing::routing::cache::{TenantSources, TenantTracerCache};
use betterlite_logging::otel::v2_callback::callback::{
    CallbackParts, EmitterParts, MetricSettings, ProviderBuild, ProviderSources, V2Callback,
};
use betterlite_logging::otel::v2_callback::live_span::LiveSpan;
use betterlite_logging::otel::v2_callback::registry::V2Registry;
use betterlite_logging::otel::v2_callback::span_error::ErrorInformationConfig;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use chrono::{TimeZone as _, Utc};
use opentelemetry::trace::{Tracer as _, TracerProvider as _};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::trace::{
    InMemorySpanExporter, SdkTracer, SdkTracerProvider, SimpleSpanProcessor,
    SpanData as SdkSpanData,
};

const LITELLM_TRACER_NAME: &str = "litellm";

pub(super) struct Harness {
    pub(super) callback: Arc<V2Callback>,
    pub(super) spans: InMemorySpanExporter,
    tracer: SdkTracer,
}

// a callback whose provider exports into the in-memory sink the case reads back
pub(super) fn build(registry: &V2Registry) -> Result<Harness, Box<HelperError>> {
    let spans = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_span_processor(SimpleSpanProcessor::new(spans.clone()))
        .build();
    let tracer = provider.tracer(LITELLM_TRACER_NAME);
    let callback = V2Callback::new(parts(provider)?, registry).or_config_error()?;
    Ok(Harness {
        callback,
        spans,
        tracer,
    })
}

impl Harness {
    pub(super) fn span(&self, name: &str) -> LiveSpan {
        LiveSpan::from(self.tracer.start(name.to_owned()))
    }

    pub(super) fn finished(&self) -> Result<Vec<SdkSpanData>, Box<HelperError>> {
        self.spans.get_finished_spans().or_config_error()
    }
}

fn parts(provider: SdkTracerProvider) -> Result<CallbackParts, Box<HelperError>> {
    let config = settings();
    let factories = factories();
    let scope_source: Arc<dyn RequestScopeSource> = Arc::new(Inert);
    Ok(CallbackParts {
        callback_name: Some("b3853".to_owned()),
        tenants: tenants(&config, &factories, &scope_source),
        config,
        raw_config: otel_settings::Settings::default(),
        trace: Arc::new(Inert),
        providers: providers(provider),
        build: ProviderBuild {
            factories: Arc::clone(&factories),
            scope_source,
            clients: clients(),
        },
        metric: metric(),
        emitter: emitter(),
        error_info: error_info()?,
    })
}

fn settings() -> Settings {
    Settings {
        exporters: Vec::new(),
        service_name: "betterlite".to_owned(),
        deployment_environment: None,
        enable_metrics: false,
        enable_events: false,
        capture_message_content: CaptureMessageContent::SpanOnly,
        legacy_compat: false,
        langfuse_span_scope: SpanScope::Full,
        excluded_services: BTreeSet::new(),
        mapper_names: vec!["genai".to_owned()],
        resource_attributes: BTreeMap::new(),
        baggage_promoted_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        baggage_team_metadata_keys: Vec::new(),
    }
}

fn factories() -> Arc<ExporterFactories> {
    Arc::new(ExporterFactories::new(
        clients(),
        BTreeMap::<String, ExporterFactory>::new(),
    ))
}

fn tenants(
    config: &Settings,
    factories: &Arc<ExporterFactories>,
    scope_source: &Arc<dyn RequestScopeSource>,
) -> TenantTracerCache {
    TenantTracerCache::new(
        config.clone(),
        Some("b3853".to_owned()),
        LITELLM_TRACER_NAME,
        TenantSources {
            factories: Arc::clone(factories),
            scope_source: Arc::clone(scope_source),
            allowed_hosts: Arc::from(Vec::<AllowedDestination>::new()),
        },
    )
}

fn providers(provider: SdkTracerProvider) -> ProviderSources {
    ProviderSources {
        tracer_provider: Some(provider),
        meter_provider: None,
        logger_provider: None,
        meter_state: ProviderState::Unset,
        logger_state: ProviderState::Unset,
        metrics: None,
        events: None,
    }
}

fn metric() -> MetricSettings {
    MetricSettings {
        callback_settings: Arc::new(Inert),
        utc_offset: Utc.fix(),
    }
}

fn emitter() -> EmitterParts {
    EmitterParts {
        mappers: vec![MapperKind::GenAI(GenAIMapper {
            tool_attr_budget: tool_attr_budget(1),
        })],
        environment: Arc::new(BTreeMap::<String, String>::new()),
        attribute_limit: None,
    }
}

fn error_info() -> Result<ErrorInformationConfig, Box<HelperError>> {
    Ok(ErrorInformationConfig {
        normalizer: Normalizer::new().or_config_error()?,
        redactor: None,
        log_client_error_tracebacks: false,
    })
}

fn clients() -> HttpClients {
    let client: Arc<dyn HttpClient> = Arc::new(Inert);
    HttpClients {
        traces: Arc::clone(&client),
        metrics: Arc::clone(&client),
        logs: client,
    }
}

#[derive(Debug)]
struct Inert;

#[async_trait::async_trait]
impl HttpClient for Inert {
    async fn send_bytes(&self, _request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        Err(Box::from("the b3853 cases export in memory"))
    }
}

impl RequestScopeSource for Inert {
    fn scope(&self) -> RequestOtelScope {
        RequestOtelScope::default()
    }
}

impl RequestTraceSource for Inert {
    fn trace_context(&self) -> RequestTraceContext {
        RequestTraceContext::default()
    }
}

impl CallbackSettingsView for Inert {
    fn otel_attributes(&self) -> Option<OtelModelValue> {
        None
    }
}
