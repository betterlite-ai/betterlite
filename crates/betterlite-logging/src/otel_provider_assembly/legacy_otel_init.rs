use std::collections::BTreeMap;

use opentelemetry::metrics::MeterProvider as _;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::logs::{BatchLogProcessor, SdkLoggerProvider};
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracer, SdkTracerProvider};
use tracing::warn;

use crate::otel::legacy_exporters::build::LegacyExporterBuilder;
use crate::otel::legacy_exporters::error::LegacyExporterError;
use crate::otel::legacy_exporters::resource::legacy_resource;
use crate::otel::legacy_exporters::selection::{
    log_exporter_plan, metric_reader_plan, span_processor_plan,
};
use crate::otel::legacy_exporters::tracer_cache::{
    CacheSettings, DynamicTracerCache, MAX_DYNAMIC_TRACER_PROVIDERS, legacy_tracer_name,
};
use crate::otel::legacy_policy::capture::CapturePolicy;
use crate::otel::legacy_policy::config::{
    MetadataKeys, OpenTelemetryConfig, normalize_metadata_keys,
};
use crate::otel::plumbing::exporters::HttpClients;
use crate::otel_provider_assembly::metric_instruments::{MetricInstruments, create_instruments};
use crate::otel_provider_assembly::opentelemetry_legacy_provider::{
    GlobalProviders, get_or_create_provider,
};
use crate::otel_provider_assembly::proxy_registry::{
    OtelRegistration, ProxyRegistry, register_on_proxy,
};

// LiteLLM names the meter after its module, so metrics keep that scope.
const METER_NAME: &str = "litellm.integrations.opentelemetry";
// langfuse_otel relies on the Langfuse SDK's providers, so it must not overwrite them.
const LANGFUSE_OTEL: &str = "langfuse_otel";

#[derive(Default)]
pub struct InjectedProviders {
    pub tracer: Option<SdkTracerProvider>,
    pub meter: Option<SdkMeterProvider>,
    pub logger: Option<SdkLoggerProvider>,
}

#[derive(Default)]
pub struct MetadataKeyOverrides<'keys> {
    pub team: Option<MetadataKeys<'keys>>,
    pub metadata: Option<MetadataKeys<'keys>>,
}

pub struct LegacyOtelInit<'keys> {
    pub config: Option<OpenTelemetryConfig>,
    pub callback_name: Option<String>,
    pub overrides: MetadataKeyOverrides<'keys>,
    pub injected: InjectedProviders,
    pub max_dynamic_tracer_providers: usize,
}

impl Default for LegacyOtelInit<'_> {
    fn default() -> Self {
        Self {
            config: None,
            callback_name: None,
            overrides: MetadataKeyOverrides::default(),
            injected: InjectedProviders::default(),
            max_dynamic_tracer_providers: MAX_DYNAMIC_TRACER_PROVIDERS,
        }
    }
}

pub struct LegacyOtelEnvironment<'env> {
    pub env: &'env BTreeMap<String, String>,
    pub clients: HttpClients,
    pub shared_exporter: InMemorySpanExporter,
    pub globals: &'env GlobalProviders,
    pub proxy: Option<&'env ProxyRegistry>,
}

pub struct LegacyOtel {
    pub config: OpenTelemetryConfig,
    pub callback_name: Option<String>,
    pub resource: Resource,
    pub tracer_provider: SdkTracerProvider,
    pub tracer: SdkTracer,
    pub tracer_cache: DynamicTracerCache,
    pub metrics: Option<MetricsSignal>,
    pub logger_provider: Option<SdkLoggerProvider>,
    pub capture_policy: CapturePolicy,
}

pub struct MetricsSignal {
    pub meter_provider: SdkMeterProvider,
    pub instruments: MetricInstruments,
}

struct ProviderParts<'parts> {
    config: &'parts OpenTelemetryConfig,
    callback_name: Option<&'parts str>,
    resource: &'parts Resource,
    env: &'parts BTreeMap<String, String>,
    builder: &'parts LegacyExporterBuilder,
    globals: &'parts GlobalProviders,
    skip_set_global: bool,
}

fn init_tracing(
    parts: &ProviderParts<'_>,
    injected: Option<SdkTracerProvider>,
) -> Result<(SdkTracerProvider, SdkTracer), LegacyExporterError> {
    let provider = get_or_create_provider(
        &parts.globals.tracer,
        injected,
        parts.skip_set_global,
        || {
            let plan = span_processor_plan(parts.config, None, None, parts.callback_name)?;
            let processor = parts.builder.span_processor(&plan)?;
            Ok::<_, LegacyExporterError>(
                SdkTracerProvider::builder()
                    .with_resource(parts.resource.clone())
                    .with_span_processor(processor)
                    .build(),
            )
        },
    )?;
    let tracer = provider.tracer(legacy_tracer_name(parts.env));
    Ok((provider, tracer))
}

