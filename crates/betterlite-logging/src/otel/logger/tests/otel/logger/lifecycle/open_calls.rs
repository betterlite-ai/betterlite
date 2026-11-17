use std::time::{Duration, SystemTime};

use betterlite_logging::otel::logger::v2::{OPEN_CALLS_MAX, OpenCall, OpenCalls};
use betterlite_logging::otel::plumbing::routing::cache::MAX_CACHED_PROVIDERS;
use indexmap::IndexMap;

use crate::otel::support::{
    PreCall, Tenant, anchored, creds, event_with, finished, harness, inference, owned_http,
    payload_start, pre_call, route_tenants, success, with_id,
};

fn fill_open_calls(calls: &mut OpenCalls, start: SystemTime) {
    for index in 0..OPEN_CALLS_MAX {
        let at = start
            .checked_add(Duration::from_secs(u64::try_from(index).expect("index")))
            .expect("carrier time");
        assert!(
            calls
                .store(
                    format!("call-{index}"),
                    OpenCall {
                        boundary: None,
                        start_time: at,
                        provider: None,
                    },
                )
                .is_none()
        );
    }
}

#[test]
fn open_calls_caps_the_map_and_evicts_the_oldest() {
    let mut calls = OpenCalls::default();
    let start = SystemTime::now();
    fill_open_calls(&mut calls, start);
    let evicted = calls
        .store(
            "overflow".to_owned(),
            OpenCall {
                boundary: None,
                start_time: start,
                provider: None,
            },
        )
        .expect("evicted carrier");
    assert_eq!(evicted.start_time, start);
    assert!(!calls.calls.contains_key("call-0"));
    assert!(calls.calls.contains_key("overflow"));
    // removing is the dedup between the success and failure paths
    assert!(calls.calls.shift_remove("call-1").is_some());
    assert!(calls.calls.shift_remove("call-1").is_none());
}

#[test]
fn eviction_releases_the_evicted_call_carriers_route() {
    let harness = harness(owned_http(), anchored());
    pre_call(
        &harness.logger,
        PreCall {
            call_id: "call-oldest",
            model: "gpt-4o",
            no_upstream: false,
            dynamic: Some(creds("S0")),
            auth: None,
        },
    );
    route_tenants(
        &harness,
        "call-",
        1,
        MAX_CACHED_PROVIDERS,
        Tenant::Credentials(|index| format!("S{index}")),
    );
    let drained = harness.built.sink(0);
    // S0's provider left the cache, but the oldest carrier still holds the route's lease
    assert!(!drained.is_shutdown_called());
    let stored = MAX_CACHED_PROVIDERS.saturating_add(1);
    // every filler carrier reuses the newest cached tenant, so only the carrier eviction can drain S0's retired provider
    route_tenants(
        &harness,
        "later-",
        0,
        OPEN_CALLS_MAX.saturating_sub(stored).saturating_add(1),
        Tenant::Credentials(|_| format!("S{MAX_CACHED_PROVIDERS}")),
    );
    // evicting that carrier released its lease, and the drained provider shut down
    assert!(drained.is_shutdown_called());
}

#[tokio::test]
async fn llm_close_releases_the_carriers_route_lease() {
    let harness = harness(owned_http(), anchored());
    pre_call(
        &harness.logger,
        PreCall {
            call_id: "call-a",
            model: "gpt-4o",
            no_upstream: false,
            dynamic: Some(creds("S0")),
            auth: None,
        },
    );
    success(
        &harness.logger,
        event_with(with_id(inference(), "call-a"), creds("S0"), IndexMap::new()),
    )
    .await;
    route_tenants(
        &harness,
        "later-",
        1,
        MAX_CACHED_PROVIDERS,
        Tenant::Credentials(|index| format!("S{index}")),
    );
    // the close released S0's lease, so the overflow eviction shuts its provider down at once
    assert!(harness.built.sink(0).is_shutdown_called());
}

#[tokio::test]
async fn overflow_eviction_still_emits_the_evicted_call_deferred() {
    let harness = harness(owned_http(), anchored());
    route_tenants(
        &harness,
        "call-",
        0,
        OPEN_CALLS_MAX.saturating_add(1),
        Tenant::Credentials(|_| "S".to_owned()),
    );
    assert_eq!(harness.built.builds(), 1);
    success(
        &harness.logger,
        event_with(with_id(inference(), "call-0"), creds("S"), IndexMap::new()),
    )
    .await;
    success(
        &harness.logger,
        event_with(with_id(inference(), "call-1"), creds("S"), IndexMap::new()),
    )
    .await;
    let spans = finished(&harness.built.sink(0));
    assert_eq!(spans.len(), 2);
    // call-0 lost its carrier: its span is created at close from the payload's own start
    assert_eq!(
        spans.first().expect("deferred span").start_time,
        payload_start()
    );
    // call-1 kept its carrier: its span was born at the pre_call boundary
    assert_ne!(spans.get(1).expect("live span").start_time, payload_start());
}
