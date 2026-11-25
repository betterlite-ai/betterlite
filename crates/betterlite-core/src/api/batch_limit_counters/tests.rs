#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_error::rate_limit::RateLimitDimension;
use rstest::rstest;

use crate::api::api_boundaries::ApiLimitError;
use crate::api::batch_enqueued_reserve::ReservationBackend;
use crate::api::batch_limit_counters::BatchLimitCountersError;
use crate::api::batch_limit_counters::input::BatchLimitCounterCall;
use crate::api::batch_limit_counters::records::{
    BatchCounterCode, BatchCounterLimitType, BatchLimitCounterAnswer, BatchReservationBackend,
    WindowStartValue,
};
use crate::api::batch_limit_counters::rejections::request_model;
use crate::api::batch_limit_descriptors::descriptors::TPD_WINDOW_SECONDS;

use self::support::{
    LimiterFailure, PARTIAL_FAILURE_DETAIL, RESET_AT, ScriptedCache, ScriptedLimiter,
    ScriptedStore, Stored, WINDOW, assert_direct_enqueued_model, assert_enqueued_rejection,
    assert_requests_rejection, daily_descriptor, daily_refund_response, data, descriptor,
    detail_text, empty_model_data, expected_operations, harness, header, increments, input,
    non_text_model_data, null_model_data, operations_of, over_limit, partial_failure_cache,
    partial_failure_limiter, project_usage_row, refund_call, refund_windows, rejection,
    requests_limiter, reserved, response, run_check, run_raise, run_refund, run_window, scope,
    status, stored_value, usage, usage_for_row, usage_with_rows, window_start_of, window_status,
};

mod support;

// one row of batch_tpm_limit's table: the descriptor, the usage row, and the figure the detail prints
struct TpmCase {
    key: &'static str,
    value: &'static str,
    model: Option<&'static str>,
    row_total: Option<i64>,
    row_output: Option<i64>,
    label: &'static str,
    tokens: i64,
}

// batch_rpm_limit: a requests counter at its limit passes and over it rejects with a requests 429
#[rstest]
#[case::over_limit(6, BatchCounterCode::OverLimit, 2)]
#[case::at_limit(5, BatchCounterCode::Ok, 0)]
#[tokio::test]
async fn batch_rpm_limit(
    #[case] requests: i64,
    #[case] code: BatchCounterCode,
    #[case] remaining: i64,
) {
    let limits = descriptor("api_key", "sk-1", Some(WINDOW), Some(5), None);
    let (rig, result) = run_check(
        requests_limiter(code, remaining),
        ScriptedCache::default(),
        data(Some("gpt-4o-mini")),
        usage(120, requests, 0),
        Some(vec![limits.clone()]),
    )
    .await;
    let calls = rig.limiter.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, vec![limits]);
    assert_eq!(calls[0].1, vec![increments(requests, 120)]);
    assert_eq!(result.answers.len(), 1);
    if code == BatchCounterCode::OverLimit {
        assert_requests_rejection(
            &result.answers[0],
            "gpt-4o-mini",
            "openai",
            "batch requests rate limit reached for api_key: sk-1: 6 requests, 2 of 5 remaining",
            "60",
        );
    } else {
        assert!(matches!(
            result.answers[0],
            BatchLimitCounterAnswer::CheckAndIncrementBatchCounters { rejection: None }
        ));
    }
    assert!(rig.stash.operations().is_empty());
}

// no descriptors at all: the synthetic key is empty, the window falls back to the limiter's, and no model resolves
#[tokio::test]
async fn batch_rpm_limit_without_descriptors() {
    let (_, result) = run_raise(
        status(
            BatchCounterCode::OverLimit,
            "api_key",
            Some("sk-1"),
            BatchCounterLimitType::Requests,
            (5, 0),
        ),
        Vec::new(),
        usage(120, 1, 0),
        None,
        None,
    )
    .await;
    assert_requests_rejection(
        &result.answers[0],
        "",
        "litellm_proxy",
        "batch requests rate limit reached for : : 1 requests, 0 of 5 remaining",
        "60",
    );
}

