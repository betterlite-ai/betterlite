use std::collections::BTreeMap;

use betterlite_logging::otel::config::ExporterOwner;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt as _;

use crate::otel::support::{
    Built, Warnings, cache, creds, default_tracer, spec, text_params, with_exporters,
};

#[test]
fn transport_keeps_credentials_off_a_headerless_exporter() {
    let built = Built::default();
    let tenants = cache(
        with_exporters(vec![spec(
            "in_memory",
            Some(ExporterOwner::ArizeAx),
            None,
            None,
        )]),
        Some("arize"),
        &built,
    );
    let default = default_tracer();
    let route = tenants
        .route_for(&default, Some(&creds("S")), None, &[])
        .expect("route");
    // nowhere to stamp them, so the span stays on the default backend rather than the console
    assert!(route.provider.is_none());
    assert!(!route.detached);
    assert_eq!(built.builds(), 0);
}

#[tokio::test]
async fn transport_routes_credentials_over_grpc() {
    let built = Built::default();
    let tenants = cache(
        with_exporters(vec![spec(
            "otlp_grpc",
            Some(ExporterOwner::ArizeAx),
            None,
            Some("http://localhost:4317"),
        )]),
        Some("arize"),
        &built,
    );
    let default = default_tracer();
    let route = tenants
        .route_for(&default, Some(&creds("S")), None, &[])
        .expect("route");
    assert!(route.detached);
    assert!(route.provider.is_some());
}

#[tokio::test]
async fn transport_keeps_project_headers_off_grpc() {
    let built = Built::default();
    let tenants = cache(
        with_exporters(vec![spec(
            "otlp_grpc",
            Some(ExporterOwner::ArizePhoenix),
            None,
            Some("http://localhost:4317"),
        )]),
        Some("arize_phoenix"),
        &built,
    );
    let default = default_tracer();
    let auth = BTreeMap::from([("phoenix_project_name".to_owned(), "proj".to_owned())]);
    let route = tenants
        .route_for(&default, None, Some(&auth), &[])
        .expect("route");
    assert!(route.provider.is_none());
    assert!(!route.detached);
    assert_eq!(built.builds(), 0);
}

#[test]
fn transport_warns_once_about_credential_routing_without_an_otlp_exporter() {
    let built = Built::default();
    let tenants = cache(
        with_exporters(vec![spec(
            "in_memory",
            Some(ExporterOwner::ArizeAx),
            None,
            None,
        )]),
        Some("arize"),
        &built,
    );
    let default = default_tracer();
    let warnings = Warnings::default();
    let subscriber = tracing_subscriber::registry().with(warnings.clone());
    let _guard = set_default(subscriber);
    tenants
        .route_for(&default, Some(&creds("S")), None, &[])
        .expect("route");
    assert_eq!(warnings.messages().len(), 1);
    tenants
        .route_for(&default, Some(&creds("S")), None, &[])
        .expect("route");
    assert_eq!(warnings.messages().len(), 1);
}

#[test]
fn transport_warns_once_about_project_routing_without_http() {
    let built = Built::default();
    let tenants = cache(
        with_exporters(vec![spec(
            "otlp_grpc",
            Some(ExporterOwner::ArizePhoenix),
            None,
            Some("http://localhost:4317"),
        )]),
        Some("arize_phoenix"),
        &built,
    );
    let default = default_tracer();
    let auth = BTreeMap::from([("phoenix_project_name".to_owned(), "proj".to_owned())]);
    let warnings = Warnings::default();
    let subscriber = tracing_subscriber::registry().with(warnings.clone());
    let _guard = set_default(subscriber);
    tenants
        .route_for(&default, None, Some(&auth), &[])
        .expect("route");
    assert_eq!(warnings.messages().len(), 1);
    tenants
        .route_for(&default, None, Some(&auth), &[])
        .expect("route");
    assert_eq!(warnings.messages().len(), 1);
}

#[test]
fn transport_keeps_another_callbacks_exporter_untouched() {
    let built = Built::default();
    let tenants = cache(
        with_exporters(vec![spec(
            "otlp_http",
            Some(ExporterOwner::ArizePhoenix),
            Some("space=static"),
            Some("http://phoenix:6006"),
        )]),
        Some("arize"),
        &built,
    );
    let default = default_tracer();
    let route = tenants
        .route_for(&default, Some(&creds("S")), None, &[])
        .expect("route");
    // the only exporter belongs to arize_phoenix, so an arize credential has nowhere to go
    assert!(route.provider.is_none());
    assert_eq!(built.builds(), 0);
}

#[test]
fn transport_routes_a_newrelic_tenant_endpoint() {
    let built = Built::default();
    let tenants = cache(
        with_exporters(vec![spec(
            "otlp_http",
            Some(ExporterOwner::Newrelic),
            None,
            Some("http://static:4318"),
        )]),
        Some("newrelic"),
        &built,
    );
    let default = default_tracer();
    let dynamic = text_params(&[("newrelic_api_key", "K")]);
    let route = tenants
        .route_for(&default, Some(&dynamic), None, &[])
        .expect("route");
    assert!(route.detached);
    let spec = built.specs().first().cloned().expect("exporter");
    assert_eq!(spec.endpoint.as_deref(), Some("https://otlp.nr-data.net"));
}
