use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_tokenizer::count::wire::TokenRequestWire;
use betterlite_tokenizer::reservation::RequestBody;

use super::support::{self, Behavior, RecordingCounter};
use crate::io_token_reservations::counter::{CountObservation, DualIoCounter, IoCounter};
use crate::io_token_reservations::error::{IoChannel, IoPreCallError};
use crate::io_token_reservations::estimate::estimate_input_tokens;
use crate::io_token_reservations::limits::IoDeployment;
use crate::io_token_reservations::reservation::read_reservation;
use crate::io_token_reservations::service::DEFAULT_MAX_TOKENS;
use crate::io_token_reservations::usage::{ReconcileUsage, UsageTokens, resolve_reconcile_usage};
use crate::routing_value_store::{RouteIncrement, RouteKey, RouteValue, StoreRead};

pub(super) fn run(name: &str) {
    support::run_case_table(name, &CASES);
}

const CASES: [(&str, fn()); 30] = [
    ("full_reservation_stashes_both_channels", || {
        super::runtime().block_on(full_reservation_stashes_both_channels());
    }),
    ("estimate_unavailable_reserves_one_token", || {
        super::runtime().block_on(reserve_one_token(None));
    }),
    ("zero_estimate_reserves_one_token", || {
        super::runtime().block_on(reserve_one_token(Some(0)));
    }),
    ("explicit_zero_output_cap_reserves_nothing", || {
        super::runtime().block_on(explicit_zero_output_cap_reserves_nothing());
    }),
    ("max_completion_tokens_used_when_max_tokens_absent", || {
        super::runtime().block_on(reserve_output_case("max_completion_tokens", 7, 7));
    }),
    ("max_output_tokens_used_when_other_caps_absent", || {
        super::runtime().block_on(reserve_output_case("max_output_tokens", 9, 9));
    }),
    ("max_completion_tokens_wins_over_max_output_tokens", || {
        super::runtime().block_on(output_cap_order());
    }),
    ("max_tokens_wins_over_later_fields", || {
        super::runtime().block_on(output_cap_order());
    }),
    ("model_cap_used_without_explicit_cap", || {
        super::runtime().block_on(model_cap_used_without_explicit_cap());
    }),
    ("unmapped_model_uses_4096_fallback", || {
        super::runtime().block_on(unmapped_model_uses_4096_fallback());
    }),
    ("counter_unavailable_still_stashes_reservation", || {
        super::runtime().block_on(counter_unavailable_still_stashes_reservation());
    }),
    ("messages_and_prompt_estimate_zero", || {
        super::runtime().block_on(messages_and_prompt_estimate_zero());
    }),
    ("output_over_limit_rolls_back_input", || {
        super::runtime().block_on(rollback_releases(Rollback::Output));
    }),
    ("input_over_limit_rolls_back_input", || {
        super::runtime().block_on(rollback_releases(Rollback::Input));
    }),
    ("failing_release_still_reports_rate_limit", || {
        super::runtime().block_on(failing_release_still_reports_rate_limit());
    }),
    ("usage_excludes_cached_tokens", usage_excludes_cached_tokens),
    (
        "usage_falls_back_to_standard_logging",
        usage_resolution_order,
    ),
    (
        "usage_completion_only_resolves",
        usage_completion_only_resolves,
    ),
    (
        "usage_missing_keeps_reservation",
        usage_missing_keeps_reservation,
    ),
    ("tokenizer_counts_messages", || {
        super::runtime().block_on(tokenizer_count_matches(true));
    }),
    ("tokenizer_counts_text", || {
        super::runtime().block_on(tokenizer_count_matches(false));
    }),
    ("store_counter_increments_and_reports", || {
        super::runtime().block_on(store_counter_increments_and_reports());
    }),
    ("redis_down_uses_the_local_count", || {
        super::runtime().block_on(local_count_enforces_limits(true));
    }),
    ("local_counter_enforces_without_redis", || {
        super::runtime().block_on(local_count_enforces_limits(false));
    }),
    ("store_counter_rejects_oversized_key", || {
        super::runtime().block_on(store_counter_rejects_oversized_key());
    }),
    ("usage_input_output_aliases", usage_input_output_aliases),
    ("usage_cached_details_win", usage_cached_details_win),
    (
        "usage_total_tokens_only_is_absent",
        usage_total_tokens_only_is_absent,
    ),
    (
        "standard_logging_cached_metadata",
        standard_logging_cached_metadata,
    ),
    (
        "usage_prefers_response_over_standard_logging",
        usage_resolution_order,
    ),
];