// a status that matches no descriptor reports the first descriptor's key and window, and its tokens label
#[tokio::test]
async fn batch_rpm_limit_unmatched_descriptor() {
    let (_, result) = run_raise(
        status(
            BatchCounterCode::OverLimit,
            "api_key_tpd",
            Some("sk-1"),
            BatchCounterLimitType::Tokens,
            (100, 0),
        ),
        vec![descriptor(
            "team",
            "team-1",
            Some(TPD_WINDOW_SECONDS),
            None,
            Some(100),
        )],
        usage(120, 1, 0),
        Some("gpt-4o-mini"),
        Some(1_699_990_000),
    )
    .await;
    let error = rejection(&result.answers[0]);
    assert_eq!(
        detail_text(&error.detail),
        "batch tokens rate limit reached for team: team-1: 120 tokens, 0 of 100 per minute remaining"
    );
    assert_eq!(header(error, "retry-after"), "76400");
}

// a negative remaining count displays as zero
#[tokio::test]
async fn batch_rpm_limit_negative_remaining() {
    let (_, result) = run_raise(
        status(
            BatchCounterCode::OverLimit,
            "api_key",
            Some("sk-1"),
            BatchCounterLimitType::Requests,
            (5, -4),
        ),
        Vec::new(),
        usage(120, 6, 0),
        None,
        None,
    )
    .await;
    assert_requests_rejection(
        &result.answers[0],
        "",
        "litellm_proxy",
        "batch requests rate limit reached for : : 6 requests, 0 of 5 remaining",
        "60",
    );
}

// batch_tpm_limit: the token figure is the charged model's row for project quotas, the batch total otherwise
#[rstest]
#[case::batch_total(TpmCase { key: "api_key", value: "sk-1", model: None, row_total: None, row_output: None, label: "minute", tokens: 120 })]
#[case::tpd_label(TpmCase { key: "api_key_tpd", value: "sk-1", model: None, row_total: None, row_output: None, label: "day", tokens: 120 })]
#[case::project_otpm_row(TpmCase { key: "model_per_project_otpm", value: "proj:gpt-4o", model: Some("gpt-4o"), row_total: None, row_output: Some(30), label: "minute", tokens: 30 })]
#[case::project_otpm_absent_falls_back(TpmCase { key: "model_per_project_otpm", value: "proj:gpt-4o", model: Some("gpt-4o"), row_total: Some(40), row_output: None, label: "minute", tokens: 7 })]
#[case::project_itpm_row(TpmCase { key: "model_per_project_itpm", value: "proj:gpt-4o", model: Some("gpt-4o"), row_total: Some(40), row_output: None, label: "minute", tokens: 40 })]
#[case::project_itpm_absent_falls_back(TpmCase { key: "model_per_project_itpm", value: "proj:gpt-4o", model: Some("gpt-4o"), row_total: None, row_output: Some(30), label: "minute", tokens: 120 })]
#[case::project_value_without_colon(TpmCase { key: "model_per_project_itpm", value: "gpt-4o", model: Some("gpt-4o"), row_total: Some(40), row_output: None, label: "minute", tokens: 40 })]
#[case::project_empty_model_falls_back(TpmCase { key: "model_per_project_itpm", value: "proj:", model: Some(""), row_total: Some(5), row_output: None, label: "minute", tokens: 120 })]
#[tokio::test]
async fn batch_tpm_limit(#[case] case: TpmCase) {
    let limits = descriptor(case.key, case.value, Some(WINDOW), None, Some(100));
    let (_, result) = run_raise(
        status(
            BatchCounterCode::OverLimit,
            case.key,
            Some(case.value),
            BatchCounterLimitType::Tokens,
            (100, 5),
        ),
        vec![limits],
        usage_for_row(case.model, case.row_total, case.row_output),
        Some("gpt-4o-mini"),
        None,
    )
    .await;
    let error = rejection(&result.answers[0]);
    assert_eq!(error.rate_limit_type, Some(RateLimitDimension::Tokens));
    assert_eq!(error.model, "gpt-4o-mini");
    assert_eq!(error.llm_provider, "openai");
    let expected = format!(
        "batch tokens rate limit reached for {}: {}: {} tokens, 5 of 100 per {} remaining",
        case.key, case.value, case.tokens, case.label
    );
    assert_eq!(detail_text(&error.detail), expected);
    assert_eq!(header(error, "retry-after"), "60");
    assert_eq!(header(error, "rate_limit_type"), "tokens");
    assert_eq!(header(error, "reset_at"), RESET_AT);
}

