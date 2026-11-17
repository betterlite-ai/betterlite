use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_logging::otel::config::destination::SpanScope;
use betterlite_logging::otel::config::{CaptureMessageContent, Settings};
use betterlite_logging::otel::emitter::span_emission::{DedupCache, SpanEmitter};
use betterlite_logging::otel::fan_out::scope::{RequestOtelScope, RequestScopeSource};
use betterlite_logging::otel::legacy_metrics::filter::CallbackSettingsView;
use betterlite_logging::otel::logger::v2::{
    OpenTelemetryV2, OpenTelemetryV2Parts, RequestTraceSource,
};
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::plumbing::context::context_slots::{
    AnchoredSpan, RequestTraceContext,
};
use betterlite_logging::otel::plumbing::exporters::ExporterFactories;
use betterlite_logging::otel::plumbing::metrics::recorder::GenAiMetricRecorder;
use betterlite_logging::otel::plumbing::routing::cache::{TenantSources, TenantTracerCache};
use betterlite_logging::otel::plumbing::sinks::SpanSink;
use betterlite_logging::otel_provider_assembly::metric_instruments::create_instruments;
use betterlite_logging::otel_provider_assembly::tracer_provider_assembly::{
    AssemblyOptions, build_tracer_provider,
};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use chrono::FixedOffset;
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry::trace::{
    SpanContext, SpanId, TraceFlags, TraceId, TraceState, TracerProvider as _,
};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracer};

use super::exporters::{Built, ExporterInput, exporter_spec, factories};

// the same anchored request root the pin's OTel tests use
const ROOT_TRACE: u128 = 0x0af7_6519_16cd_43dd_8448_eb21_1c80_319c;
const ROOT_SPAN: u64 = 0xb7ad_6b71_6920_3331;

#[derive(Debug, thiserror::Error)]
pub(super) enum HarnessError {
    #[error("the payload fixture is not an object")]
    Fixture,
    #[error("no utc offset")]
    Offset,
    #[error("the route built no provider to inspect")]
    NoProvider,
    #[error("the eviction scenario needs at least one space")]
    NoSpaces,
}

pub(super) struct Harness {
    pub(super) logger: OpenTelemetryV2,
    pub(super) default: InMemorySpanExporter,
    pub(super) built: Built,
}

pub(super) struct CacheHarness {
    pub(super) cache: TenantTracerCache,
    pub(super) default: SdkTracer,
    pub(super) built: Built,
}

pub(super) fn settings(exporters: &[ExporterInput]) -> Result<Settings, Box<HelperError>> {
    Ok(Settings {
        exporters: exporters
            .iter()
            .map(exporter_spec)
            .collect::<Result<Vec<_>, _>>()?,
        service_name: "litellm".to_owned(),
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
    })
}

// the request's own start stands in for an absent carrier at close, as the anchored request context carries it when pre_call saw a parent
pub(super) fn trace(anchored: bool) -> RequestTraceContext {
    let root = anchored.then(|| AnchoredSpan {
        context: SpanContext::new(
            TraceId::from(ROOT_TRACE),
            SpanId::from(ROOT_SPAN),
            TraceFlags::SAMPLED,
            true,
            TraceState::default(),
        ),
        http_route: Some("/v1/chat/completions".to_owned()),
    });
    RequestTraceContext {
        root,
        mcp_carrier: None,
        mcp_transport: None,
        destinations: Vec::new(),
    }
}

pub(super) fn make_harness(
    settings: Settings,
    callback: Option<String>,
    metrics: Option<Arc<GenAiMetricRecorder>>,
    trace: RequestTraceContext,
) -> Result<Harness, Box<HelperError>> {
    let built = Built::default();
    let default = InMemorySpanExporter::default();
    let factories = factories(&built);
    let scope: Arc<dyn RequestScopeSource> = Arc::new(FixedScope);
    let sink = SpanSink::InMemory(default.clone());
    let tracer = default_tracer(&settings, &factories, &scope, sink)?;
    let emitter = span_emitter(&tracer);
    let sources = TenantSources {
        factories,
        scope_source: Arc::clone(&scope),
        allowed_hosts: Arc::from(Vec::<AllowedDestination>::new()),
    };
    let tenants = TenantTracerCache::new(settings.clone(), callback.clone(), "litellm", sources);
    Ok(Harness {
        logger: OpenTelemetryV2::with_parts(OpenTelemetryV2Parts {
            name: callback.unwrap_or_default(),
            config: settings,
            tracer,
            emitter,
            metrics,
            tenants,
            trace: Arc::new(FixedTrace(Mutex::new(trace))),
        }),
        default,
        built,
    })
}

pub(super) fn make_cache(
    settings: Settings,
    callback: Option<String>,
) -> Result<CacheHarness, Box<HelperError>> {
    let built = Built::default();
    let factories = factories(&built);
    let scope: Arc<dyn RequestScopeSource> = Arc::new(FixedScope);
    let sink = SpanSink::InMemory(InMemorySpanExporter::default());
    let default = default_tracer(&settings, &factories, &scope, sink)?;
    let sources = TenantSources {
        factories,
        scope_source: scope,
        allowed_hosts: Arc::from(Vec::<AllowedDestination>::new()),
    };
    Ok(CacheHarness {
        cache: TenantTracerCache::new(settings, callback, "litellm", sources),
        default,
        built,
    })
}

// the provider the logger emits on when no tenant routes the request, in memory so nothing leaves
fn default_tracer(
    settings: &Settings,
    factories: &ExporterFactories,
    scope: &Arc<dyn RequestScopeSource>,
    sink: SpanSink,
) -> Result<SdkTracer, Box<HelperError>> {
    let provider = build_tracer_provider(
        settings,
        factories,
        scope,
        AssemblyOptions {
            exporter: Some(sink),
            baggage_processor: None,
            use_simple_processor: Some(true),
            tenant_overrides: false,
        },
    )
    .or_config_error()?;
    Ok(provider.tracer("litellm"))
}

fn span_emitter(tracer: &SdkTracer) -> SpanEmitter<SdkTracer> {
    let environment: Arc<dyn EnvironmentLookup + Send + Sync> =
        Arc::new(BTreeMap::<String, String>::new());
    SpanEmitter {
        tracer: tracer.clone(),
        environment,
        mappers: Vec::new(),
        event_recorder: None,
        attribute_limit: None,
        emitted: DedupCache::default(),
    }
}

#[derive(Debug)]
struct FixedScope;

impl RequestScopeSource for FixedScope {
    fn scope(&self) -> RequestOtelScope {
        RequestOtelScope::default()
    }
}

#[derive(Debug)]
struct FixedTrace(Mutex<RequestTraceContext>);

impl RequestTraceSource for FixedTrace {
    fn trace_context(&self) -> RequestTraceContext {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

struct FilterSettings(Option<OtelModelValue>);

impl CallbackSettingsView for FilterSettings {
    fn otel_attributes(&self) -> Option<OtelModelValue> {
        self.0.clone()
    }
}

pub(super) fn metric_recorder(
    attributes: OtelModelValue,
) -> Result<(Arc<GenAiMetricRecorder>, SdkMeterProvider), Box<HelperError>> {
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter).build())
        .build();
    let recorder = GenAiMetricRecorder::new(
        create_instruments(&provider.meter("b3852")),
        // the filter is read only for the unnamed callback and for "otel", as LiteLLM reads it
        None,
        Arc::new(FilterSettings(Some(attributes))),
        FixedOffset::east_opt(0)
            .ok_or(HarnessError::Offset)
            .or_value_error()?,
    )
    .or_value_error()?;
    Ok((Arc::new(recorder), provider))
}
