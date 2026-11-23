#![cfg(test)]

mod support;

use std::sync::Arc;
use std::time::Duration;

use betterlite_redis::request_batch::lock;
use rust_decimal::Decimal;
use tokio::task::yield_now;
use tokio::time::advance;

use crate::reservation_admission::error::AdmissionError;
use crate::reservation_settlement::handle::get_reserved_counter_keys;

use super::error::ReservationRequestError;
use super::lease::{self, renewal_interval};
use super::reserve_budget_for_request;
use super::routes::{is_token_counting_route, is_unbilled_route};
use super::start_renewal;

use self::support::{
    AdmissionPlan, KEY, LeaseHarness, ROUTE, RequestFixture, World, amount, budget_counter, object,
    reservation_entry, reservation_handle, reservation_handle_without_keys,
};

// the TTL half of BaseCache's 60 second default
const TTL: i64 = 60;
const INTERVAL: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(6000);

#[test]
fn route_the_six_unbilled_routes_are_skipped() {
    for route in [
        "/models",
        "/v1/models",
        "/utils/token_counter",
        "/responses/input_tokens",
        "/v1/responses/input_tokens",
        "/openai/v1/responses/input_tokens",
    ] {
        assert!(is_unbilled_route(route), "{route}");
    }
}

#[test]
fn route_unbilled_matches_are_exact() {
    for route in [
        "/v1/models/gpt-4o",
        "/models/gpt-4o",
        "/V1/models",
        "/v1/models/",
        "/utils/token_counter/extra",
        "/v1/responses/input_tokens/extra",
        "/openai/v1/responses/input_tokens/x",
        "",
    ] {
        assert!(!is_unbilled_route(route), "{route}");
    }
}

#[test]
fn route_count_tokens_segments_are_skipped() {
    for route in [
        "/v1/messages/count_tokens",
        "/count_tokens",
        "count_tokens",
        "/a/b/count-tokens",
        "/v1/models/count_tokens:extra",
    ] {
        assert!(is_token_counting_route(route), "{route}");
    }
}

#[test]
fn route_count_tokens_action_is_skipped() {
    for route in [
        "/v1beta/models/gemini-pro:countTokens",
        "/models/x:countTokens",
        "/v1beta/models/a/b:countTokens",
    ] {
        assert!(is_token_counting_route(route), "{route}");
    }
}

#[test]
fn route_count_tokens_near_misses_are_billed() {
    for route in [
        "/v1beta/models/gemini-pro:countTokensX",
        "/v1/messages/count_tokens/extra",
        "/v1/messages/CountTokens",
        "/v1/messages/countTokens",
        "/v1beta/models/x:countTokens:extra",
    ] {
        assert!(!is_token_counting_route(route), "{route}");
    }
}

#[test]
fn route_a_token_counting_route_is_unbilled() {
    assert!(is_unbilled_route("/v1/messages/count_tokens"));
    assert!(is_unbilled_route("/v1beta/models/gemini-pro:countTokens"));
}

#[tokio::test]
async fn request_without_a_token_returns_none() {
    let world = World::new();
    let body = support::body();
    let request = RequestFixture::new(&body, ROUTE, &world)
        .without_token()
        .build();
    let outcome = reserve_budget_for_request(&request, &world.services(None))
        .await
        .expect("the reservation finishes");
    assert!(outcome.is_none());
    assert!(world.calls.names().is_empty());
}

#[tokio::test]
async fn request_route_outside_the_llm_api_returns_none() {
    let world = World::new();
    let body = support::body();
    let request = RequestFixture::new(&body, "/health", &world).build();
    let outcome = reserve_budget_for_request(&request, &world.services(None))
        .await
        .expect("the reservation finishes");
    assert!(outcome.is_none());
    assert!(world.calls.names().is_empty());
}

#[tokio::test]
async fn request_unbilled_route_returns_none() {
    let world = World::new();
    let body = support::body();
    let request = RequestFixture::new(&body, "/v1/models", &world).build();
    let outcome = reserve_budget_for_request(&request, &world.services(None))
        .await
        .expect("the reservation finishes");
    assert!(outcome.is_none());
    assert!(world.calls.names().is_empty());
}