// batch_enqueued_limit: an over-limit scope rejects, and data's model stays optional
#[rstest]
#[case::with_model(Some("gpt-4o-mini"), "gpt-4o-mini", "openai")]
#[case::without_model(None, "", "litellm_proxy")]
#[tokio::test]
async fn batch_enqueued_limit(
    #[case] model: Option<&str>,
    #[case] expected_model: &str,
    #[case] expected_provider: &str,
) {
    let scopes = vec![scope("sk-1", 15)];
    let rig = harness(
        ScriptedLimiter::default(),
        ScriptedStore::with_reply(Ok(over_limit(scope("sk-1", 15), 10))),
        ScriptedCache::default(),
    );
    let result = rig
        .service
        .run(&input(vec![
            BatchLimitCounterCall::ReserveBatchEnqueuedTokens {
                data: data(model),
                batch_usage: usage(30, 2, 0),
                scopes: scopes.clone(),
            },
        ]))
        .await
        .unwrap();
    assert_eq!(rig.store.calls(), vec![(30, scopes)]);
    assert!(matches!(
        result.answers[0],
        BatchLimitCounterAnswer::ReserveBatchEnqueuedTokens { rejection: Some(_) }
    ));
    assert_enqueued_rejection(&result.answers[0], expected_model, expected_provider);
    assert!(rig.stash.reservation().is_none());
}

// a granted reservation is stashed with the store's backend and the call answers without a rejection
#[tokio::test]
async fn enqueued_reservation_is_stashed() {
    let scopes = vec![scope("sk-1", 15)];
    let rig = harness(
        ScriptedLimiter::default(),
        ScriptedStore::with_reply(Ok(reserved(30, &scopes))),
        ScriptedCache::default(),
    );
    let result = rig
        .service
        .run(&input(vec![
            BatchLimitCounterCall::ReserveBatchEnqueuedTokens {
                data: data(Some("gpt-4o-mini")),
                batch_usage: usage(30, 2, 0),
                scopes: scopes.clone(),
            },
        ]))
        .await
        .unwrap();
    assert_eq!(rig.store.calls(), vec![(30, scopes.clone())]);
    let reservation = rig.stash.reservation().expect("the grant is stashed");
    assert_eq!(reservation.tokens, 30);
    assert_eq!(reservation.scopes, scopes);
    assert_eq!(reservation.backend, ReservationBackend::Memory);
    assert!(matches!(
        result.answers[0],
        BatchLimitCounterAnswer::ReserveBatchEnqueuedTokens { rejection: None }
    ));
}

// more enqueued tokens than the allowance still displays zero remaining
#[tokio::test]
async fn enqueued_over_allowance_shows_zero_remaining() {
    let rig = harness(
        ScriptedLimiter::default(),
        ScriptedStore::with_reply(Ok(over_limit(scope("sk-1", 15), 20))),
        ScriptedCache::default(),
    );
    let result = rig
        .service
        .run(&input(vec![
            BatchLimitCounterCall::ReserveBatchEnqueuedTokens {
                data: data(Some("gpt-4o-mini")),
                batch_usage: usage(30, 2, 0),
                scopes: vec![scope("sk-1", 15)],
            },
        ]))
        .await
        .unwrap();
    let error = rejection(&result.answers[0]);
    assert_eq!(
        detail_text(&error.detail),
        "batch enqueued token limit reached for api_key: sk-1: 30 tokens requested, 0 of 15 remaining"
    );
}

