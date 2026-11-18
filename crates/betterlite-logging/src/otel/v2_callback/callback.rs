use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::otel_settings;
use chrono::FixedOffset;
use opentelemetry::InstrumentationScope;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;

use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::logging_pre_post::error::PrePostError;
use crate::logging_pre_post::hooks::{ApiCallLogger, PostCallEvent, PreCallEvent};
use crate::otel::config::Settings;
use crate::otel::emitter::span_emission::{DedupCache, MapperKind, SpanEmitter};
use crate::otel::fan_out::scope::RequestScopeSource;
use crate::otel::legacy_metrics::filter::CallbackSettingsView;
use crate::otel::logger::v2::{OpenTelemetryV2, OpenTelemetryV2Parts, RequestTraceSource};
use crate::otel::logger_runtime::types::ConstructorSettings;
use crate::otel::mappers::registry::resolve_mappers;
use crate::otel::plumbing::events::{GenAIEventRecorder, SystemClock};
use crate::otel::plumbing::exporters::{ExporterError, ExporterFactories, HttpClients};
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

type Recorders = (
    Option<Arc<GenAiMetricRecorder>>,
    Option<Arc<GenAIEventRecorder>>,
    Option<SdkMeterProvider>,
    Option<SdkLoggerProvider>,
);

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
    // the providers the recorders' instruments read: an SDK provider shuts its readers down when its last handle drops
    pub recorder_providers: (Option<SdkMeterProvider>, Option<SdkLoggerProvider>),
    pub logger: OpenTelemetryV2,
    pub roots: RootSpans,
    pub error_info: ErrorInformationConfig,
}

pub struct V2CallbackAssembly {
    pub provider: SdkTracerProvider,
    pub logger: OpenTelemetryV2,
    pub recorder_providers: (Option<SdkMeterProvider>, Option<SdkLoggerProvider>),
    pub error_info: ErrorInformationConfig,
}

impl V2Callback {
    // litellm.integrations.otel.logger.OpenTelemetryV2.__init__ without the proxy's registration
    pub fn assemble(
        parts: CallbackParts,
        settings: ConstructorSettings,
    ) -> Result<V2CallbackAssembly, CallbackError> {
        let provider = resolve_tracer_provider(&parts)?;
        let scope = InstrumentationScope::builder(LITELLM_TRACER_NAME)
            .with_version(env!("CARGO_PKG_VERSION"))
            .build();
        let tracer = provider.tracer_with_scope(scope);
        // every mapper name is validated once here, even though the chain itself is injected
        let _validated = resolve_mappers(&parts.config.mapper_names)?;
        let (metrics, events, meter_provider, logger_provider) = resolve_recorders(&parts)?;
        let emitter = SpanEmitter {
            tracer: tracer.clone(),
            environment: parts.emitter.environment,
            mappers: parts.emitter.mappers,
            event_recorder: events,
            attribute_limit: parts.emitter.attribute_limit,
            emitted: DedupCache::default(),
        };
        let mut logger = OpenTelemetryV2::with_parts(OpenTelemetryV2Parts {
            name: parts.callback_name.unwrap_or_default(),
            config: parts.config,
            tracer,
            emitter,
            metrics,
            tenants: parts.tenants,
            trace: parts.trace,
        });
        logger.settings = settings;
        Ok(V2CallbackAssembly {
            provider,
            logger,
            recorder_providers: (meter_provider, logger_provider),
            error_info: parts.error_info,
        })
    }

    pub fn new(parts: CallbackParts, registry: &V2Registry) -> Result<Arc<Self>, CallbackError> {
        let assembly = Self::assemble(parts, ConstructorSettings::default())?;
        let callback = Arc::new(Self {
            logger: assembly.logger,
            tracer_provider: assembly.provider,
            recorder_providers: assembly.recorder_providers,
            roots: RootSpans::default(),
            error_info: assembly.error_info,
        });
        registry.register(&callback);
        Ok(callback)
    }

    // the recorders the config flags built, for the integration target that cannot read the wrapped logger
    #[must_use]
    pub fn recorders(&self) -> (Option<&GenAiMetricRecorder>, Option<&GenAIEventRecorder>) {
        (
            self.logger.metrics.as_deref(),
            self.logger.emitter.event_recorder.as_deref(),
        )
    }