fn rate_limit(error: IoPreCallError) -> (IoChannel, i64, i64) {
    match error {
        IoPreCallError::RateLimit(limit) => (limit.channel, limit.limit, limit.current),
        IoPreCallError::OutputCap { field } => panic!("unexpected cap error for {field}"),
    }
}

async fn full_reservation_stashes_both_channels() {
    let counter = Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Record));
    let reservations = support::service(Arc::clone(&counter), Some(10));
    let deployment = support::deployment(Some(100), Some(50));
    let mut request = support::body();
    request.insert("max_tokens".to_owned(), support::number(20));
    let reserved = reservations
        .pre_call_check(&deployment, &mut request)
        .await
        .unwrap()
        .unwrap();
    let keys = support::keys(&deployment);
    assert_eq!(reserved.itpm_reserved, 10);
    assert_eq!(reserved.otpm_reserved, 20);
    assert_eq!(reserved.itpm_key.as_deref(), Some(keys.itpm.as_str()));
    assert_eq!(reserved.otpm_key.as_deref(), Some(keys.otpm.as_str()));
    assert_eq!(counter.value(&keys.itpm), 10);
    assert_eq!(counter.value(&keys.otpm), 20);
    assert_eq!(read_reservation(&request), Some(reserved));
}

async fn reserve_one_token(estimate: Option<i64>) {
    let counter = Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Record));
    let reservations = support::service(Arc::clone(&counter), estimate);
    let deployment = support::deployment(Some(100), None);
    let mut request = support::body();
    let reserved = reservations
        .pre_call_check(&deployment, &mut request)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reserved.itpm_reserved, 1);
    assert_eq!(counter.value(&support::keys(&deployment).itpm), 1);
}

async fn explicit_zero_output_cap_reserves_nothing() {
    let counter = Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Record));
    let reservations = support::service(Arc::clone(&counter), Some(10));
    let deployment = support::deployment(Some(100), Some(0));
    let mut request = support::body();
    request.insert("max_tokens".to_owned(), support::number(0));
    let reserved = reservations
        .pre_call_check(&deployment, &mut request)
        .await
        .unwrap()
        .unwrap();
    let keys = support::keys(&deployment);
    assert_eq!(reserved.otpm_reserved, 0);
    assert_eq!(reserved.otpm_key.as_deref(), Some(keys.otpm.as_str()));
    assert_eq!(counter.value(&keys.otpm), 0);
    let calls = counter.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].key, keys.itpm);
    assert_eq!(calls[0].value, 10);
    assert_eq!(calls[0].ttl_seconds, 60);
}

async fn reserve_with_caps(first: Option<(&str, i64)>, second: (&str, i64)) -> i64 {
    let counter = Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Record));
    let reservations = support::service(counter, Some(10));
    let deployment = support::deployment(None, Some(100));
    let mut request = support::body();
    if let Some((name, cap)) = first {
        request.insert(name.to_owned(), support::number(cap));
    }
    request.insert(second.0.to_owned(), support::number(second.1));
    reservations
        .pre_call_check(&deployment, &mut request)
        .await
        .unwrap()
        .unwrap()
        .otpm_reserved
}

async fn reserve_output_case(name: &str, cap: i64, expected: i64) {
    assert_eq!(reserve_with_caps(None, (name, cap)).await, expected);
}

// max_tokens wins over max_completion_tokens, which wins over max_output_tokens
async fn output_cap_order() {
    assert_eq!(
        reserve_with_caps(Some(("max_tokens", 20)), ("max_completion_tokens", 7)).await,
        20
    );
    assert_eq!(
        reserve_with_caps(Some(("max_completion_tokens", 7)), ("max_output_tokens", 9)).await,
        7
    );
}

