use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use crate::routing_read_batch::{ReadTiers, RequestReads, RoutingPrefetch};
use crate::routing_value_store::RouteValue;

use super::support::{
    FakeStore, TestClock, counter, deployment, first, last, number, plain, reads, rpm_window, seed,
    selection, service, tiers, tpm_window, usage_tier, values,
};

type CaseFuture = Pin<Box<dyn Future<Output = ()>>>;
type Case = fn() -> CaseFuture;

#[rustfmt::skip]
const CASES: &[(&str, Case)] = &[
    ("lowest_absolute_tpm_wins", || Box::pin(async { lowest_absolute_tpm_wins() })),
    ("a_tie_keeps_pool_order", || Box::pin(async { a_tie_keeps_pool_order() })),
    ("a_missing_counter_counts_as_zero", || Box::pin(async { a_missing_counter_counts_as_zero() })),
    ("an_explicit_zero_limit_admits_no_tokens", || Box::pin(async { an_explicit_zero_limit_admits_no_tokens() })),
    ("the_tpm_boundary_excludes_the_sum_past_the_limit", || Box::pin(async { the_tpm_boundary_excludes_the_sum_past_the_limit() })),
    ("the_rpm_boundary_rejects_the_request_that_reaches_it", || Box::pin(async { the_rpm_boundary_rejects_the_request_that_reaches_it() })),
    ("an_unread_rpm_counter_does_not_screen", || Box::pin(async { an_unread_rpm_counter_does_not_screen() })),
    ("the_limit_folds_from_the_deployment_first", || Box::pin(async { the_limit_folds_from_the_deployment_first() })),
    ("the_limit_folds_through_litellm_params", || Box::pin(async { the_limit_folds_through_litellm_params() })),
    ("a_null_limit_falls_through", || Box::pin(async { a_null_limit_falls_through() })),
    ("a_limit_from_model_info_is_used", || Box::pin(async { a_limit_from_model_info_is_used() })),
    ("a_repeated_id_keeps_its_place_and_its_last_deployment", || Box::pin(async { a_repeated_id_keeps_its_place_and_its_last_deployment() })),
    ("an_id_with_a_colon_keeps_its_counter", || Box::pin(an_id_with_a_colon_keeps_its_counter())),
    ("a_value_of_another_kind_is_unknown", || Box::pin(async { a_value_of_another_kind_is_unknown() })),
    ("a_failed_read_answers_the_cold_pick", || Box::pin(a_failed_read_answers_the_cold_pick())),
    ("the_async_facade_reads_the_pool_in_one_batch", || Box::pin(the_async_facade_reads_the_pool_in_one_batch())),
    ("the_async_facade_uses_the_request_prefetch", || Box::pin(the_async_facade_uses_the_request_prefetch())),
    ("the_sync_facade_reads_the_local_tier", || Box::pin(async { the_sync_facade_reads_the_local_tier() })),
    ("the_sync_facade_without_a_tier_sees_no_counters", || Box::pin(async { the_sync_facade_without_a_tier_sees_no_counters() })),
    ("the_async_facade_without_a_usage_tier_sees_no_counters", || Box::pin(the_async_facade_without_a_usage_tier_sees_no_counters())),
    ("the_counter_read_returns_after_the_local_entry_expires", || Box::pin(the_counter_read_returns_after_the_local_entry_expires())),
];

pub(super) fn run(name: &str) -> CaseFuture {
    let Some((_, case)) = CASES.iter().find(|(case_name, _)| *case_name == name) else {
        panic!("unknown case {name}");
    };
    (*case)()
}

fn lowest_absolute_tpm_wins() {
    let pool = [plain("a"), plain("b"), plain("c")];
    let tpm = values(&[Some(7), Some(2), Some(9)]);
    let rpm = values(&[None, None, None]);
    let picked = selection(&pool, Some(&tpm), Some(&rpm), 0)
        .choose(&mut first())
        .unwrap();
    assert_eq!(picked, Some(1));
}

