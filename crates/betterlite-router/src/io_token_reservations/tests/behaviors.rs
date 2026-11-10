use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_tokenizer::count::wire::TokenRequestWire;
use betterlite_tokenizer::reservation::RequestBody;

use super::support::{self, Behavior, RecordingCounter};
use crate::io_token_reservations::counter::IoCounter;
use crate::io_token_reservations::error::{IoChannel, IoPreCallError};
use crate::io_token_reservations::estimate::estimate_input_tokens;
use crate::io_token_reservations::limits::IoDeployment;
use crate::io_token_reservations::reservation::read_reservation;
use crate::io_token_reservations::service::DEFAULT_MAX_TOKENS;
use crate::io_token_reservations::usage::{ReconcileUsage, UsageTokens, resolve_reconcile_usage};

pub(super) async fn run(name: &str) {
    if reservation_cases(name).await {
        return;
    }
    if remaining_cases(name).await {
        return;
    }
    panic!("unknown case {name}");
}

async fn reservation_cases(name: &str) -> bool {
    match name {
        "full_reservation_stashes_both_channels" => full_reservation_stashes_both_channels().await,
        "estimate_unavailable_reserves_one_token" => reserve_one_token(None).await,
        "zero_estimate_reserves_one_token" => reserve_one_token(Some(0)).await,
        "explicit_zero_output_cap_reserves_nothing" => {
            explicit_zero_output_cap_reserves_nothing().await;
        }
        "max_completion_tokens_used_when_max_tokens_absent" => {
            reserve_output("max_completion_tokens", 7).await;
        }
        "max_output_tokens_used_when_other_caps_absent" => {
            reserve_output("max_output_tokens", 9).await;
        }
        "max_tokens_wins_over_later_fields" => max_tokens_wins_over_later_fields().await,
        "model_cap_used_without_explicit_cap" => model_cap_used_without_explicit_cap().await,
        "unmapped_model_uses_4096_fallback" => unmapped_model_uses_4096_fallback().await,
        "counter_unavailable_still_stashes_reservation" => {
            counter_unavailable_still_stashes_reservation().await;
        }
        "messages_and_prompt_estimate_zero" => messages_and_prompt_estimate_zero().await,
        _ => return false,
    }
    true
}

async fn remaining_cases(name: &str) -> bool {
    match name {
        "output_over_limit_rolls_back_input" => rollback_releases(Rollback::Output).await,
        "input_over_limit_rolls_back_input" => rollback_releases(Rollback::Input).await,
        "failing_release_still_reports_rate_limit" => {
            failing_release_still_reports_rate_limit().await;
        }
        "usage_excludes_cached_tokens" => usage_excludes_cached_tokens(),
        "usage_falls_back_to_standard_logging" => usage_falls_back_to_standard_logging(),
        "usage_missing_keeps_reservation" => usage_missing_keeps_reservation(),
        "tokenizer_counts_messages" => tokenizer_count_matches(true).await,
        "tokenizer_counts_text" => tokenizer_count_matches(false).await,
        "store_counter_increments_and_reports" => store_counter_increments_and_reports().await,
        "store_counter_unavailable_is_none" => store_counter_unavailable_is_none().await,
        _ => return false,
    }
    true
}

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

async fn reserve_output(name: &str, cap: i64) -> i64 {
    let counter = Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Record));
    let reservations = support::service(counter, Some(10));
    let deployment = support::deployment(None, Some(100));
    let mut request = support::body();
    request.insert(name.to_owned(), support::number(cap));
    reservations
        .pre_call_check(&deployment, &mut request)
        .await
        .unwrap()
        .unwrap()
        .otpm_reserved
}

async fn max_tokens_wins_over_later_fields() {
    let counter = Arc::new(RecordingCounter::new(Behavior::Record, Behavior::Record));
    let reservations = support::service(counter, Some(10));
    let deployment = support::deployment(None, Some(100));
    let mut request = support::body();
    request.insert("max_tokens".to_owned(), support::number(20));
    request.insert("max_completion_tokens".to_owned(), support::number(7));
    let reserved = reservations
        .pre_call_check(&deployment, &mut request)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reserved.otpm_reserved, 20);
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
        Rollback::Input => (IoChannel::Itpm, 5_i64, 10_i64, 0_i64),
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
            assert_eq!(counter.value(&keys.otpm), 0);
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

fn usage_falls_back_to_standard_logging() {
    let usage = resolve_reconcile_usage(
        None,
        Some(UsageTokens {
            prompt_tokens: 7,
            completion_tokens: 2,
            cached_tokens: 3,
        }),
    );
    assert_eq!(
        usage,
        ReconcileUsage {
            billable_input: 4,
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
    let key = format!("global_router:dep-a:gpt-4o:itpm:{}", support::minute());
    assert_eq!(
        IoCounter::increment_counter(&fixture.store, &key, 5, 60).await,
        Some(5)
    );
    assert_eq!(
        IoCounter::increment_counter(&fixture.store, &key, 2, 60).await,
        Some(7)
    );
    let (stored, ttl) = fixture.server.raw(&key).unwrap();
    assert_eq!(stored, br#"{"counter":7}"#);
    assert_eq!(ttl, 60);
}

async fn store_counter_unavailable_is_none() {
    let fixture = support::store_fixture().await;
    fixture.server.set_down(true);
    let key = format!("global_router:dep-a:itpm:{}", support::minute());
    assert_eq!(
        IoCounter::increment_counter(&fixture.store, &key, 1, 60).await,
        None
    );
}