#[tokio::test]
async fn request_without_a_model_returns_none() {
    let world = World::new();
    let body = object("{}");
    let request = RequestFixture::new(&body, ROUTE, &world).build();
    let outcome = reserve_budget_for_request(&request, &world.services(None))
        .await
        .expect("the reservation finishes");
    assert!(outcome.is_none());
    assert!(world.calls.names().is_empty());
}

#[tokio::test]
async fn request_without_counters_returns_none() {
    let mut world = World::new();
    world.counters.result = Vec::new();
    let body = support::body();
    let request = RequestFixture::new(&body, ROUTE, &world).build();
    let outcome = reserve_budget_for_request(&request, &world.services(None))
        .await
        .expect("the reservation finishes");
    assert!(outcome.is_none());
    assert_eq!(world.calls.names(), vec!["counters"]);
}

#[tokio::test]
async fn request_missing_estimate_returns_none() {
    let mut world = World::new();
    world.costs.max_cost = None;
    let body = support::body();
    let request = RequestFixture::new(&body, ROUTE, &world).build();
    let outcome = reserve_budget_for_request(&request, &world.services(None))
        .await
        .expect("the reservation finishes");
    assert!(outcome.is_none());
    assert_eq!(
        world.calls.names(),
        vec!["counters", "input_tokens", "max_cost"]
    );
}

#[tokio::test]
async fn request_non_positive_estimate_returns_none() {
    for estimate in [Decimal::ZERO, Decimal::NEGATIVE_ONE] {
        let mut world = World::new();
        world.costs.max_cost = Some(estimate);
        let body = support::body();
        let request = RequestFixture::new(&body, ROUTE, &world).build();
        let outcome = reserve_budget_for_request(&request, &world.services(None))
            .await
            .expect("the reservation finishes");
        assert!(outcome.is_none(), "{estimate}");
        assert_eq!(
            world.calls.names(),
            vec!["counters", "input_tokens", "max_cost"],
            "{estimate}"
        );
    }
}

#[tokio::test]
async fn request_error_releases_the_applied_entries() {
    let mut world = World::new();
    world.admission.plan = AdmissionPlan::Failed {
        entries: vec![(KEY.to_owned(), amount("7"))],
    };
    let body = support::body();
    let request = RequestFixture::new(&body, ROUTE, &world).build();
    let error = reserve_budget_for_request(&request, &world.services(None))
        .await
        .expect_err("the reservation fails");
    assert!(matches!(
        error,
        ReservationRequestError::Admission(AdmissionError::Overflow)
    ));
    let releases = world.settlement.releases.lock().expect("the release log");
    assert_eq!(releases.len(), 1);
    assert_eq!(releases[0].default_reserved_cost, amount("10"));
    assert_eq!(
        releases[0].entries,
        vec![(Some(KEY.to_owned()), Some(amount("7")))]
    );
    drop(releases);
    assert_eq!(
        world.calls.names(),
        vec![
            "counters",
            "input_tokens",
            "max_cost",
            "scope",
            "admit",
            "release"
        ]
    );
}

#[tokio::test]
async fn request_error_without_entries_still_releases() {
    let mut world = World::new();
    world.admission.plan = AdmissionPlan::Failed {
        entries: Vec::new(),
    };
    let body = support::body();
    let request = RequestFixture::new(&body, ROUTE, &world).build();
    let error = reserve_budget_for_request(&request, &world.services(None))
        .await
        .expect_err("the reservation fails");
    assert!(matches!(
        error,
        ReservationRequestError::Admission(AdmissionError::Overflow)
    ));
    let releases = world.settlement.releases.lock().expect("the release log");
    assert_eq!(releases.len(), 1);
    assert!(releases[0].entries.is_empty());
    assert_eq!(releases[0].default_reserved_cost, amount("10"));
}

