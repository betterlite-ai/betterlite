use std::collections::BTreeMap;

use betterlite_logging::otel::config::ExporterOwner;
use betterlite_logging::otel::config::destination::SpanScope;
use betterlite_logging::otel::plumbing::routing::routed::{RouteKey, TenantRouting as _};

use crate::otel::support::{Built, cache, spec, with_exporters};

fn credentials() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("api_key".to_owned(), "K".to_owned()),
        ("arize-space-id".to_owned(), "S".to_owned()),
    ])
}

#[test]
fn headers_replace_the_static_credentials_of_an_owned_exporter() {
    let built = Built::default();
    let tenants = cache(
        with_exporters(vec![spec(
            "otlp_http",
            Some(ExporterOwner::ArizeAx),
            Some("space=static"),
            None,
        )]),
        Some("arize"),
        &built,
    );
    let routed = tenants.routed_config(&credentials(), &BTreeMap::new(), None, None);
    assert_eq!(
        routed
            .exporters
            .first()
            .expect("exporter")
            .headers
            .as_deref(),
        Some("api_key=K,arize-space-id=S")
    );
}

#[test]
fn headers_append_percent_encoded_project_headers() {
    let built = Built::default();
    let tenants = cache(
        with_exporters(vec![spec(
            "otlp_http",
            Some(ExporterOwner::ArizePhoenix),
            Some("Authorization=Bearer k"),
            Some("http://phoenix:6006"),
        )]),
        Some("arize_phoenix"),
        &built,
    );
    let project = BTreeMap::from([("x-project-name".to_owned(), "my proj, prod=1".to_owned())]);
    let routed = tenants.routed_config(&BTreeMap::new(), &project, None, None);
    assert_eq!(
        routed
            .exporters
            .first()
            .expect("exporter")
            .headers
            .as_deref(),
        Some("Authorization=Bearer k,x-project-name=my%20proj%2C%20prod%3D1")
    );
}

#[test]
fn headers_keep_another_owners_exporter_untouched() {
    let built = Built::default();
    let tenants = cache(
        with_exporters(vec![
            spec(
                "otlp_http",
                Some(ExporterOwner::ArizeAx),
                Some("space=base"),
                None,
            ),
            spec("otlp_http", None, Some("x=collector"), None),
        ]),
        Some("arize"),
        &built,
    );
    let routed = tenants.routed_config(&credentials(), &BTreeMap::new(), None, None);
    assert_eq!(
        routed
            .exporters
            .first()
            .expect("exporter")
            .headers
            .as_deref(),
        Some("api_key=K,arize-space-id=S")
    );
    assert_eq!(
        routed
            .exporters
            .get(1)
            .expect("collector")
            .headers
            .as_deref(),
        Some("x=collector")
    );
}

#[test]
fn headers_keep_project_headers_off_grpc_but_still_replace_credentials() {
    let built = Built::default();
    let tenants = cache(
        with_exporters(vec![spec(
            "otlp_grpc",
            Some(ExporterOwner::ArizePhoenix),
            Some("space=base"),
            Some("http://localhost:4317"),
        )]),
        Some("arize_phoenix"),
        &built,
    );
    let project = BTreeMap::from([("x-project-name".to_owned(), "proj".to_owned())]);
    let routed = tenants.routed_config(&BTreeMap::new(), &project, None, None);
    assert_eq!(
        routed
            .exporters
            .first()
            .expect("exporter")
            .headers
            .as_deref(),
        Some("space=base")
    );
    let credentialed = tenants.routed_config(&credentials(), &BTreeMap::new(), None, None);
    assert_eq!(
        credentialed
            .exporters
            .first()
            .expect("exporter")
            .headers
            .as_deref(),
        Some("api_key=K,arize-space-id=S")
    );
}

#[test]
fn headers_skip_a_headerless_exporter() {
    let built = Built::default();
    let tenants = cache(
        with_exporters(vec![spec(
            "in_memory",
            Some(ExporterOwner::ArizeAx),
            Some("space=base"),
            None,
        )]),
        Some("arize"),
        &built,
    );
    let routed = tenants.routed_config(&credentials(), &BTreeMap::new(), None, None);
    assert_eq!(
        routed
            .exporters
            .first()
            .expect("exporter")
            .headers
            .as_deref(),
        Some("space=base")
    );
}

#[test]
fn headers_keep_credentials_out_of_the_route_key_debug() {
    let credentials = BTreeMap::from([("api_key".to_owned(), "secret".to_owned())]);
    let key = RouteKey {
        credential_headers: credentials.into_iter().collect(),
        project_headers: Vec::new(),
        endpoint: None,
        service_name: None,
    };
    let rendered = format!("{key:?}");
    // the header value is a collector credential, so Debug prints only the header name
    assert!(rendered.contains("api_key"));
    assert!(!rendered.contains("secret"));
}

#[test]
fn headers_apply_the_tenant_endpoint_and_service_name() {
    let mut routed_settings = with_exporters(vec![spec(
        "otlp_http",
        Some(ExporterOwner::ArizeAx),
        None,
        Some("http://static:4318"),
    )]);
    routed_settings.langfuse_span_scope = SpanScope::LlmOnly;
    let built = Built::default();
    let tenants = cache(routed_settings, Some("arize"), &built);
    let routed = tenants.routed_config(
        &BTreeMap::new(),
        &BTreeMap::new(),
        Some("http://tenant:4318"),
        Some("payments"),
    );
    assert_eq!(routed.service_name, "payments");
    // a routed config always exports full spans, whatever the static setting said
    assert_eq!(routed.langfuse_span_scope, SpanScope::Full);
    assert_eq!(
        routed
            .exporters
            .first()
            .expect("exporter")
            .endpoint
            .as_deref(),
        Some("http://tenant:4318")
    );
}
