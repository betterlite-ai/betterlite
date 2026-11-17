use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::otel_settings;
use chrono::FixedOffset;
use opentelemetry::InstrumentationScope;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::{SdkTracer, SdkTracerProvider};

use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::logging_pre_post::error::PrePostError;
use crate::logging_pre_post::hooks::{ApiCallLogger, PostCallEvent, PreCallEvent};
use crate::otel::config::Settings;
use crate::otel::emitter::span_emission::{DedupCache, MapperKind, SpanEmitter};
use crate::otel::fan_out::scope::RequestScopeSource;
use crate::otel::legacy_metrics::filter::CallbackSettingsView;
use crate::otel::logger::v2::{
    OpenCall, OpenTelemetryV2, OpenTelemetryV2Parts, RequestTraceSource,
};
use crate::otel::mappers::registry::resolve_mappers;
use crate::otel::plumbing::events::{GenAIEventRecorder, SystemClock};
use crate::otel::plumbing::exporters::{ExporterFactories, HttpClients};
use crate::otel::plumbing::metrics::recorder::GenAiMetricRecorder;
use crate::otel::plumbing::providers::{
    ProviderState, Resolved, get_event_logger, get_meter, resolve_logger_provider,
    resolve_meter_provider,
};
use crate::otel::plumbing::routing::cache::TenantTracerCache;
use crate::otel::v2_callback::error::CallbackError;
use crate::otel::v2_callback::live_span::RootSpans;
use crate::otel::v2_callback::registry::V2Registry;
use crate::otel::v2_callback::span_error::ErrorInformationConfig;
use crate::otel_provider_assembly::metric_instruments::create_instruments;
use crate::otel_provider_assembly::tracer_provider_assembly::{
    AssemblyOptions, build_tracer_provider,
};

const LITELLM_TRACER_NAME: &str = "litellm";

pub struct CallbackParts {
    pub callback_name: Option<String>,
    pub config: Settings,
    pub raw_config: otel_settings::Settings,
    pub tenants: TenantTracerCache,
    pub trace: Arc<dyn RequestTraceSource>,
    pub providers: ProviderSources,
    pub build: ProviderBuild,
    pub metric: MetricSettings,
    pub emitter: EmitterParts,
    pub error_info: ErrorInformationConfig,
}

pub struct ProviderSources {
    pub tracer_provider: Option<SdkTracerProvider>,
    pub meter_provider: Option<SdkMeterProvider>,
    pub logger_provider: Option<SdkLoggerProvider>,
    pub meter_state: ProviderState<SdkMeterProvider>,
    pub logger_state: ProviderState<SdkLoggerProvider>,
    pub metrics: Option<Arc<GenAiMetricRecorder>>,
    pub events: Option<Arc<GenAIEventRecorder>>,
}

pub struct ProviderBuild {
    pub factories: Arc<ExporterFactories>,
    pub scope_source: Arc<dyn RequestScopeSource>,
    pub clients: HttpClients,
}

pub struct MetricSettings {
    pub callback_settings: Arc<dyn CallbackSettingsView>,
    pub utc_offset: FixedOffset,
}

pub struct EmitterParts {
    pub mappers: Vec<MapperKind>,
    pub environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    pub attribute_limit: Option<usize>,
}

// litellm.integrations.otel.logger.OpenTelemetryV2 as the proxy's CustomLogger callback
pub struct V2Callback {
    pub tracer_provider: SdkTracerProvider,
    pub(crate) logger: OpenTelemetryV2,
    pub(crate) roots: RootSpans,
    pub(crate) error_info: ErrorInformationConfig,
}

impl V2Callback {
    pub fn new(parts: CallbackParts, registry: &V2Registry) -> Result<Arc<Self>, CallbackError> {
        let provider = resolve_tracer_provider(&parts)?;
        let tracer = tracer_for(&provider);
        let metrics = resolve_metrics(&parts)?;
        // every mapper name is validated once here, even though the chain itself is injected
        let _validated = resolve_mappers(&parts.config.mapper_names)?;
        let events = resolve_events(&parts)?;
        let emitter = SpanEmitter {
            tracer: tracer.clone(),
            environment: parts.emitter.environment,
            mappers: parts.emitter.mappers,
            event_recorder: events,
            attribute_limit: parts.emitter.attribute_limit,
            emitted: DedupCache::default(),
        };
        let logger = OpenTelemetryV2::with_parts(OpenTelemetryV2Parts {
            name: parts.callback_name.unwrap_or_default(),
            config: parts.config,
            tracer,
            emitter,
            metrics,
            tenants: parts.tenants,
            trace: parts.trace,
        });
        let callback = Arc::new(Self {
            logger,
            tracer_provider: provider,
            roots: RootSpans::default(),
            error_info: parts.error_info,
        });
        registry.register(&callback);
        Ok(callback)
    }

