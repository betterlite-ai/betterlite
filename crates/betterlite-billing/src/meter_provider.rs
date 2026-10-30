use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::billing_metrics::Metering;
use betterlite_http::otlp_client::{OtlpClient, OtlpClientError};
use opentelemetry::KeyValue;
use opentelemetry_otlp::{
    ExporterBuildError, MetricExporter, WithExportConfig as _, WithHttpConfig as _,
};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use thiserror::Error;

mod tests;

pub const SERVICE_NAME: &str = "litellm-proxy";
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BillingMetricsConfig {
    pub metering: Metering,
    pub litellm_version: String,
    pub license_id: Option<String>,
}

#[derive(Debug, Error)]
pub enum ExporterError {
    #[error("interval value {interval_ms} is invalid and needs to be larger than zero")]
    Interval { interval_ms: i64 },
    #[error("could not build the HTTP client: {source}")]
    Client {
        #[source]
        source: OtlpClientError,
    },
    #[error("could not build the OTLP exporter: {source}")]
    Exporter {
        #[source]
        source: ExporterBuildError,
    },
}

#[must_use]
pub fn resource(config: &BillingMetricsConfig) -> Resource {
    let license = config
        .license_id
        .as_deref()
        .filter(|license_id| !license_id.is_empty())
        .map(|license_id| KeyValue::new("litellm.license.id", license_id.to_owned()));
    Resource::builder()
        .with_service_name(SERVICE_NAME)
        .with_attribute(KeyValue::new(
            "litellm.version",
            config.litellm_version.clone(),
        ))
        .with_attributes(license)
        .build()
}

pub fn build_mtls_meter_provider(
    config: &BillingMetricsConfig,
    policy: &Arc<ProxyPolicy>,
) -> Result<SdkMeterProvider, ExporterError> {
    let metering = &config.metering;
    let interval_ms = metering.export_interval_ms;
    let interval = u64::try_from(interval_ms)
        .ok()
        .filter(|millis| *millis > 0)
        .map(Duration::from_millis)
        .ok_or(ExporterError::Interval { interval_ms })?;
    let client =
        OtlpClient::build(metering, policy).map_err(|source| ExporterError::Client { source })?;
    let exporter = MetricExporter::builder()
        .with_http()
        .with_endpoint(metering.endpoint.clone())
        .with_http_client(client)
        .build()
        .map_err(|source| ExporterError::Exporter { source })?;
    let reader = PeriodicReader::builder(exporter)
        .with_interval(interval)
        .build();
    Ok(SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(resource(config))
        .build())
}
