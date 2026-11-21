use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::otel_settings::FeatureFlag;
use betterlite_config::types::Config;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::FixedOffset;
use opentelemetry_sdk::trace::SdkTracerProvider;

use crate::otel::config::Settings;
use crate::otel::content_projections::StreamOutputAssembler;
use crate::otel::emitter::span_emission::DEFAULT_SPAN_ATTRIBUTE_LIMIT;
use crate::otel::fan_out::attach::TenantFanOutRegistry;
use crate::otel::legacy_metrics::filter::CallbackSettingsView;
use crate::otel::logger_runtime::LoggerRuntime;
use crate::otel::logger_runtime::construction::CallbackConstructor;
use crate::otel::plumbing::exporters::{ExporterFactories, HttpClients};
use crate::otel::v2_callback::registry::V2Registry;

use super::error::ProxyRuntimeError;
use super::sources::ProxyTraceSources;

const SPAN_ATTRIBUTE_COUNT_LIMIT: &str = "OTEL_SPAN_ATTRIBUTE_COUNT_LIMIT";
const ATTRIBUTE_COUNT_LIMIT: &str = "OTEL_ATTRIBUTE_COUNT_LIMIT";

pub struct ProxyRuntimeInputs<'config> {
    pub config: &'config Config,
    pub environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    pub http_clients: HttpClients,
    pub callback_settings: Arc<dyn CallbackSettingsView>,
    pub configured_exclusions: Option<Vec<String>>,
    pub assembler: Arc<dyn StreamOutputAssembler>,
    pub utc_offset: FixedOffset,
    pub sources: Arc<ProxyTraceSources>,
}

pub fn build_proxy_runtime(
    inputs: ProxyRuntimeInputs<'_>,
) -> Result<Arc<LoggerRuntime>, ProxyRuntimeError> {
    let base = Settings::from_config(inputs.config)?;
    let v2_enabled = FeatureFlag::default().enabled(inputs.environment.as_ref())?;
    let attribute_limit = span_attribute_limit(inputs.environment.as_ref())?;
    let constructor = Arc::new(callback_constructor(&inputs, attribute_limit)?);
    Ok(Arc::new(assemble_runtime(
        inputs,
        base,
        v2_enabled,
        constructor,
    )))
}

fn callback_constructor(
    inputs: &ProxyRuntimeInputs<'_>,
    attribute_limit: Option<usize>,
) -> Result<CallbackConstructor, ProxyRuntimeError> {
    let roots = Arc::clone(&inputs.sources);
    Ok(CallbackConstructor {
        raw_config: inputs.config.sections.otel_settings.clone(),
        callback_settings: Arc::clone(&inputs.callback_settings),
        allowed_hosts: Arc::from(
            inputs
                .config
                .sections
                .provider_url_destination
                .allowed_hosts
                .clone(),
        ),
        roots,
        attribute_limit,
        redactor: Some(Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?)),
        log_client_error_tracebacks: false,
        utc_offset: inputs.utc_offset,
    })
}

fn assemble_runtime(
    inputs: ProxyRuntimeInputs<'_>,
    base: Settings,
    v2_enabled: bool,
    constructor: Arc<CallbackConstructor>,
) -> LoggerRuntime {
    let registered = Arc::new(V2Registry::default());
    let hooks = Arc::clone(&registered);
    let trace = Arc::clone(&inputs.sources);
    let scope = Arc::clone(&inputs.sources);
    LoggerRuntime {
        registered,
        loggers: Mutex::new(Vec::new()),
        fan_out: Arc::new(Mutex::new(TenantFanOutRegistry::default())),
        constructor,
        base,
        environment: inputs.environment,
        trace,
        factories: Arc::new(ExporterFactories::new(inputs.http_clients, BTreeMap::new())),
        scope,
        assembler: inputs.assembler,
        hooks: Some(hooks),
        configured_exclusions: inputs.configured_exclusions,
        v2_enabled,
        sdk_provider: SdkTracerProvider::builder().build(),
        generic: Mutex::new(None),
        published: Mutex::new(None),
    }
}

// litellm.integrations.otel.emitter.span_attribute_limit: the SDK's SpanLimits resolves the span count from its own variable, then the global one, else 128
fn span_attribute_limit(
    environment: &dyn EnvironmentLookup,
) -> Result<Option<usize>, ProxyRuntimeError> {
    let global = match environment.lookup(ATTRIBUTE_COUNT_LIMIT) {
        Some(value) => read_attribute_limit(ATTRIBUTE_COUNT_LIMIT, &value)?
            .unwrap_or(DEFAULT_SPAN_ATTRIBUTE_LIMIT),
        None => DEFAULT_SPAN_ATTRIBUTE_LIMIT,
    };
    environment
        .lookup(SPAN_ATTRIBUTE_COUNT_LIMIT)
        .map_or(Ok(Some(global)), |value| {
            read_attribute_limit(SPAN_ATTRIBUTE_COUNT_LIMIT, &value)
        })
}

// the SDK reads a stripped value; an empty one is unlimited and anything else must parse as a non-negative integer
fn read_attribute_limit(
    variable: &'static str,
    value: &str,
) -> Result<Option<usize>, ProxyRuntimeError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let Ok(limit) = trimmed.parse::<usize>() else {
        return Err(ProxyRuntimeError::AttributeLimit { variable });
    };
    Ok(Some(limit))
}