async fn model_cap_used_without_explicit_cap() {
    let counter = Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Record));
    let reservations = support::service(counter, Some(10));
    let deployment = support::deployment(None, Some(20_000));
    let mut request = support::body();
    let reserved = reservations
        .pre_call_check(&deployment, &mut request)
        .await
        .unwrap()
        .unwrap();
    // gpt-4o's max_output_tokens in the pinned catalog
    assert_eq!(reserved.otpm_reserved, 0x4000);
}

async fn unmapped_model_uses_4096_fallback() {
    let counter = Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Record));
    let reservations = support::service(counter, Some(10));
    let deployment = IoDeployment {
        model_id: Some("dep-a"),
        model: Some("no-such-model-b365"),
        limits: support::deployment(None, Some(20_000)).limits,
    };
    let mut request = support::body();
    let reserved = reservations
        .pre_call_check(&deployment, &mut request)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reserved.otpm_reserved, DEFAULT_MAX_TOKENS);
}

async fn counter_unavailable_still_stashes_reservation() {
    let counter = Arc::new(RecordingCounter::new(
        Behavior::Unavailable,
        Behavior::Unavailable,
    ));
    let reservations = support::service(Arc::clone(&counter), Some(10));
    let deployment = support::deployment(Some(100), Some(50));
    let mut request = support::body();
    request.insert("max_tokens".to_owned(), support::number(20));
    let reserved = reservations
        .pre_call_check(&deployment, &mut request)
        .await
        .unwrap()
        .unwrap();
    let keys = support::keys(&deployment);
    assert_eq!(reserved.itpm_reserved, 10);
    assert_eq!(reserved.otpm_reserved, 20);
    assert_eq!(counter.value(&keys.itpm), 0);
    assert_eq!(counter.value(&keys.otpm), 0);
    assert_eq!(read_reservation(&request), Some(reserved));
}

#[derive(Clone, Copy)]
enum Rollback {
    Output,
    Input,
}

async fn rollback_releases(scenario: Rollback) {
    let (itpm_limit, otpm_limit, estimate, max_tokens) = match scenario {
        Rollback::Output => (100_i64, 15_i64, 10_i64, 20_i64),
        Rollback::Input => (5_i64, 50_i64, 10_i64, 20_i64),
    };
    let (channel, limit, current, itpm_after) = match scenario {
        Rollback::Output => (IoChannel::Otpm, 15_i64, 27_i64, 5_i64),
        Rollback::Input => (IoChannel::Itpm, 5_i64, 15_i64, 5_i64),
    };
    let counter = Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Record));
    let deployment = support::deployment(Some(itpm_limit), Some(otpm_limit));
    let keys = support::keys(&deployment);
    counter.seed(&keys.itpm, 5);
    counter.seed(&keys.otpm, 7);
    let reservations = support::service(Arc::clone(&counter), Some(estimate));
    let mut request = support::body();
    request.insert("max_tokens".to_owned(), support::number(max_tokens));
    let error = reservations
        .pre_call_check(&deployment, &mut request)
        .await
        .unwrap_err();
    assert_eq!(rate_limit(error), (channel, limit, current));
    assert_eq!(counter.value(&keys.itpm), itpm_after);
    assert!(read_reservation(&request).is_none());
    let values: Vec<i64> = counter.calls().iter().map(|call| call.value).collect();
    match scenario {
        Rollback::Output => {
            assert_eq!(counter.value(&keys.otpm), 7);
            assert_eq!(values, vec![10, 20, -20, -10]);
        }
        Rollback::Input => {
            assert_eq!(counter.value(&keys.otpm), 7);
            assert_eq!(values, vec![10, -10]);
        }
    }
}