// the direct RaiseEnqueuedLimitError callable reads data's model too, and an explicit null is not a model
#[tokio::test]
async fn direct_enqueued_model() {
    let rig = harness(
        ScriptedLimiter::default(),
        ScriptedStore::default(),
        ScriptedCache::default(),
    );
    assert_direct_enqueued_model(&rig, data(Some("gpt-4o-mini")), "gpt-4o-mini", "openai").await;
    assert_direct_enqueued_model(&rig, null_model_data(), "", "litellm_proxy").await;
    assert_direct_enqueued_model(
        &rig,
        data(Some("unknown-batch-model")),
        "unknown-batch-model",
        "litellm_proxy",
    )
    .await;
}

// tpd_window_refund: a daily window's stored value truncates to seconds, and an unreadable one is no window
#[rstest]
#[case::integer(Some(Stored::Integer(1_699_990_000)), Some(1_699_990_000))]
#[case::float_truncates(Some(Stored::Float(1_699_990_000.9)), Some(1_699_990_000))]
#[case::numeric_text(Some(Stored::Text(" 1699990000.5 ")), Some(1_699_990_000))]
#[case::missing(None, None)]
#[case::invalid_text(Some(Stored::Text("yesterday")), None)]
#[case::infinite(Some(Stored::Float(f64::INFINITY)), None)]
#[case::beyond_i64(Some(Stored::Float(1.0e300)), None)]
#[tokio::test]
async fn tpd_window_refund(#[case] stored: Option<Stored>, #[case] expected_start: Option<i64>) {
    let cache = stored.map_or_else(ScriptedCache::default, |value| {
        ScriptedCache::with_value("{api_key_tpd:sk-1}:window", stored_value(value))
    });
    let (rig, result) = run_window(cache, window_status("api_key_tpd", Some("sk-1"))).await;
    assert_eq!(window_start_of(&result.answers[0]), expected_start);
    assert_eq!(
        rig.cache.reads(),
        vec![String::from("{api_key_tpd:sk-1}:window")]
    );
}

// a per-minute counter has no window key, so the cache is never read
#[tokio::test]
async fn per_minute_counter_has_no_window() {
    let (rig, result) = run_window(
        ScriptedCache::default(),
        window_status("api_key", Some("sk-1")),
    )
    .await;
    assert_eq!(window_start_of(&result.answers[0]), None);
    assert!(rig.cache.reads().is_empty());
}

// an absent descriptor value builds the window key with an empty value
#[tokio::test]
async fn absent_descriptor_value_builds_empty_key() {
    let cache = ScriptedCache::with_value(
        "{team_tpd:}:window",
        WindowStartValue::Integer(1_699_990_000),
    );
    let (rig, result) = run_window(cache, window_status("team_tpd", None)).await;
    assert_eq!(window_start_of(&result.answers[0]), Some(1_699_990_000));
    assert_eq!(rig.cache.reads(), vec![String::from("{team_tpd:}:window")]);
}

// tpd_window_refund: the daily counters this batch charged become negative-token refund ops
#[rstest]
#[case::positive(120, 3)]
#[case::zero(0, 0)]
#[case::negative(-5, 0)]
#[tokio::test]
async fn tpd_window_refund_operations(#[case] tokens: i64, #[case] expected: usize) {
    let (_, result) = run_refund(
        vec![
            daily_descriptor(),
            descriptor("api_key", "sk-1", Some(WINDOW), Some(5), None),
        ],
        tokens,
        refund_windows(),
    )
    .await;
    let operations = operations_of(&result.answers[0]);
    assert_eq!(operations.len(), expected);
    if expected > 0 {
        let refunded = tokens.checked_neg().expect("a positive count negates");
        assert_eq!(*operations, expected_operations(refunded));
    }
}

