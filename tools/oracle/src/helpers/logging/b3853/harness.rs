use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_config::sections::otel_settings;
use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_error::normalize::Normalizer;
use betterlite_logging::otel::config::{CaptureMessageContent, Settings, destination::SpanScope};
use betterlite_logging::otel::emitter::span_emission::MapperKind;
use betterlite_logging::otel::fan_out::scope::{RequestOtelScope, RequestScopeSource};
use betterlite_logging::otel::legacy_metrics::filter::CallbackSettingsView;
use betterlite_logging::otel::logger::v2::RequestTraceSource;
use betterlite_logging::otel::mappers::{genai::GenAIMapper, utils::tool_attr_budget};
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::plumbing::context::context_slots::RequestTraceContext;
use betterlite_logging::otel::plumbing::exporters::{ExporterFactories, HttpClients};
use betterlite_logging::otel::plumbing::providers::ProviderState;
use betterlite_logging::otel::plumbing::routing::cache::{TenantSources, TenantTracerCache};
use betterlite_logging::otel::v2_callback::callback::{
    CallbackParts, EmitterParts, MetricSettings, ProviderBuild, ProviderSources, V2Callback,
};
use betterlite_logging::otel::v2_callback::live_span::LiveSpan;
use betterlite_logging::otel::v2_callback::registry::V2Registry;
use betterlite_logging::otel::v2_callback::span_error::ErrorInformationConfig;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use chrono::{Offset as _, Utc};
use opentelemetry::trace::{Tracer as _, TracerProvider as _};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SimpleSpanProcessor};

pub(super) struct Harness {
    pub(super) callback: Arc<V2Callback>,
    pub(super) spans: InMemorySpanExporter,
    pub(super) root: LiveSpan,
}

pub(super) fn build(registry: &V2Registry) -> Result<Harness, Box<HelperError>> {
    let spans = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_span_processor(SimpleSpanProcessor::new(spans.clone()))
        .build();
    let tracer = provider.tracer("litellm");
    let root = LiveSpan::from(tracer.start("root".to_owned()));
    let callback = V2Callback::new(parts(provider)?, registry).or_config_error()?;
    Ok(Harness {
        callback,
        spans,
        root,
    })
}

fn parts(provider: SdkTracerProvider) -> Result<CallbackParts, Box<HelperError>> {
    let config = settings();
    Ok(CallbackParts {
        callback_name: Some("b3853".to_owned()),
        tenants: tenants(&config),
        config,
        raw_config: otel_settings::Settings::default(),
        trace: Arc::new(Inert),
        providers: ProviderSources {
            tracer_provider: Some(provider),
            meter_provider: None,
            logger_provider: None,
            meter_state: ProviderState::Unset,
            logger_state: ProviderState::Unset,
            metrics: None,
            events: None,
        },
        build: ProviderBuild {
            factories: Arc::new(ExporterFactories::new(clients(), BTreeMap::new())),
            scope_source: Arc::new(Inert),
            clients: clients(),
        },
        metric: MetricSettings {
            callback_settings: Arc::new(Inert),
            utc_offset: Utc.fix(),
        },
        emitter: EmitterParts {
            mappers: vec![MapperKind::GenAI(GenAIMapper {
                tool_attr_budget: tool_attr_budget(1),
            })],
            environment: Arc::new(BTreeMap::<String, String>::new()),
            attribute_limit: None,
        },
        error_info: ErrorInformationConfig {
            normalizer: Normalizer::new().or_config_error()?,
            redactor: None,
            log_client_error_tracebacks: false,
        },
    })
}

fn tenants(config: &Settings) -> TenantTracerCache {
    TenantTracerCache::new(
        config.clone(),
        Some("b3853".to_owned()),
        "litellm",
        TenantSources {
            factories: Arc::new(ExporterFactories::new(clients(), BTreeMap::new())),
            scope_source: Arc::new(Inert),
            allowed_hosts: Arc::from(Vec::<AllowedDestination>::new()),
        },
    )
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

fn clients() -> HttpClients {
    HttpClients {
        traces: Arc::new(Inert),
        metrics: Arc::new(Inert),
        logs: Arc::new(Inert),
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