async fn failing_release_still_reports_rate_limit() {
    let counter =
        Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Record).with_failing_releases());
    let deployment = support::deployment(Some(100), Some(15));
    let keys = support::keys(&deployment);
    counter.seed(&keys.itpm, 5);
    counter.seed(&keys.otpm, 7);
    let reservations = support::service(Arc::clone(&counter), Some(10));
    let mut request = support::body();
    request.insert("max_tokens".to_owned(), support::number(20));
    let error = reservations
        .pre_call_check(&deployment, &mut request)
        .await
        .unwrap_err();
    assert_eq!(rate_limit(error).0, IoChannel::Otpm);
    assert_eq!(counter.value(&keys.itpm), 15);
    assert_eq!(counter.value(&keys.otpm), 27);
}

fn usage_excludes_cached_tokens() {
    let usage = resolve_reconcile_usage(
        Some(UsageTokens {
            prompt_tokens: 10,
            completion_tokens: 3,
            cached_tokens: 4,
        }),
        None,
    );
    assert_eq!(
        usage,
        ReconcileUsage {
            billable_input: 6,
            completion_tokens: 3,
            usage_resolved: true,
        },
        "cached prompt-read tokens are excluded from billable input"
    );
}

// a response usage wins over the standard_logging_object copy when both are present
fn usage_resolution_order() {
    let fallback = resolve_reconcile_usage(
        None,
        Some(UsageTokens {
            prompt_tokens: 7,
            completion_tokens: 0,
            cached_tokens: 3,
        }),
    );
    assert_eq!(
        fallback,
        ReconcileUsage {
            billable_input: 4,
            completion_tokens: 0,
            usage_resolved: true,
        }
    );
    let preferred = resolve_reconcile_usage(
        Some(UsageTokens {
            prompt_tokens: 10,
            completion_tokens: 3,
            cached_tokens: 4,
        }),
        Some(UsageTokens {
            prompt_tokens: 50,
            completion_tokens: 20,
            cached_tokens: 0,
        }),
    );
    assert_eq!(
        preferred,
        ReconcileUsage {
            billable_input: 6,
            completion_tokens: 3,
            usage_resolved: true,
        }
    );
}

fn usage_completion_only_resolves() {
    let usage = resolve_reconcile_usage(
        None,
        Some(UsageTokens {
            prompt_tokens: 0,
            completion_tokens: 2,
            cached_tokens: 0,
        }),
    );
    assert_eq!(
        usage,
        ReconcileUsage {
            billable_input: 0,
            completion_tokens: 2,
            usage_resolved: true,
        }
    );
}

fn usage_missing_keeps_reservation() {
    let usage = resolve_reconcile_usage(None, None);
    assert_eq!(usage, ReconcileUsage::default());
}

async fn messages_and_prompt_estimate_zero() {
    let counter = Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Record));
    let reservations = support::service(counter, Some(10));
    let deployment = support::deployment(Some(100), None);
    let mut request = support::body();
    request.insert("prompt".to_owned(), Json::Text("hello".to_owned()));
    let reserved = reservations
        .pre_call_check(&deployment, &mut request)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reserved.itpm_reserved, 1);
}

async fn tokenizer_count_matches(messages: bool) {
    let counter = Arc::new(support::token_counter());
    let estimator = support::tokenizer_estimator(Arc::clone(&counter));
    let (request, wire): (RequestBody, &[u8]) = if messages {
        (
            support::body(),
            br#"{"model":"gpt-4","messages":[{"role":"user","content":"hello"}]}"#.as_slice(),
        )
    } else {
        (
            support::body_with("prompt", Json::Text("hello world".to_owned())),
            br#"{"model":"gpt-4","text":"hello world"}"#.as_slice(),
        )
    };
    let measured = estimate_input_tokens(&estimator, "gpt-4", &request).await;
    let parsed = TokenRequestWire::try_from(wire).unwrap();
    let expected = counter.count(&parsed.request(None)).await.unwrap();
    assert_eq!(measured, i64::try_from(expected).unwrap());
}