fn a_tie_keeps_pool_order() {
    let pool = [plain("a"), plain("b"), plain("c")];
    let tpm = values(&[Some(3), Some(3), Some(3)]);
    let rpm = values(&[None, None, None]);
    assert_eq!(
        selection(&pool, Some(&tpm), Some(&rpm), 0)
            .choose(&mut first())
            .unwrap(),
        Some(0)
    );
    assert_eq!(
        selection(&pool, Some(&tpm), Some(&rpm), 0)
            .choose(&mut last())
            .unwrap(),
        Some(2)
    );
}

fn a_missing_counter_counts_as_zero() {
    let pool = [plain("a"), plain("b")];
    let tpm = values(&[None, Some(1)]);
    let rpm = values(&[None, None]);
    let picked = selection(&pool, Some(&tpm), Some(&rpm), 0)
        .choose(&mut last())
        .unwrap();
    assert_eq!(picked, Some(0));
}

fn an_explicit_zero_limit_admits_no_tokens() {
    let pool = [deployment("a", &[("tpm", number(0))], &[], &[]), plain("b")];
    let tpm = values(&[Some(0), Some(5)]);
    let rpm = values(&[None, None]);
    assert_eq!(
        selection(&pool, Some(&tpm), Some(&rpm), 0)
            .choose(&mut first())
            .unwrap(),
        Some(0)
    );
    assert_eq!(
        selection(&pool, Some(&tpm), Some(&rpm), 1)
            .choose(&mut first())
            .unwrap(),
        Some(1)
    );
}

fn the_tpm_boundary_excludes_the_sum_past_the_limit() {
    let pool = [
        deployment("a", &[("tpm", number(10))], &[], &[]),
        plain("b"),
    ];
    let tpm = values(&[Some(4), Some(9)]);
    let rpm = values(&[None, None]);
    assert_eq!(
        selection(&pool, Some(&tpm), Some(&rpm), 6)
            .choose(&mut first())
            .unwrap(),
        Some(0)
    );
    assert_eq!(
        selection(&pool, Some(&tpm), Some(&rpm), 7)
            .choose(&mut first())
            .unwrap(),
        Some(1)
    );
}

fn the_rpm_boundary_rejects_the_request_that_reaches_it() {
    let pool = [deployment("a", &[("rpm", number(2))], &[], &[]), plain("b")];
    let tpm = values(&[Some(0), Some(5)]);
    let rpm = values(&[Some(1), None]);
    assert_eq!(
        selection(&pool, Some(&tpm), Some(&rpm), 0)
            .choose(&mut first())
            .unwrap(),
        Some(1)
    );
    let higher = [deployment("a", &[("rpm", number(3))], &[], &[]), plain("b")];
    assert_eq!(
        selection(&higher, Some(&tpm), Some(&rpm), 0)
            .choose(&mut first())
            .unwrap(),
        Some(0)
    );
}

fn an_unread_rpm_counter_does_not_screen() {
    let pool = [deployment("a", &[("rpm", number(1))], &[], &[]), plain("b")];
    let tpm = values(&[Some(0), Some(5)]);
    let rpm = values(&[None, None]);
    let picked = selection(&pool, Some(&tpm), Some(&rpm), 0)
        .choose(&mut first())
        .unwrap();
    assert_eq!(picked, Some(0));
}

fn the_limit_folds_from_the_deployment_first() {
    let pool = [
        deployment(
            "a",
            &[("tpm", number(1))],
            &[("tpm", number(1))],
            &[("tpm", number(100))],
        ),
        plain("b"),
    ];
    let tpm = values(&[Some(5), Some(9)]);
    let rpm = values(&[None, None]);
    let picked = selection(&pool, Some(&tpm), Some(&rpm), 0)
        .choose(&mut first())
        .unwrap();
    assert_eq!(picked, Some(0));
}

fn the_limit_folds_through_litellm_params() {
    let pool = [
        deployment("a", &[("rpm", number(2))], &[("rpm", number(100))], &[]),
        plain("b"),
    ];
    let tpm = values(&[None, None]);
    let rpm = values(&[Some(1), None]);
    // the litellm_params rpm of 2 screens a out; the model_info 100 would have admitted it
    assert_eq!(
        selection(&pool, Some(&tpm), Some(&rpm), 0)
            .choose(&mut first())
            .unwrap(),
        Some(1)
    );
}