// a check that succeeds stashes the refunds, and a window-aware rejection resets from that window
#[tokio::test]
async fn refunds_stash_and_aware_rejection() {
    let daily = daily_descriptor();
    let limiter = ScriptedLimiter::with_responses(vec![Ok(daily_refund_response())]);
    let (rig, result) = run_check(
        limiter,
        ScriptedCache::default(),
        data(Some("gpt-4o-mini")),
        usage(120, 3, 0),
        Some(vec![daily.clone()]),
    )
    .await;
    assert!(matches!(
        result.answers[0],
        BatchLimitCounterAnswer::CheckAndIncrementBatchCounters { rejection: None }
    ));
    let stashed = rig.stash.operations();
    assert_eq!(stashed.len(), 1);
    assert_eq!(stashed[0].key, "{api_key_tpd:sk-1}:tokens");
    assert_eq!(stashed[0].increment_value, -120);
    let (_, aware) = run_raise(
        status(
            BatchCounterCode::OverLimit,
            "api_key_tpd",
            Some("sk-1"),
            BatchCounterLimitType::Tokens,
            (100, 5),
        ),
        vec![daily],
        usage(120, 3, 0),
        Some("gpt-4o-mini"),
        Some(1_699_990_000),
    )
    .await;
    let error = rejection(&aware.answers[0]);
    assert_eq!(header(error, "retry-after"), "76400");
    assert_eq!(header(error, "reset_at"), "2023-11-15 19:26:40 UTC");
}

// counter_partial_failure: the first over-limit counter rejects and later over-limit counters are never read
#[tokio::test]
async fn counter_partial_failure() {
    let (rig, result) = run_check(
        partial_failure_limiter(),
        partial_failure_cache(),
        data(Some("gpt-4o-mini")),
        usage_with_rows(),
        None,
    )
    .await;
    assert_eq!(rig.limiter.project_usages(), vec![project_usage_row()]);
    let calls = rig.limiter.calls();
    assert_eq!(rig.limiter.base_calls(), 1);
    assert_eq!(calls.len(), 1);
    let charged = &calls[0].0;
    assert_eq!(charged.len(), 3);
    assert_eq!(charged[0].key, "api_key");
    assert_eq!(charged[1].key, "api_key_tpd");
    assert_eq!(
        charged[2],
        descriptor(
            "model_per_project_otpm",
            "proj:gpt-4o",
            Some(WINDOW),
            None,
            Some(100)
        )
    );
    assert_eq!(
        calls[0].1,
        vec![increments(3, 120), increments(3, 120), increments(0, 30)]
    );
    let error = rejection(&result.answers[0]);
    assert_eq!(detail_text(&error.detail), PARTIAL_FAILURE_DETAIL);
    assert_eq!(header(error, "retry-after"), "76400");
    // only the first over-limit status reads its window
    assert_eq!(
        rig.cache.reads(),
        vec![String::from("{api_key_tpd:sk-1}:window")]
    );
    assert!(rig.stash.operations().is_empty());
}

// a limiter failure leaves the stash's earlier refunds in place
#[tokio::test]
async fn limiter_failure_keeps_stash() {
    let limiter =
        ScriptedLimiter::with_responses(vec![Ok(daily_refund_response()), Err(LimiterFailure)]);
    let (rig, first) = run_check(
        limiter,
        ScriptedCache::default(),
        data(Some("gpt-4o-mini")),
        usage(120, 3, 0),
        Some(vec![daily_descriptor()]),
    )
    .await;
    assert!(matches!(
        first.answers[0],
        BatchLimitCounterAnswer::CheckAndIncrementBatchCounters { rejection: None }
    ));
    assert_eq!(rig.stash.operations().len(), 1);
    let failure = rig
        .service
        .run(&input(vec![
            BatchLimitCounterCall::CheckAndIncrementBatchCounters {
                data: data(Some("gpt-4o-mini")),
                batch_usage: usage(10, 1, 0),
                descriptors: Some(vec![descriptor(
                    "team",
                    "team-1",
                    Some(WINDOW),
                    Some(1),
                    None,
                )]),
            },
        ]))
        .await;
    assert!(matches!(failure, Err(BatchLimitCountersError::Limits(_))));
    assert_eq!(rig.stash.operations().len(), 1);
}

