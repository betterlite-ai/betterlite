use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::otel_settings::Settings as OtelConfig;
use opentelemetry::InstrumentationScope;
use opentelemetry::KeyValue;
use opentelemetry::logs::LoggerProvider as _;
use opentelemetry::metrics::{Meter, MeterProvider as _};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::logs::{
    BatchLogProcessor, SdkLogger, SdkLoggerProvider, SimpleLogProcessor,
};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};

use crate::otel::plumbing::exporters::{ExporterError, HttpClients, log_sink, metric_sink};
use crate::otel::plumbing::sinks::{LogSink, MetricSink};

pub const METRIC_EXPORT_INTERVAL: Duration = Duration::from_secs(5);
const SCOPE_VERSION: &str = env!("CARGO_PKG_VERSION");

// What the process already has for a signal: an operator's provider, an explicit opt-out, or nothing.
#[derive(Debug, Clone)]
pub enum ProviderState<T> {
    Unset,
    Configured(T),
    OptedOut,
}

// Publish means the caller installs the freshly built provider as the process default.
#[derive(Debug, Clone)]
pub enum Resolved<T> {
    Reuse(T),
    Publish(T),
    Disabled,
}

#[must_use]
pub fn build_resource(config: &OtelConfig) -> Resource {
    let mut attributes = BTreeMap::new();
    attributes.insert("service.name".to_owned(), config.service_name.clone());
    if let Some(environment) = config
        .deployment_environment
        .as_ref()
        .filter(|name| !name.is_empty())
    {
        attributes.insert("deployment.environment".to_owned(), environment.clone());
    }
    attributes.extend(config.resource_attributes.clone());
    Resource::builder()
        .with_attributes(
            attributes
                .into_iter()
                .map(|(name, value)| KeyValue::new(name, value)),
        )
        .build()
}

// Histograms keep the SDK's cumulative temporality: Prometheus-backed receivers reject delta histograms.
pub fn build_metric_reader(
    config: &OtelConfig,
    clients: &HttpClients,
) -> Result<PeriodicReader<MetricSink>, ExporterError> {
    Ok(PeriodicReader::builder(metric_sink(config, clients)?)
        .with_interval(METRIC_EXPORT_INTERVAL)
        .build())
}

#[must_use]
pub fn build_meter_provider(
    config: &OtelConfig,
    reader: PeriodicReader<MetricSink>,
) -> SdkMeterProvider {
    SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(build_resource(config))
        .build()
}

// Console and in-memory logs export synchronously, everything else batches, as for spans.
#[must_use]
pub fn build_logger_provider(config: &OtelConfig, sink: LogSink) -> SdkLoggerProvider {
    let builder = SdkLoggerProvider::builder().with_resource(build_resource(config));
    if matches!(sink, LogSink::Console(_) | LogSink::InMemory(_)) {
        builder
            .with_log_processor(SimpleLogProcessor::new(sink))
            .build()
    } else {
        builder
            .with_log_processor(BatchLogProcessor::builder(sink).build())
            .build()
    }
}

// An injected provider wins, then the operator's, and only an unset process gets one built from config.
fn resolve_provider<T, F>(
    injected: Option<T>,
    existing: ProviderState<T>,
    build: F,
) -> Result<Resolved<T>, ExporterError>
where
    F: FnOnce() -> Result<T, ExporterError>,
{
    if let Some(provider) = injected {
        return Ok(Resolved::Reuse(provider));
    }
    match existing {
        ProviderState::Configured(provider) => Ok(Resolved::Reuse(provider)),
        ProviderState::OptedOut => Ok(Resolved::Disabled),
        ProviderState::Unset => build().map(Resolved::Publish),
    }
}

// Both resolvers share one signature and differ only in how the provider is built from config.
macro_rules! provider_resolver {
    ($name:ident, $provider:ty, $build:expr) => {
        pub fn $name(
            config: &OtelConfig,
            clients: &HttpClients,
            injected: Option<$provider>,
            existing: ProviderState<$provider>,
        ) -> Result<Resolved<$provider>, ExporterError> {
            resolve_provider(injected, existing, || $build(config, clients))
        }
    };
}

provider_resolver!(
    resolve_meter_provider,
    SdkMeterProvider,
    |config, clients| {
        Ok(build_meter_provider(
            config,
            build_metric_reader(config, clients)?,
        ))
    }
);

provider_resolver!(
    resolve_logger_provider,
    SdkLoggerProvider,
    |config, clients| { Ok(build_logger_provider(config, log_sink(config, clients)?)) }
);

#[must_use]
pub fn get_meter(provider: &SdkMeterProvider, name: &'static str) -> Meter {
    let scope = InstrumentationScope::builder(name)
        .with_version(SCOPE_VERSION)
        .build();
    provider.meter_with_scope(scope)
}

#[must_use]
pub fn get_event_logger(provider: &SdkLoggerProvider, name: &'static str) -> SdkLogger {
    let scope = InstrumentationScope::builder(name)
        .with_version(SCOPE_VERSION)
        .build();
    provider.logger_with_scope(scope)
}

#[path = "providers/tests.rs"]
mod tests;