    // litellm.integrations.otel.logger._OPEN_CALLS_MAX bounds the carrier map through B3852's OpenCalls
    pub fn store_open_call(&self, call_id: String, call: OpenCall) {
        self.logger.store_open_call(call_id, call);
    }

    #[must_use]
    pub fn open_call_ids(&self) -> Vec<String> {
        let calls = self.logger.open_calls();
        let mut ids: Vec<String> = Vec::with_capacity(calls.calls.len());
        for id in calls.calls.keys() {
            ids.push(id.clone());
        }
        ids
    }
}

fn resolve_tracer_provider(parts: &CallbackParts) -> Result<SdkTracerProvider, CallbackError> {
    match parts.providers.tracer_provider.as_ref() {
        Some(provider) => Ok(provider.clone()),
        None => Ok(build_tracer_provider(
            &parts.config,
            &parts.build.factories,
            &parts.build.scope_source,
            AssemblyOptions {
                exporter: None,
                baggage_processor: None,
                use_simple_processor: None,
                tenant_overrides: true,
            },
        )?),
    }
}

fn tracer_for(provider: &SdkTracerProvider) -> SdkTracer {
    let scope = InstrumentationScope::builder(LITELLM_TRACER_NAME)
        .with_version(env!("CARGO_PKG_VERSION"))
        .build();
    provider.tracer_with_scope(scope)
}

fn resolve_metrics(
    parts: &CallbackParts,
) -> Result<Option<Arc<GenAiMetricRecorder>>, CallbackError> {
    if !parts.config.enable_metrics {
        return Ok(None);
    }
    if let Some(injected) = parts.providers.metrics.as_ref() {
        return Ok(Some(Arc::clone(injected)));
    }
    let resolved = resolve_meter_provider(
        &parts.raw_config,
        &parts.build.clients,
        parts.providers.meter_provider.clone(),
        parts.providers.meter_state.clone(),
    )?;
    let provider = match resolved {
        Resolved::Reuse(provider) | Resolved::Publish(provider) => provider,
        Resolved::Disabled => return Ok(None),
    };
    let recorder = GenAiMetricRecorder::new(
        create_instruments(&get_meter(&provider, LITELLM_TRACER_NAME)),
        parts.callback_name.clone(),
        Arc::clone(&parts.metric.callback_settings),
        parts.metric.utc_offset,
    )?;
    Ok(Some(Arc::new(recorder)))
}

fn resolve_events(parts: &CallbackParts) -> Result<Option<Arc<GenAIEventRecorder>>, CallbackError> {
    if !parts.config.enable_events {
        return Ok(None);
    }
    if let Some(injected) = parts.providers.events.as_ref() {
        return Ok(Some(Arc::clone(injected)));
    }
    let resolved = resolve_logger_provider(
        &parts.raw_config,
        &parts.build.clients,
        parts.providers.logger_provider.clone(),
        parts.providers.logger_state.clone(),
    )?;
    let provider = match resolved {
        Resolved::Reuse(provider) | Resolved::Publish(provider) => provider,
        Resolved::Disabled => return Ok(None),
    };
    let recorder = GenAIEventRecorder {
        event_logger: get_event_logger(&provider, LITELLM_TRACER_NAME),
        clock: Box::new(SystemClock),
    };
    Ok(Some(Arc::new(recorder)))
}

#[async_trait]
impl CallbackLogger for V2Callback {
    fn callback_name(&self) -> &str {
        self.logger.callback_name()
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.logger.async_log_success_event(event).await
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.logger.async_log_failure_event(event).await
    }
}

impl ApiCallLogger for V2Callback {
    fn log_pre_api_call(&self, event: &PreCallEvent<'_>) -> Result<(), PrePostError> {
        self.logger.log_pre_api_call(event)
    }

    fn log_post_api_call(&self, event: &PostCallEvent<'_>) -> Result<(), PrePostError> {
        self.logger.log_post_api_call(event)
    }
}