fn a_null_limit_falls_through() {
    // the null falls through to the litellm_params tpm of 4, which screens a; stopping at the null keeps it
    let pool = [
        deployment("a", &[("tpm", number(4))], &[], &[("tpm", Json::Null)]),
        plain("b"),
    ];
    let tpm = values(&[Some(5), Some(9)]);
    let rpm = values(&[None, None]);
    let picked = selection(&pool, Some(&tpm), Some(&rpm), 0)
        .choose(&mut first())
        .unwrap();
    assert_eq!(picked, Some(1));
}

fn a_limit_from_model_info_is_used() {
    let pool = [deployment("a", &[], &[("tpm", number(4))], &[]), plain("b")];
    let tpm = values(&[Some(5), Some(9)]);
    let rpm = values(&[None, None]);
    assert_eq!(
        selection(&pool, Some(&tpm), Some(&rpm), 0)
            .choose(&mut first())
            .unwrap(),
        Some(1)
    );
    let roomy = [
        deployment("a", &[], &[("tpm", number(40))], &[]),
        plain("b"),
    ];
    assert_eq!(
        selection(&roomy, Some(&tpm), Some(&rpm), 0)
            .choose(&mut first())
            .unwrap(),
        Some(0)
    );
}

fn a_repeated_id_keeps_its_place_and_its_last_deployment() {
    let pool = [
        plain("a"),
        deployment("a", &[("tpm", number(1))], &[], &[]),
        plain("b"),
    ];
    let tpm = values(&[Some(0), Some(8), Some(5)]);
    let rpm = values(&[None, None, None]);
    assert_eq!(
        selection(&pool, Some(&tpm), Some(&rpm), 0)
            .choose(&mut first())
            .unwrap(),
        Some(2)
    );
    // the pinned deployment_dict[id] overwrites in place: the first place keeps the last deployment's data
    let reported = selection(&pool, Some(&tpm), Some(&rpm), 0)
        .no_deployment()
        .unwrap();
    let ids: Vec<&str> = reported
        .deployments
        .iter()
        .map(|load| load.id.as_str())
        .collect();
    assert_eq!(ids, vec!["a", "b"]);
    assert_eq!(reported.deployments[0].current_tpm, Some(8));
    assert_eq!(reported.deployments[0].tpm_limit, Some(Decimal::ONE));
}

// LiteLLM splits a counter key on ':' to find its deployment, so an id holding one loses its counter there.
async fn an_id_with_a_colon_keeps_its_counter() {
    let pool = [plain("a:b"), plain("c")];
    let store = FakeStore::new(&[
        (tpm_window("a:b").as_str(), 1),
        (tpm_window("c").as_str(), 5),
    ]);
    let clock = TestClock::new();
    let mut request = RequestReads::default();
    let picked = service(&pool, 0)
        .select_async(&store, &tiers(&clock), &mut request, &mut first())
        .await
        .unwrap();
    assert_eq!(picked, 0);
    let expected = vec![vec![
        tpm_window("a:b"),
        tpm_window("c"),
        rpm_window("a:b"),
        rpm_window("c"),
    ]];
    assert_eq!(store.reads(), expected);
}

fn a_value_of_another_kind_is_unknown() {
    let pool = [plain("a"), plain("b")];
    let tpm = vec![
        Some(RouteValue::Spend(Decimal::from(3_i32))),
        Some(counter(1)),
    ];
    let rpm = values(&[None, None]);
    let picked = selection(&pool, Some(&tpm), Some(&rpm), 0)
        .choose(&mut last())
        .unwrap();
    assert_eq!(picked, Some(0));
}

async fn a_failed_read_answers_the_cold_pick() {
    let pool = [plain("a"), plain("b")];
    let mut store = FakeStore::new(&[]);
    store.go_down();
    let clock = TestClock::new();
    let mut request = RequestReads::default();
    let picked = service(&pool, 0)
        .select_async(&store, &tiers(&clock), &mut request, &mut last())
        .await
        .unwrap();
    assert_eq!(picked, 1);
    assert_eq!(store.reads().len(), 1);
}

