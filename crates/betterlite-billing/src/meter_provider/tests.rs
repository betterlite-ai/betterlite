#![cfg(test)]

use std::path::PathBuf;

use betterlite_config::sections::billing_metrics::Metering;
use opentelemetry::{Key, Value};
use rstest::rstest;
use tokio::runtime::Builder;

use super::{BillingMetricsConfig, ExporterError, build_mtls_meter_provider, resource};
use crate::tests::{direct_policy, fixture};

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
    let error = build_mtls_meter_provider(&config(settings), &direct_policy()).unwrap_err();
    assert!(
        matches!(error, ExporterError::Interval { interval_ms: rejected } if rejected == interval_ms)
    );
    assert_eq!(
        error.to_string(),
        format!("interval value {interval_ms} is invalid and needs to be larger than zero")
    );
}

#[rstest]
#[case::no_certificate(["client.key", "client.key", "ca.crt"], "https://c", "no certificate")]
#[case::no_key(["client.crt", "client.crt", "ca.crt"], "https://c", "could not read PEM")]
#[case::wrong_key(["client.crt", "server.key", "ca.crt"], "https://c", "mutual TLS")]
#[case::no_authority(["client.crt", "client.key", "client.key"], "https://c", "no trusted CA")]
#[case::not_a_url(["client.crt", "client.key", "ca.crt"], "not a url", "OTLP exporter")]
fn rejects_unusable_credentials(
    #[case] files: [&str; 3],
    #[case] endpoint: &str,
    #[case] message: &str,
) {
    let [cert, key, authority] = files;
    let settings = Metering {
        endpoint: endpoint.to_owned(),
        client_cert: fixture(cert),
        client_key: fixture(key),
        ca_cert: Some(fixture(authority)),
        export_interval_ms: 60_000,
    };
    let error = build_mtls_meter_provider(&config(settings), &direct_policy()).unwrap_err();
    assert!(error.to_string().contains(message), "{error}");
}

#[test]
fn builds_with_the_system_trust_store() {
    let provider = build_mtls_meter_provider(&config(metering(None)), &direct_policy()).unwrap();
    provider.shutdown().unwrap();
}

#[test]
fn builds_inside_a_tokio_runtime() {
    let runtime = Builder::new_multi_thread().build().unwrap();
    let built = runtime.block_on(async {
        build_mtls_meter_provider(&config(metering(Some(fixture("ca.crt")))), &direct_policy())
    });
    built.unwrap().shutdown().unwrap();
}