// rejection_stops_later_calls: a rejection ends the run, while a run without one answers every call
#[tokio::test]
async fn rejection_stops_later_calls() {
    let rig = harness(
        ScriptedLimiter::default(),
        ScriptedStore::default(),
        ScriptedCache::default(),
    );
    let all = rig
        .service
        .run(&input(vec![refund_call(), refund_call()]))
        .await
        .unwrap();
    assert_eq!(all.answers.len(), 2);
    let stopped = rig
        .service
        .run(&input(vec![
            BatchLimitCounterCall::RaiseRateLimitError {
                status: status(
                    BatchCounterCode::OverLimit,
                    "api_key",
                    Some("sk-1"),
                    BatchCounterLimitType::Requests,
                    (5, 0),
                ),
                descriptors: vec![descriptor("api_key", "sk-1", Some(WINDOW), Some(5), None)],
                batch_usage: usage(120, 6, 0),
                limit_type: String::from("requests"),
                requested_model: None,
                window_start: None,
            },
            refund_call(),
        ]))
        .await
        .unwrap();
    assert_eq!(stopped.answers.len(), 1);
}

// non_text_model_is_no_model: data's model that is not text resolves nothing, as docs/deviations/B1045.yaml records
#[tokio::test]
async fn non_text_model_is_no_model() {
    let (_, result) = run_check(
        requests_limiter(BatchCounterCode::OverLimit, 0),
        ScriptedCache::default(),
        non_text_model_data(),
        usage(120, 6, 0),
        Some(vec![descriptor(
            "api_key",
            "sk-1",
            Some(WINDOW),
            Some(5),
            None,
        )]),
    )
    .await;
    let error = rejection(&result.answers[0]);
    assert_eq!(error.model, "");
    assert_eq!(error.llm_provider, "litellm_proxy");
}

// request_model keeps only non-empty text, the same model filter the provider resolver applies
#[test]
fn request_model_requires_non_empty_text() {
    assert!(request_model(&data(None)).is_none());
    assert!(request_model(&null_model_data()).is_none());
    assert!(request_model(&non_text_model_data()).is_none());
    assert!(request_model(&empty_model_data()).is_none());
    assert_eq!(
        request_model(&data(Some("gpt-4o-mini"))),
        Some("gpt-4o-mini")
    );
}

// bounded-inputs: the call list and each call's own collections stay inside ApiLimits
#[tokio::test]
async fn oversized_call_list_is_rejected() {
    let mut rig = harness(
        ScriptedLimiter::default(),
        ScriptedStore::default(),
        ScriptedCache::default(),
    );
    rig.service.limits.max_items = 1;
    let result = rig
        .service
        .run(&input(vec![refund_call(), refund_call()]))
        .await;
    assert!(matches!(
        result,
        Err(BatchLimitCountersError::Boundary(
            ApiLimitError::TooManyItems { .. }
        ))
    ));
}

// bounded-inputs: the caller's scopes are counted before the store or the stash sees them
#[tokio::test]
async fn oversized_scope_list_is_rejected() {
    let mut rig = harness(
        ScriptedLimiter::default(),
        ScriptedStore::default(),
        ScriptedCache::default(),
    );
    rig.service.limits.max_items = 1;
    let result = rig
        .service
        .run(&input(vec![
            BatchLimitCounterCall::ReserveBatchEnqueuedTokens {
                data: data(None),
                batch_usage: usage(30, 2, 0),
                scopes: vec![scope("sk-1", 15), scope("sk-2", 15)],
            },
        ]))
        .await;
    assert!(matches!(
        result,
        Err(BatchLimitCountersError::Boundary(
            ApiLimitError::TooManyItems { .. }
        ))
    ));
    assert!(rig.store.calls().is_empty());
}

// bounded-inputs: the caller's descriptors are counted before the charge clones them
#[tokio::test]
async fn oversized_descriptor_list_is_rejected() {
    let mut rig = harness(
        ScriptedLimiter::default(),
        ScriptedStore::default(),
        ScriptedCache::default(),
    );
    rig.service.limits.max_items = 1;
    let result = rig
        .service
        .run(&input(vec![
            BatchLimitCounterCall::CheckAndIncrementBatchCounters {
                data: data(None),
                batch_usage: usage(10, 1, 0),
                descriptors: Some(vec![
                    descriptor("api_key", "sk-1", Some(WINDOW), Some(5), None),
                    descriptor("team", "team-1", Some(WINDOW), Some(5), None),
                ]),
            },
        ]))
        .await;
    assert!(matches!(
        result,
        Err(BatchLimitCountersError::Boundary(
            ApiLimitError::TooManyItems { .. }
        ))
    ));
    assert!(rig.limiter.calls().is_empty());
}

