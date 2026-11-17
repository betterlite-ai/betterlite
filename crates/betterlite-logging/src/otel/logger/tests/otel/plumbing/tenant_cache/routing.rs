use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_logging::otel::config::ExporterOwner;
use betterlite_logging::otel::plumbing::routing::routed::tenant_service_name;

use crate::otel::support::{
    Built, cache, creds, default_tracer, destination, owned_http, phoenix_http, spec, text_params,
    with_exporters,
};

#[test]
fn routing_keeps_a_service_name_route_unrouted_while_a_destination_applies() {
    let built = Built::default();
    let tenants = cache(owned_http(), Some("arize"), &built);
    let default = default_tracer();
    let auth = BTreeMap::from([("otel_service_name".to_owned(), "payments".to_owned())]);
    let destinations = vec![destination("langfuse_otel")];
    let route = tenants
        .route_for(&default, None, Some(&auth), &destinations)
        .expect("route");
    // a request-level destination already picks the backend, so the service name stays unrouted
    assert!(route.provider.is_none());
    assert_eq!(built.builds(), 0);
}

#[test]
fn routing_caches_one_provider_per_credential_set() {
    let built = Built::default();
    let tenants = cache(owned_http(), Some("arize"), &built);
    let default = default_tracer();
    let first = tenants
        .route_for(&default, Some(&creds("S1")), None, &[])
        .expect("route");
    let again = tenants
        .route_for(&default, Some(&creds("S1")), None, &[])
        .expect("route");
    let other = tenants
        .route_for(&default, Some(&creds("S2")), None, &[])
        .expect("route");
    assert!(first.detached);
    assert!(other.detached);
    assert_eq!(built.builds(), 2);
    assert!(Arc::ptr_eq(
        first.provider.as_ref().expect("first"),
        again.provider.as_ref().expect("again")
    ));
    assert!(!Arc::ptr_eq(
        first.provider.as_ref().expect("first"),
        other.provider.as_ref().expect("other")
    ));
}

#[test]
fn routing_returns_the_default_tracer_without_a_tenant_signal() {
    let built = Built::default();
    let tenants = cache(owned_http(), Some("arize"), &built);
    let default = default_tracer();
    let route = tenants.route_for(&default, None, None, &[]).expect("route");
    assert!(route.provider.is_none());
    assert!(!route.detached);
    assert_eq!(built.builds(), 0);
}

#[test]
fn routing_skips_a_blank_service_name() {
    let built = Built::default();
    let tenants = cache(owned_http(), Some("arize"), &built);
    let default = default_tracer();
    let auth = BTreeMap::from([("otel_service_name".to_owned(), "  ".to_owned())]);
    let route = tenants
        .route_for(&default, None, Some(&auth), &[])
        .expect("route");
    // whitespace names no tenant, so the call stays on the default backend
    assert!(route.provider.is_none());
    assert!(!route.detached);
    assert_eq!(built.builds(), 0);
}

#[test]
fn routing_keeps_one_provider_per_project() {
    let built = Built::default();
    let tenants = cache(phoenix_http(), Some("arize_phoenix"), &built);
    let default = default_tracer();
    let first = BTreeMap::from([("phoenix_project_name".to_owned(), "a".to_owned())]);
    let second = BTreeMap::from([("phoenix_project_name".to_owned(), "b".to_owned())]);
    let one = tenants
        .route_for(&default, None, Some(&first), &[])
        .expect("route");
    let two = tenants
        .route_for(&default, None, Some(&second), &[])
        .expect("route");
    assert_eq!(built.builds(), 2);
    assert!(!Arc::ptr_eq(
        one.provider.as_ref().expect("one"),
        two.provider.as_ref().expect("two")
    ));
}

#[test]
fn routing_keeps_a_credentialed_tenant_while_another_callbacks_destination_applies() {
    let built = Built::default();
    let tenants = cache(owned_http(), Some("arize"), &built);
    let default = default_tracer();
    let destinations = vec![destination("langfuse_otel")];
    let route = tenants
        .route_for(&default, Some(&creds("S")), None, &destinations)
        .expect("route");
    // the destination names another callback, so the tenant's own credentials still route
    assert!(route.provider.is_some());
    assert!(route.detached);
    assert_eq!(built.builds(), 1);
}

#[test]
fn routing_keeps_one_provider_per_service_name() {
    let built = Built::default();
    let tenants = cache(owned_http(), Some("arize"), &built);
    let default = default_tracer();
    let payments = BTreeMap::from([("otel_service_name".to_owned(), "payments".to_owned())]);
    let search = BTreeMap::from([("otel_service_name".to_owned(), "search".to_owned())]);
    let one = tenants
        .route_for(&default, None, Some(&payments), &[])
        .expect("route");
    let two = tenants
        .route_for(&default, None, Some(&search), &[])
        .expect("route");
    // two tenants that differ only in otel_service_name keep their own service.name
    assert_eq!(built.builds(), 2);
    assert!(!Arc::ptr_eq(
        one.provider.as_ref().expect("payments"),
        two.provider.as_ref().expect("search")
    ));
}

#[test]
fn routing_keeps_one_provider_per_newrelic_region() {
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
    let us = text_params(&[("newrelic_api_key", "K"), ("newrelic_region", "us")]);
    let eu = text_params(&[("newrelic_api_key", "K"), ("newrelic_region", "eu")]);
    let one = tenants
        .route_for(&default, Some(&us), None, &[])
        .expect("route");
    let two = tenants
        .route_for(&default, Some(&eu), None, &[])
        .expect("route");
    // the same key in two regions exports to each region's endpoint, so the endpoint stays in the key
    assert_eq!(built.builds(), 2);
    assert!(!Arc::ptr_eq(
        one.provider.as_ref().expect("us"),
        two.provider.as_ref().expect("eu")
    ));
}

#[test]
fn routing_prefers_the_service_name_override_key() {
    let auth = BTreeMap::from([
        (
            "otel_service_name_override".to_owned(),
            "payments".to_owned(),
        ),
        ("otel_service_name".to_owned(), "other".to_owned()),
    ]);
    // OTEL_SERVICE_NAME_METADATA_KEYS reads the override first
    assert_eq!(
        tenant_service_name(Some(&auth)).as_deref(),
        Some("payments")
    );
}
