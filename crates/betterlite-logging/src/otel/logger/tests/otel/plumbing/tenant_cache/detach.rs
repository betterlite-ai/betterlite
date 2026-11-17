use std::collections::BTreeMap;

use crate::otel::support::{
    Built, cache, creds, default_tracer, destination, owned_http, phoenix_http,
};

#[test]
fn detach_is_true_for_credential_and_project_routes() {
    let built = Built::default();
    let tenants = cache(owned_http(), Some("arize"), &built);
    let default = default_tracer();
    let credential = tenants
        .route_for(&default, Some(&creds("S")), None, &[])
        .expect("route");
    assert!(credential.detached);
    let project_built = Built::default();
    let projects = cache(phoenix_http(), Some("arize_phoenix"), &project_built);
    let auth = BTreeMap::from([("phoenix_project_name".to_owned(), "proj".to_owned())]);
    let project = projects
        .route_for(&default, None, Some(&auth), &[])
        .expect("route");
    assert!(project.detached);
}

#[test]
fn detach_is_false_for_a_service_name_only_route() {
    let built = Built::default();
    let tenants = cache(owned_http(), Some("arize"), &built);
    let default = default_tracer();
    let auth = BTreeMap::from([("otel_service_name".to_owned(), "payments".to_owned())]);
    let route = tenants
        .route_for(&default, None, Some(&auth), &[])
        .expect("route");
    assert!(route.provider.is_some());
    assert!(!route.detached);
}

#[test]
fn detach_is_false_when_the_callback_already_has_a_destination() {
    let built = Built::default();
    let tenants = cache(owned_http(), Some("arize"), &built);
    let default = default_tracer();
    let destinations = vec![destination("arize")];
    let route = tenants
        .route_for(&default, Some(&creds("S")), None, &destinations)
        .expect("route");
    // the request already names a backend for this callback, so the tenants' route is bypassed
    assert!(route.provider.is_none());
    assert!(!route.detached);
    assert_eq!(built.builds(), 0);
}
