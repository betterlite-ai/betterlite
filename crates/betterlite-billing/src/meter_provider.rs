use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::billing_metrics::Metering;
use betterlite_http::otlp_client::{OtlpClient, OtlpClientError};
use opentelemetry::KeyValue;
use opentelemetry_otlp::{
    ExporterBuildError, MetricExporter, WithExportConfig as _, WithHttpConfig as _,
};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use rustls::crypto::ring;
use rustls::pki_types::pem::{self, PemObject as _};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ClientConfig, RootCertStore};
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
    #[error("could not read PEM from {}: {source}", path.display())]
    Pem {
        path: PathBuf,
        #[source]
        source: pem::Error,
    },
    #[error("{} holds no certificate", path.display())]
    NoCertificate { path: PathBuf },
    #[error("no trusted CA certificate is available to verify the collector")]
    NoTrustRoots,
    #[error("rejected the CA certificate in {}: {source}", path.display())]
    Authority {
        path: PathBuf,
        #[source]
        source: rustls::Error,
    },
    #[error("could not set up mutual TLS: {source}")]
    Tls {
        #[source]
        source: rustls::Error,
    },
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
) -> Result<SdkMeterProvider, ExporterError> {
    let metering = &config.metering;
    let interval_ms = metering.export_interval_ms;
    let interval = u64::try_from(interval_ms)
        .ok()
        .filter(|millis| *millis > 0)
        .map(Duration::from_millis)
        .ok_or(ExporterError::Interval { interval_ms })?;
    let client = OtlpClient::build(tls_config(metering)?)
        .map_err(|source| ExporterError::Client { source })?;
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

fn tls_config(metering: &Metering) -> Result<ClientConfig, ExporterError> {
    let certificates = certificates(&metering.client_cert)?;
    if certificates.is_empty() {
        return Err(ExporterError::NoCertificate {
            path: metering.client_cert.clone(),
        });
    }
    let key = PrivateKeyDer::from_pem_file(&metering.client_key).map_err(|source| {
        ExporterError::Pem {
            path: metering.client_key.clone(),
            source,
        }
    })?;
    let roots = trust_roots(metering.ca_cert.as_deref())?;
    ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|source| ExporterError::Tls { source })?
        .with_root_certificates(roots)
        .with_client_auth_cert(certificates, key)
        .map_err(|source| ExporterError::Tls { source })
}

fn certificates(path: &Path) -> Result<Vec<CertificateDer<'static>>, ExporterError> {
    let unreadable = |source| ExporterError::Pem {
        path: path.to_path_buf(),
        source,
    };
    CertificateDer::pem_file_iter(path)
        .map_err(unreadable)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(unreadable)
}

fn trust_roots(authority: Option<&Path>) -> Result<RootCertStore, ExporterError> {
    let mut roots = RootCertStore::empty();
    if let Some(path) = authority {
        for certificate in certificates(path)? {
            roots
                .add(certificate)
                .map_err(|source| ExporterError::Authority {
                    path: path.to_path_buf(),
                    source,
                })?;
        }
    } else {
        let native = rustls_native_certs::load_native_certs();
        for error in native.errors {
            tracing::warn!("Billing metrics: skipped part of the system trust store: {error}");
        }
        roots.add_parsable_certificates(native.certs);
    }
    if roots.is_empty() {
        return Err(ExporterError::NoTrustRoots);
    }
    Ok(roots)
}