#[tokio::test]
async fn request_without_applied_entries_returns_none() {
    let mut world = World::new();
    world.admission.plan = AdmissionPlan::Empty {
        reserved_cost: amount("10"),
    };
    let body = support::body();
    let request = RequestFixture::new(&body, ROUTE, &world).build();
    let outcome = reserve_budget_for_request(&request, &world.services(None))
        .await
        .expect("the reservation finishes");
    assert!(outcome.is_none());
    assert_eq!(
        world.calls.names(),
        vec!["counters", "input_tokens", "max_cost", "scope", "admit"]
    );
}

#[tokio::test]
async fn request_success_builds_the_record() {
    let world = World::new();
    let body = support::body();
    let request = RequestFixture::new(&body, ROUTE, &world).build();
    let handle = reserve_budget_for_request(&request, &world.services(None))
        .await
        .expect("the reservation finishes")
        .expect("a reservation");
    {
        let state = lock(&handle.0);
        assert_eq!(state.reserved_cost, amount("10"));
        assert_eq!(state.entries, vec![reservation_entry(KEY, amount("10"))]);
        assert!(!state.finalized);
        assert!(!state.callback_bound);
        assert_eq!(state.input_cost, amount("3"));
    }
    assert_eq!(
        world.calls.names(),
        vec![
            "counters",
            "input_tokens",
            "max_cost",
            "scope",
            "admit",
            "input_cost"
        ]
    );
    let scopes = world.settlement.scope_keys.lock().expect("the scope log");
    assert_eq!(scopes.len(), 1);
    assert!(scopes[0].contains(KEY));
}

#[tokio::test]
async fn request_input_cost_is_capped_at_the_reserved_cost() {
    let mut world = World::new();
    world.costs.input_cost = Some(amount("50"));
    let body = support::body();
    let request = RequestFixture::new(&body, ROUTE, &world).build();
    let handle = reserve_budget_for_request(&request, &world.services(None))
        .await
        .expect("the reservation finishes")
        .expect("a reservation");
    assert_eq!(lock(&handle.0).input_cost, amount("10"));
}

#[tokio::test]
async fn request_resized_admission_cost_is_the_record_cost() {
    let mut world = World::new();
    world.admission.plan = AdmissionPlan::Applied {
        entries: vec![(KEY.to_owned(), amount("7"))],
        reserved_cost: amount("7"),
    };
    world.costs.input_cost = Some(amount("9"));
    let body = support::body();
    let request = RequestFixture::new(&body, ROUTE, &world).build();
    let handle = reserve_budget_for_request(&request, &world.services(None))
        .await
        .expect("the reservation finishes")
        .expect("a reservation");
    let state = lock(&handle.0);
    assert_eq!(state.reserved_cost, amount("7"));
    assert_eq!(state.entries, vec![reservation_entry(KEY, amount("7"))]);
    assert_eq!(state.input_cost, amount("7"));
}

#[tokio::test]
async fn request_missing_input_cost_is_zero() {
    let mut world = World::new();
    world.costs.input_cost = None;
    let body = support::body();
    let request = RequestFixture::new(&body, ROUTE, &world).build();
    let handle = reserve_budget_for_request(&request, &world.services(None))
        .await
        .expect("the reservation finishes")
        .expect("a reservation");
    assert_eq!(lock(&handle.0).input_cost, Decimal::ZERO);
}

#[tokio::test]
async fn request_admission_receives_the_configured_policy() {
    let world = World::new();
    let body = support::body();
    let request = RequestFixture::new(&body, ROUTE, &world)
        .fail_closed()
        .build();
    let mut services = world.services(None);
    services.budget_exceeded_status = 429;
    services.throttle_percentage = Some(0.25);
    let _handle = reserve_budget_for_request(&request, &services)
        .await
        .expect("the reservation finishes")
        .expect("a reservation");
    let seen = world.admission.seen.lock().expect("the admission log");
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].reservation_cost, amount("10"));
    assert!(seen[0].fail_closed_budget_enforcement);
    assert_eq!(seen[0].budget_exceeded_status, 429);
    assert_eq!(seen[0].throttle_percentage, Some(0.25));
    assert_eq!(seen[0].counters, 1);
    assert!(seen[0].has_token);
}