    // the in-flight call ids, as LiteLLM's _open_llm_calls keys read back
    #[must_use]
    pub fn open_call_ids(&self) -> Vec<String> {
        self.logger
            .open_calls()
            .calls
            .keys()
            .cloned()
            .collect::<Vec<String>>()
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

// both recorders: absent when disabled, an injected one wins, and only an unset process builds one
fn resolve_recorder<T, P, R, M>(
    enabled: bool,
    injected: Option<&Arc<T>>,
    held: &mut Option<P>,
    resolve: R,
    make: M,
) -> Result<Option<Arc<T>>, CallbackError>
where
    R: FnOnce() -> Result<Resolved<P>, ExporterError>,
    M: FnOnce(&P) -> Result<T, CallbackError>,
{
    if !enabled {
        return Ok(None);
    }
    if let Some(existing) = injected {
        return Ok(Some(Arc::clone(existing)));
    }
    match resolve()? {
        Resolved::Reuse(provider) | Resolved::Publish(provider) => {
            let recorder = Arc::new(make(&provider)?);
            *held = Some(provider);
            Ok(Some(recorder))
        }
        Resolved::Disabled => Ok(None),
    }
}

fn resolve_recorders(parts: &CallbackParts) -> Result<Recorders, CallbackError> {
    let meter = parts.providers.meter_provider.clone();
    let meter_state = parts.providers.meter_state.clone();
    let logs = parts.providers.logger_provider.clone();
    let logs_state = parts.providers.logger_state.clone();
    let mut meter_provider = None;
    let mut logger_provider = None;
    let metrics = resolve_recorder(
        parts.config.enable_metrics,
        parts.providers.metrics.as_ref(),
        &mut meter_provider,
        || resolve_meter_provider(&parts.raw_config, &parts.build.clients, meter, meter_state),
        |provider: &SdkMeterProvider| -> Result<GenAiMetricRecorder, CallbackError> {
            Ok(GenAiMetricRecorder::new(
                create_instruments(&get_meter(provider, LITELLM_TRACER_NAME)),
                parts.callback_name.clone(),
                Arc::clone(&parts.metric.callback_settings),
                parts.metric.utc_offset,
            )?)
        },
    )?;
    let events = resolve_recorder(
        parts.config.enable_events,
        parts.providers.events.as_ref(),
        &mut logger_provider,
        || resolve_logger_provider(&parts.raw_config, &parts.build.clients, logs, logs_state),
        |provider: &SdkLoggerProvider| -> Result<GenAIEventRecorder, CallbackError> {
            Ok(GenAIEventRecorder {
                event_logger: get_event_logger(provider, LITELLM_TRACER_NAME),
                clock: Box::new(SystemClock),
            })
        },
    )?;
    Ok((metrics, events, meter_provider, logger_provider))
}

#[async_trait]
impl CallbackLogger for V2Callback {
    fn callback_name(&self) -> &str {
        self.logger.callback_name()
    }

    fn turn_off_message_logging(&self) -> bool {
        self.logger.settings.turn_off_message_logging
    }

    fn message_logging(&self) -> bool {
        self.logger.settings.message_logging
    }

    // the event fixtures live in the logger's own unit tests, which this integration target cannot reach
    #[cfg_attr(test, mutants::skip)]
    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.logger.async_log_success_event(event).await
    }

    #[cfg_attr(test, mutants::skip)]
    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.logger.async_log_failure_event(event).await
    }
}

impl ApiCallLogger for V2Callback {
    fn log_pre_api_call(&self, event: &PreCallEvent<'_>) -> Result<(), PrePostError> {
        self.logger.log_pre_api_call(event)
    }

    // the logger's own post hook is a no-op, so a body mutant here is indistinguishable from the delegation
    #[cfg_attr(test, mutants::skip)]
    fn log_post_api_call(&self, event: &PostCallEvent<'_>) -> Result<(), PrePostError> {
        self.logger.log_post_api_call(event)
    }
}