async fn the_async_facade_reads_the_pool_in_one_batch() {
    let pool = [plain("a"), plain("b")];
    let store = FakeStore::new(&[(tpm_window("a").as_str(), 5), (tpm_window("b").as_str(), 1)]);
    let clock = TestClock::new();
    let mut request = RequestReads::default();
    let picked = service(&pool, 0)
        .select_async(&store, &tiers(&clock), &mut request, &mut first())
        .await
        .unwrap();
    assert_eq!(picked, 1);
    let expected = vec![vec![
        tpm_window("a"),
        tpm_window("b"),
        rpm_window("a"),
        rpm_window("b"),
    ]];
    assert_eq!(store.reads(), expected);
}

async fn the_async_facade_uses_the_request_prefetch() {
    let pool = [plain("a"), plain("b")];
    let store = FakeStore::new(&[(tpm_window("b").as_str(), 1)]);
    let clock = TestClock::new();
    let tier = usage_tier(&clock);
    seed(&tier, &[(tpm_window("a").as_str(), 3)]);
    let tiers = ReadTiers {
        cooldown: Arc::new(usage_tier(&clock)),
        usage: Some(Arc::new(tier)),
    };
    let mut request = RequestReads::default();
    assert!(RoutingPrefetch::arm(
        &reads(&["a", "b"]),
        &tiers,
        true,
        &mut request
    ));
    let picked = service(&pool, 0)
        .select_async(&store, &tiers, &mut request, &mut first())
        .await
        .unwrap();
    assert_eq!(picked, 1);
    assert!(request.prefetch.is_none());
    // the fresh local tpm counter of a is not read again; the batch keeps the key order the read declares
    let expected = vec![vec![tpm_window("b"), rpm_window("a"), rpm_window("b")]];
    assert_eq!(store.reads(), expected);
}

fn the_sync_facade_reads_the_local_tier() {
    let pool = [plain("a"), plain("b")];
    let clock = TestClock::new();
    let tier = usage_tier(&clock);
    seed(
        &tier,
        &[(tpm_window("a").as_str(), 5), (tpm_window("b").as_str(), 1)],
    );
    let picked = service(&pool, 0)
        .select_now(Some(&tier), &mut first())
        .unwrap();
    assert_eq!(picked, 1);
}

fn the_sync_facade_without_a_tier_sees_no_counters() {
    let pool = [plain("a"), plain("b")];
    let picked = service(&pool, 0).select_now(None, &mut last()).unwrap();
    assert_eq!(picked, 1);
}

async fn the_async_facade_without_a_usage_tier_sees_no_counters() {
    let pool = [plain("a"), plain("b")];
    let store = FakeStore::new(&[]);
    let clock = TestClock::new();
    let tiers = ReadTiers {
        cooldown: Arc::new(usage_tier(&clock)),
        usage: None,
    };
    let mut request = RequestReads::default();
    let picked = service(&pool, 0)
        .select_async(&store, &tiers, &mut request, &mut first())
        .await
        .unwrap();
    assert_eq!(picked, 0);
    assert!(store.reads().is_empty());
}

async fn the_counter_read_returns_after_the_local_entry_expires() {
    let pool = [plain("a"), plain("b")];
    let mut store = FakeStore::new(&[(tpm_window("a").as_str(), 5), (tpm_window("b").as_str(), 1)]);
    let clock = TestClock::new();
    let tiers = tiers(&clock);
    let mut request = RequestReads::default();
    let first_pick = service(&pool, 0)
        .select_async(&store, &tiers, &mut request, &mut first())
        .await
        .unwrap();
    assert_eq!(first_pick, 1);
    store.set(&tpm_window("a"), 1);
    store.set(&tpm_window("b"), 5);
    // past the local entry's 600-second backfill TTL and the 10-second read expiry
    clock.set(601.0);
    let second_pick = service(&pool, 0)
        .select_async(&store, &tiers, &mut request, &mut first())
        .await
        .unwrap();
    assert_eq!(second_pick, 0);
    assert_eq!(store.reads().len(), 2);
}