// bounded-inputs: the usage rows are counted before they expand into project descriptors
#[tokio::test]
async fn oversized_usage_rows_are_rejected() {
    let mut batch_usage = usage(10, 1, 0);
    batch_usage
        .per_model_usage
        .insert(String::from("gpt-4o"), BTreeMap::new());
    batch_usage
        .per_model_usage
        .insert(String::from("gpt-4o-mini"), BTreeMap::new());
    let mut rig = harness(
        ScriptedLimiter::default(),
        ScriptedStore::default(),
        ScriptedCache::default(),
    );
    rig.service.limits.max_items = 1;
    let result = rig
        .service
        .run(&input(vec![
            BatchLimitCounterCall::CheckAndIncrementBatchCounters {
                data: data(None),
                batch_usage,
                descriptors: None,
            },
        ]))
        .await;
    assert!(matches!(
        result,
        Err(BatchLimitCountersError::Boundary(
            ApiLimitError::TooManyItems { .. }
        ))
    ));
    assert!(rig.limiter.project_usages().is_empty());
}

// bounded-inputs: the refund windows are counted before a refund operation is built
#[tokio::test]
async fn oversized_refund_windows_are_rejected() {
    let mut rig = harness(
        ScriptedLimiter::default(),
        ScriptedStore::default(),
        ScriptedCache::default(),
    );
    rig.service.limits.max_items = 1;
    let result = rig
        .service
        .run(&input(vec![BatchLimitCounterCall::BuildTpdRefundOps {
            descriptors: vec![daily_descriptor()],
            tokens: 10,
            reservation_windows: refund_windows(),
        }]))
        .await;
    assert!(matches!(
        result,
        Err(BatchLimitCountersError::Boundary(
            ApiLimitError::TooManyItems { .. }
        ))
    ));
    assert!(rig.stash.operations().is_empty());
}

// bounded-inputs: a charge the limiter derives is counted before the atomic call
#[tokio::test]
async fn oversized_limiter_charge_is_rejected() {
    let mut limiter = ScriptedLimiter::with_responses(Vec::new());
    limiter.base = vec![
        descriptor("api_key", "sk-1", Some(WINDOW), Some(5), None),
        descriptor("team", "team-1", Some(WINDOW), Some(5), None),
    ];
    let mut rig = harness(limiter, ScriptedStore::default(), ScriptedCache::default());
    rig.service.limits.max_items = 1;
    let result = rig
        .service
        .run(&input(vec![
            BatchLimitCounterCall::CheckAndIncrementBatchCounters {
                data: data(None),
                batch_usage: usage(10, 1, 0),
                descriptors: None,
            },
        ]))
        .await;
    assert!(matches!(
        result,
        Err(BatchLimitCountersError::Boundary(
            ApiLimitError::TooManyItems { .. }
        ))
    ));
    assert!(rig.limiter.calls().is_empty());
}

// bounded-inputs: the windows the counter charge returns are counted before refunds expand them
#[tokio::test]
async fn oversized_response_windows_are_rejected() {
    let limiter = ScriptedLimiter::with_responses(vec![Ok(response(
        BatchCounterCode::Ok,
        Vec::new(),
        vec![
            (
                "{api_key_tpd:sk-1}:tokens",
                "1699990000",
                BatchReservationBackend::Redis,
            ),
            (
                "{api_key_tpd:sk-2}:tokens",
                "1699990000",
                BatchReservationBackend::Redis,
            ),
        ],
    ))]);
    let mut rig = harness(limiter, ScriptedStore::default(), ScriptedCache::default());
    rig.service.limits.max_items = 1;
    let result = rig
        .service
        .run(&input(vec![
            BatchLimitCounterCall::CheckAndIncrementBatchCounters {
                data: data(None),
                batch_usage: usage(10, 1, 0),
                descriptors: Some(vec![daily_descriptor()]),
            },
        ]))
        .await;
    assert!(matches!(
        result,
        Err(BatchLimitCountersError::Boundary(
            ApiLimitError::TooManyItems { .. }
        ))
    ));
    assert!(rig.stash.operations().is_empty());
}