#[tokio::test]
async fn request_counter_input_carries_the_request_facts() {
    let world = World::new();
    let body = support::body();
    let tags = vec!["team-a".to_owned(), "team-b".to_owned()];
    let request = RequestFixture::new(&body, ROUTE, &world)
        .with_tag_names(&tags)
        .apply_team_keys()
        .build();
    let _handle = reserve_budget_for_request(&request, &world.services(None))
        .await
        .expect("the reservation finishes")
        .expect("a reservation");
    let seen = world.counters.seen.lock().expect("the counter log");
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].tag_names, tags);
    assert!(seen[0].apply_user_budget_to_team_keys);
    assert!(seen[0].token_present);
}

#[tokio::test(start_paused = true)]
async fn lease_successful_reservation_starts_the_renewal() {
    let world = World::new();
    let harness = LeaseHarness::new();
    let body = support::body();
    let request = RequestFixture::new(&body, ROUTE, &world).build();
    let renewal = harness.renewal(TTL, REQUEST_TIMEOUT);
    let handle = reserve_budget_for_request(&request, &world.services(Some(renewal)))
        .await
        .expect("the reservation finishes")
        .expect("a reservation");
    yield_now().await;
    assert!(!harness.tracker.is_empty());
    advance(INTERVAL).await;
    yield_now().await;
    assert_eq!(harness.refreshed(), vec![KEY.to_owned()]);
    drop(handle);
}

#[tokio::test(start_paused = true)]
async fn lease_does_not_start_without_a_redis_backend() {
    let world = World::new();
    let harness = LeaseHarness::new();
    let handle = reservation_handle(&[KEY]);
    let services = world.services(None);
    start_renewal(&handle, &services);
    yield_now().await;
    assert!(harness.tracker.is_empty());
    advance(INTERVAL).await;
    yield_now().await;
    assert!(harness.refreshed().is_empty());
}

#[tokio::test(start_paused = true)]
async fn lease_does_not_start_without_counter_keys() {
    let world = World::new();
    let harness = LeaseHarness::new();
    let handle = reservation_handle_without_keys();
    let renewal = harness.renewal(TTL, REQUEST_TIMEOUT);
    let services = world.services(Some(renewal));
    start_renewal(&handle, &services);
    yield_now().await;
    assert!(harness.tracker.is_empty());
}

#[test]
fn lease_renewal_interval_halves_the_redis_ttl() {
    assert_eq!(renewal_interval(60), Some(Duration::from_secs(30)));
    assert_eq!(renewal_interval(21), Some(Duration::from_millis(10_500)));
    assert_eq!(renewal_interval(1), Some(Duration::from_millis(500)));
    assert_eq!(renewal_interval(0), None);
    assert_eq!(renewal_interval(-60), None);
    assert_eq!(renewal_interval(i64::MAX), None);
}

#[tokio::test(start_paused = true)]
async fn lease_zero_ttl_does_not_start_a_renewal() {
    let world = World::new();
    let harness = LeaseHarness::new();
    let handle = reservation_handle(&[KEY]);
    let renewal = harness.renewal(0, REQUEST_TIMEOUT);
    let services = world.services(Some(renewal));
    start_renewal(&handle, &services);
    yield_now().await;
    assert!(harness.tracker.is_empty());
    advance(INTERVAL).await;
    yield_now().await;
    assert!(harness.refreshed().is_empty());
}

#[tokio::test(start_paused = true)]
async fn lease_renews_each_key_every_interval() {
    let harness = LeaseHarness::new();
    let handle = reservation_handle(&["spend:key:a", "spend:key:b"]);
    let renewal = harness.renewal(TTL, REQUEST_TIMEOUT);
    lease::start(
        &renewal,
        Arc::downgrade(&handle.0),
        get_reserved_counter_keys(&handle),
    );
    yield_now().await;
    assert!(harness.refreshed().is_empty());
    advance(INTERVAL).await;
    yield_now().await;
    assert_eq!(
        harness.refreshed(),
        vec!["spend:key:a".to_owned(), "spend:key:b".to_owned()]
    );
    advance(INTERVAL).await;
    yield_now().await;
    assert_eq!(harness.refreshed().len(), 4);
}

