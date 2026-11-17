use std::sync::Arc;

use betterlite_logging::otel::plumbing::routing::cache::{
    MAX_CACHED_PROVIDERS, MAX_RETIRED_PROVIDERS, TenantTracerCache,
};
use opentelemetry_sdk::trace::SdkTracer;

use crate::otel::support::{Built, cache, creds, default_tracer, owned_http};

fn route_spaces(tenants: &TenantTracerCache, default: &SdkTracer, from: usize, to: usize) {
    for index in from..=to {
        let space = format!("S{index}");
        tenants
            .route_for(default, Some(&creds(&space)), None, &[])
            .expect("route");
    }
}

#[test]
fn eviction_drains_an_evicted_provider_until_its_last_lease_releases() {
    let built = Built::default();
    let tenants = cache(owned_http(), Some("arize"), &built);
    let default = default_tracer();
    let first = tenants
        .route_for(&default, Some(&creds("S0")), None, &[])
        .expect("route");
    route_spaces(&tenants, &default, 1, MAX_CACHED_PROVIDERS);
    assert_eq!(built.builds(), MAX_CACHED_PROVIDERS.saturating_add(1));
    // S0 was the oldest and still carried its route's lease: it drains rather than shutting down
    let drained = built.sink(0);
    assert!(!drained.is_shutdown_called());
    tenants.release(first.provider.as_ref());
    assert!(drained.is_shutdown_called());
}

#[test]
fn eviction_shuts_down_a_provider_with_no_open_spans_at_once() {
    let built = Built::default();
    let tenants = cache(owned_http(), Some("arize"), &built);
    let default = default_tracer();
    let first = tenants
        .route_for(&default, Some(&creds("S0")), None, &[])
        .expect("route");
    tenants.release(first.provider.as_ref());
    route_spaces(&tenants, &default, 1, MAX_CACHED_PROVIDERS);
    // S0 had no open spans left, so the overflow eviction shuts it down inside route_for
    assert!(built.sink(0).is_shutdown_called());
}

#[test]
fn eviction_keeps_a_retired_provider_until_its_last_lease_releases() {
    let built = Built::default();
    let tenants = cache(owned_http(), Some("arize"), &built);
    let default = default_tracer();
    let first = tenants
        .route_for(&default, Some(&creds("S0")), None, &[])
        .expect("route");
    tenants
        .route_for(&default, Some(&creds("S0")), None, &[])
        .expect("route");
    route_spaces(&tenants, &default, 1, MAX_CACHED_PROVIDERS);
    let drained = built.sink(0);
    // S0 carries two leases, so the overflow retires it rather than shutting it down
    assert!(!drained.is_shutdown_called());
    tenants.release(first.provider.as_ref());
    assert!(!drained.is_shutdown_called());
    tenants.release(first.provider.as_ref());
    assert!(drained.is_shutdown_called());
}

#[test]
fn eviction_shuts_down_the_stalest_retiree_past_the_retired_cap() {
    let built = Built::default();
    let tenants = cache(owned_http(), Some("arize"), &built);
    let default = default_tracer();
    let total = MAX_CACHED_PROVIDERS
        .saturating_add(MAX_RETIRED_PROVIDERS)
        .saturating_add(1);
    route_spaces(&tenants, &default, 0, total.saturating_sub(1));
    assert_eq!(built.builds(), total);
    // every provider here is still leased, so each eviction retires; the 65th retiree sheds the stalest one
    assert!(built.sink(0).is_shutdown_called());
}

#[test]
fn eviction_waits_for_the_first_overflow_before_shedding() {
    let built = Built::default();
    let tenants = cache(owned_http(), Some("arize"), &built);
    let default = default_tracer();
    route_spaces(
        &tenants,
        &default,
        0,
        MAX_CACHED_PROVIDERS.saturating_sub(1),
    );
    let oldest = tenants
        .route_for(&default, Some(&creds("S0")), None, &[])
        .expect("route");
    // at exactly the cap nothing is shed yet, so the oldest route is still cached and builds nothing
    assert_eq!(built.builds(), MAX_CACHED_PROVIDERS);
    assert!(oldest.provider.is_some());
}

#[test]
fn eviction_keeps_the_newest_provider_cached() {
    let built = Built::default();
    let tenants = cache(owned_http(), Some("arize"), &built);
    let default = default_tracer();
    route_spaces(
        &tenants,
        &default,
        0,
        MAX_CACHED_PROVIDERS.saturating_sub(1),
    );
    let newest = tenants
        .route_for(&default, Some(&creds("S256")), None, &[])
        .expect("route");
    let builds = built.builds();
    let cached = tenants
        .route_for(&default, Some(&creds("S256")), None, &[])
        .expect("route");
    assert_eq!(built.builds(), builds);
    assert!(Arc::ptr_eq(
        newest.provider.as_ref().expect("newest"),
        cached.provider.as_ref().expect("provider")
    ));
}