fn init_metrics(
    parts: &ProviderParts<'_>,
    injected: Option<SdkMeterProvider>,
) -> Result<MetricsSignal, LegacyExporterError> {
    let meter_provider = get_or_create_provider(
        &parts.globals.meter,
        injected,
        parts.skip_set_global,
        || {
            let plan = metric_reader_plan(parts.config, parts.callback_name)?;
            let reader = parts.builder.metric_reader(plan)?;
            Ok::<_, LegacyExporterError>(
                SdkMeterProvider::builder()
                    .with_reader(reader)
                    .with_resource(parts.resource.clone())
                    .build(),
            )
        },
    )?;
    let instruments = create_instruments(&meter_provider.meter(METER_NAME));
    Ok(MetricsSignal {
        meter_provider,
        instruments,
    })
}

fn new_tracer_cache(
    parts: &ProviderParts<'_>,
    max_providers: usize,
    environment: &LegacyOtelEnvironment<'_>,
) -> DynamicTracerCache {
    DynamicTracerCache::new(
        CacheSettings {
            config: parts.config.clone(),
            callback_name: parts.callback_name.map(str::to_owned),
            resource: parts.resource.clone(),
            tracer_name: legacy_tracer_name(parts.env),
            max_providers,
        },
        LegacyExporterBuilder::new(
            environment.clients.clone(),
            environment.shared_exporter.clone(),
        ),
    )
}

// Legacy logs always batch, whatever the exporter, unlike the v2 provider.
fn init_logs(
    parts: &ProviderParts<'_>,
    injected: Option<SdkLoggerProvider>,
) -> Result<Option<SdkLoggerProvider>, LegacyExporterError> {
    if !parts.config.signals.enable_events {
        return Ok(None);
    }
    let provider = get_or_create_provider(
        &parts.globals.logger,
        injected,
        parts.skip_set_global,
        || {
            let plan = log_exporter_plan(parts.config, parts.callback_name, parts.env)?;
            let sink = parts.builder.log_exporter(plan)?;
            Ok::<_, LegacyExporterError>(
                SdkLoggerProvider::builder()
                    .with_resource(parts.resource.clone())
                    .with_log_processor(BatchLogProcessor::builder(sink).build())
                    .build(),
            )
        },
    )?;
    Ok(Some(provider))
}

fn resolve_config(
    supplied: Option<OpenTelemetryConfig>,
    overrides: &MetadataKeyOverrides<'_>,
    env: &BTreeMap<String, String>,
) -> OpenTelemetryConfig {
    let mut config = supplied.unwrap_or_else(|| {
        let mut from_env = OpenTelemetryConfig::from_env(env);
        from_env.normalize(env);
        from_env
    });
    if let Some(keys) = overrides.team {
        config.baggage_team_metadata_keys = normalize_metadata_keys(keys);
    }
    if let Some(keys) = overrides.metadata {
        config.baggage_metadata_keys = normalize_metadata_keys(keys);
    }
    config
}

fn register_on_proxy_if_installed(proxy: Option<&ProxyRegistry>, callback_name: Option<String>) {
    if let Some(registry) = proxy {
        register_on_proxy(registry, &OtelRegistration { callback_name });
    } else {
        warn!("Proxy Server is not installed. Skipping OpenTelemetry initialization.");
    }
}

pub fn init_legacy_otel(
    init: LegacyOtelInit<'_>,
    environment: &LegacyOtelEnvironment<'_>,
) -> Result<LegacyOtel, LegacyExporterError> {
    let env = environment.env;
    let config = resolve_config(init.config, &init.overrides, env);
    let callback_name = init.callback_name;
    let resource = legacy_resource(&config, env);
    let builder = LegacyExporterBuilder::new(
        environment.clients.clone(),
        environment.shared_exporter.clone(),
    );
    let parts = ProviderParts {
        config: &config,
        callback_name: callback_name.as_deref(),
        resource: &resource,
        env,
        builder: &builder,
        globals: environment.globals,
        skip_set_global: config.skip_set_global || callback_name.as_deref() == Some(LANGFUSE_OTEL),
    };
    let (tracer_provider, tracer) = init_tracing(&parts, init.injected.tracer)?;
    let tracer_cache = new_tracer_cache(&parts, init.max_dynamic_tracer_providers, environment);
    let metrics = if config.signals.enable_metrics {
        Some(init_metrics(&parts, init.injected.meter)?)
    } else {
        None
    };
    let logger_provider = init_logs(&parts, init.injected.logger)?;
    register_on_proxy_if_installed(environment.proxy, callback_name.clone());
    Ok(LegacyOtel {
        capture_policy: CapturePolicy::from_init_state(&config, env),
        config,
        callback_name,
        resource,
        tracer_provider,
        tracer,
        tracer_cache,
        metrics,
        logger_provider,
    })
}

#[path = "legacy_otel_init/tests.rs"]
mod tests;