async fn store_counter_increments_and_reports() {
    let fixture = support::store_fixture().await;
    let counter = DualIoCounter {
        local: support::local_store(),
        remote: Some(Arc::new(fixture.store)),
    };
    let key = format!("global_router:dep-a:gpt-4o:itpm:{}", support::minute());
    assert_eq!(
        IoCounter::increment_counter(&counter, &key, 5, 60).await,
        CountObservation::Counted(5)
    );
    assert_eq!(
        IoCounter::increment_counter(&counter, &key, 2, 60).await,
        CountObservation::Counted(7)
    );
    let route = RouteKey::parse(&key).unwrap();
    assert_eq!(
        counter.local.get(&route).unwrap(),
        StoreRead::Value(RouteValue::Counter(7))
    );
    let (stored, ttl) = fixture.server.raw(&key).unwrap();
    assert_eq!(stored, br#"{"counter":7}"#);
    assert_eq!(ttl, 60);
}

// LiteLLM's DualCache keeps the in-memory increment when Redis is down or not configured at all
async fn local_count_enforces_limits(redis_down: bool) {
    let local = support::local_store();
    let deployment = support::deployment(Some(100), None);
    let keys = support::keys(&deployment);
    let route = RouteKey::parse(&keys.itpm).unwrap();
    assert_eq!(
        local
            .increment(&route, &RouteIncrement::Counter(95), None)
            .unwrap(),
        RouteValue::Counter(95)
    );
    let (remote, _server) = if redis_down {
        let fixture = support::store_fixture().await;
        fixture.server.set_down(true);
        (Some(Arc::new(fixture.store)), Some(fixture.server))
    } else {
        (None, None)
    };
    let counter = Arc::new(DualIoCounter { local, remote });
    let reservations = support::service(counter, Some(10));
    let mut request = support::body();
    let error = reservations
        .pre_call_check(&deployment, &mut request)
        .await
        .unwrap_err();
    assert_eq!(rate_limit(error), (IoChannel::Itpm, 100, 105));
}

async fn store_counter_rejects_oversized_key() {
    let fixture = support::store_fixture().await;
    let counter = DualIoCounter {
        local: support::local_store(),
        remote: Some(Arc::new(fixture.store)),
    };
    let key = "k".repeat(1025);
    assert_eq!(
        IoCounter::increment_counter(&counter, &key, 1, 60).await,
        CountObservation::Unavailable
    );
}

fn usage_input_output_aliases() {
    let usage = UsageTokens::from_usage(&support::object(&[
        ("input_tokens", support::number(100)),
        ("output_tokens", support::number(20)),
        ("cache_read_input_tokens", support::number(30)),
    ]))
    .unwrap();
    assert_eq!(
        usage,
        UsageTokens {
            prompt_tokens: 100,
            completion_tokens: 20,
            cached_tokens: 30,
        }
    );
    assert_eq!(
        resolve_reconcile_usage(Some(usage), None),
        ReconcileUsage {
            billable_input: 70,
            completion_tokens: 20,
            usage_resolved: true,
        }
    );
}

fn usage_cached_details_win() {
    let usage = UsageTokens::from_usage(&support::object(&[
        ("prompt_tokens", support::number(50)),
        ("completion_tokens", support::number(5)),
        (
            "prompt_tokens_details",
            support::object(&[("cached_tokens", support::number(20))]),
        ),
        ("cache_read_input_tokens", support::number(9)),
    ]))
    .unwrap();
    assert_eq!(usage.cached_tokens, 20);
}

fn usage_total_tokens_only_is_absent() {
    let total_only = support::object(&[("total_tokens", support::number(100))]);
    assert!(UsageTokens::from_usage(&total_only).is_none());
    assert!(!resolve_reconcile_usage(None, None).usage_resolved);
}

fn standard_logging_cached_metadata() {
    let logging = support::object(&[
        ("prompt_tokens", support::number(10)),
        ("completion_tokens", support::number(4)),
        (
            "metadata",
            support::object(&[("cache_read_input_tokens", support::number(6))]),
        ),
    ]);
    let usage = UsageTokens::from_standard_logging(&logging).unwrap();
    assert_eq!(
        usage,
        UsageTokens {
            prompt_tokens: 10,
            completion_tokens: 4,
            cached_tokens: 6,
        }
    );
    assert_eq!(resolve_reconcile_usage(None, Some(usage)).billable_input, 4);
}
