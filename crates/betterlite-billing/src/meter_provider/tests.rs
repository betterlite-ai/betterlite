#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_config::sections::billing_metrics::Metering;
use opentelemetry::{Key, Value};
use rstest::rstest;
use tokio::runtime::Builder;

use super::{BillingMetricsConfig, ExporterError, build_mtls_meter_provider, resource};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn config(metering: Metering) -> BillingMetricsConfig {
    BillingMetricsConfig {
        metering,
        litellm_version: "1.105.1".to_owned(),
        license_id: Some("license-7".to_owned()),
    }
}

fn metering(authority: Option<PathBuf>) -> Metering {
    Metering {
        endpoint: "https://localhost:9/v1/metrics".to_owned(),
        client_cert: fixture("client.crt"),
        client_key: fixture("client.key"),
        ca_cert: authority,
        export_interval_ms: 60_000,
    }
}

#[rstest]
#[case::licensed(Some("license-7"), Some("license-7"))]
#[case::unlicensed(None, None)]
#[case::empty(Some(""), None)]
fn resource_names_the_deployment(#[case] license_id: Option<&str>, #[case] expected: Option<&str>) {
    let built = resource(&BillingMetricsConfig {
        metering: metering(None),
        litellm_version: "1.105.1".to_owned(),
        license_id: license_id.map(str::to_owned),
    });
    assert_eq!(
        built.get(&Key::new("service.name")),
        Some(Value::from("litellm-proxy"))
    );
    assert_eq!(
        built.get(&Key::new("litellm.version")),
        Some(Value::from("1.105.1"))
    );
    assert_eq!(
        built.get(&Key::new("litellm.license.id")),
        expected.map(|license| Value::from(license.to_owned()))
    );
}

#[rstest]
#[case::zero(0)]
#[case::negative(-1)]
fn rejects_intervals_below_one_millisecond(#[case] interval_ms: i64) {
    let mut settings = metering(Some(fixture("ca.crt")));
    settings.export_interval_ms = interval_ms;
    let error = build_mtls_meter_provider(&config(settings)).unwrap_err();
    assert!(
        matches!(error, ExporterError::Interval { interval_ms: rejected } if rejected == interval_ms)
    );
    assert_eq!(
        error.to_string(),
        format!("interval value {interval_ms} is invalid and needs to be larger than zero")
    );
}

#[test]
fn rejects_a_certificate_file_without_certificates() {
    let mut settings = metering(Some(fixture("ca.crt")));
    settings.client_cert = fixture("client.key");
    let error = build_mtls_meter_provider(&config(settings)).unwrap_err();
    assert!(
        matches!(error, ExporterError::NoCertificate { path } if path == fixture("client.key"))
    );
}

#[test]
fn rejects_a_key_file_without_a_key() {
    let mut settings = metering(Some(fixture("ca.crt")));
    settings.client_key = fixture("client.crt");
    let error = build_mtls_meter_provider(&config(settings)).unwrap_err();
    assert!(matches!(error, ExporterError::Pem { path, .. } if path == fixture("client.crt")));
}

#[test]
fn rejects_a_mismatched_key() {
    let mut settings = metering(Some(fixture("ca.crt")));
    settings.client_key = fixture("server.key");
    let error = build_mtls_meter_provider(&config(settings)).unwrap_err();
    assert!(matches!(error, ExporterError::Tls { .. }), "{error}");
}

#[test]
fn rejects_a_ca_file_without_certificates() {
    let directory = tempfile::tempdir().unwrap();
    let empty = directory.path().join("empty.pem");
    fs::write(&empty, "").unwrap();
    let error = build_mtls_meter_provider(&config(metering(Some(empty)))).unwrap_err();
    assert!(matches!(error, ExporterError::NoTrustRoots), "{error}");
}

#[test]
fn rejects_an_endpoint_that_is_not_a_url() {
    let mut settings = metering(Some(fixture("ca.crt")));
    settings.endpoint = "not a url/v1/metrics".to_owned();
    let error = build_mtls_meter_provider(&config(settings)).unwrap_err();
    assert!(matches!(error, ExporterError::Exporter { .. }), "{error}");
}

#[test]
fn builds_with_the_system_trust_store() {
    let provider = build_mtls_meter_provider(&config(metering(None))).unwrap();
    provider.shutdown().unwrap();
}

#[test]
fn builds_inside_a_tokio_runtime() {
    let runtime = Builder::new_multi_thread().build().unwrap();
    let built = runtime
        .block_on(async { build_mtls_meter_provider(&config(metering(Some(fixture("ca.crt"))))) });
    built.unwrap().shutdown().unwrap();
}