#[tokio::test(start_paused = true)]
async fn lease_does_not_renew_before_the_first_interval() {
    let harness = LeaseHarness::new();
    let handle = reservation_handle(&[KEY]);
    let renewal = harness.renewal(TTL, REQUEST_TIMEOUT);
    lease::start(
        &renewal,
        Arc::downgrade(&handle.0),
        get_reserved_counter_keys(&handle),
    );
    yield_now().await;
    advance(Duration::from_secs(29)).await;
    yield_now().await;
    assert!(harness.refreshed().is_empty());
    advance(Duration::from_secs(1)).await;
    yield_now().await;
    assert_eq!(harness.refreshed(), vec![KEY.to_owned()]);
}

#[tokio::test(start_paused = true)]
async fn lease_stops_after_finalization() {
    let harness = LeaseHarness::new();
    let handle = reservation_handle(&[KEY]);
    let renewal = harness.renewal(TTL, REQUEST_TIMEOUT);
    lease::start(
        &renewal,
        Arc::downgrade(&handle.0),
        get_reserved_counter_keys(&handle),
    );
    yield_now().await;
    lock(&handle.0).finalized = true;
    advance(INTERVAL).await;
    yield_now().await;
    assert!(harness.refreshed().is_empty());
    assert!(harness.tracker.is_empty());
}

#[tokio::test(start_paused = true)]
async fn lease_stops_when_the_record_is_dropped() {
    let harness = LeaseHarness::new();
    let handle = reservation_handle(&[KEY]);
    let renewal = harness.renewal(TTL, REQUEST_TIMEOUT);
    lease::start(
        &renewal,
        Arc::downgrade(&handle.0),
        get_reserved_counter_keys(&handle),
    );
    yield_now().await;
    drop(handle);
    advance(INTERVAL).await;
    yield_now().await;
    assert!(harness.refreshed().is_empty());
    assert!(harness.tracker.is_empty());
}

#[tokio::test(start_paused = true)]
async fn lease_stops_at_the_request_timeout() {
    let harness = LeaseHarness::new();
    let handle = reservation_handle(&[KEY]);
    let renewal = harness.renewal(TTL, Duration::from_secs(90));
    lease::start(
        &renewal,
        Arc::downgrade(&handle.0),
        get_reserved_counter_keys(&handle),
    );
    yield_now().await;
    for _ in 0..3 {
        advance(INTERVAL).await;
        yield_now().await;
    }
    assert_eq!(
        harness.refreshed(),
        vec![KEY.to_owned(), KEY.to_owned(), KEY.to_owned()]
    );
    assert!(harness.tracker.is_empty());
    advance(INTERVAL).await;
    yield_now().await;
    assert_eq!(harness.refreshed().len(), 3);
}

#[tokio::test(start_paused = true)]
async fn lease_timeout_too_large_does_not_renew() {
    let harness = LeaseHarness::new();
    let handle = reservation_handle(&[KEY]);
    let renewal = harness.renewal(TTL, Duration::MAX);
    lease::start(
        &renewal,
        Arc::downgrade(&handle.0),
        get_reserved_counter_keys(&handle),
    );
    yield_now().await;
    assert!(harness.tracker.is_empty());
    advance(INTERVAL).await;
    yield_now().await;
    assert!(harness.refreshed().is_empty());
}

#[tokio::test]
async fn request_uses_the_counter_key_set_for_the_batch_scope() {
    let mut world = World::new();
    world.counters.result = vec![
        budget_counter(KEY, "100"),
        budget_counter("spend:key:tok2", "50"),
    ];
    let body = support::body();
    let request = RequestFixture::new(&body, ROUTE, &world).build();
    let _handle = reserve_budget_for_request(&request, &world.services(None))
        .await
        .expect("the reservation finishes")
        .expect("a reservation");
    let scopes = world.settlement.scope_keys.lock().expect("the scope log");
    assert_eq!(scopes.len(), 1);
    assert_eq!(scopes[0].len(), 2);
    assert!(scopes[0].contains("spend:key:tok2"));
}
