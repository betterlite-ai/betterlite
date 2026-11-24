use std::iter::once;
use std::sync::Arc;
use std::time::Duration;

use betterlite_error::rate_limit::{ProxyRateLimitError, RateLimitDimension};
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use betterlite_rate_limit::dynamic_limits::DynamicRateLimiterCache;
use betterlite_rate_limit::dynamic_limits::headers::{
    ACTIVE_PROJECTS_HEADER, MODEL_GROUP_HEADER, MODEL_REQUESTS_HEADER, MODEL_TOKENS_HEADER,
    PROJECT_REQUESTS_HEADER, PROJECT_TOKENS_HEADER,
};
use betterlite_rate_limit::dynamic_limits::ports::{DynamicLimitRouter, MinuteSetCache};
use betterlite_rate_limit::dynamic_limits::quota::AvailableUsage;
use betterlite_rate_limit::dynamic_limits::rejection::DynamicLimitError;
use betterlite_rate_limit::provider_attribution::{
    PriorityKind, PriorityReservation, PriorityValue,
};
use betterlite_redact::structured::Payload;
use rstest::rstest;

use crate::support::{
    FIXED_MINUTE, Failure, FakeRouter, GatedCache, KEY_HASH, KEYLESS_FIXTURE, MINUTE_KEY, MODEL,
    NEXT_MINUTE, NEXT_MINUTE_LABEL, PLAIN_KEY_FIXTURE, PROD_KEY_FIXTURE, RecordingCache, SetWrite,
    cache_counting, cache_over, clock_at, default_policy, failing_harness, handler_over,
    harness_with, harness_without_router, number, post_call, principal, prod_reservation, request,
};

#[tokio::test]
async fn cache_returns_none_for_missing() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache_over(&recording);
    assert_eq!(cache.async_get_cache(MODEL).await, Ok(None));
    assert_eq!(recording.read_keys(), vec![String::from(MINUTE_KEY)]);
}

#[tokio::test]
async fn count_for_existing_key() {
    let recording = Arc::new(RecordingCache::holding(
        MINUTE_KEY,
        &[KEY_HASH, "key-b", "key-c"],
    ));
    let cache = cache_over(&recording);
    assert_eq!(cache.async_get_cache(MODEL).await, Ok(Some(3)));
    // the pitfall: a set that exists but holds nothing counts zero, it is not a miss
    recording.store(MINUTE_KEY, &[]);
    assert_eq!(cache.async_get_cache(MODEL).await, Ok(Some(0)));
}

#[tokio::test]
async fn minute_key_reads_the_utc_hour_and_minute() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache_over(&recording);
    assert_eq!(cache.async_get_cache(MODEL).await, Ok(None));
    let later = DynamicRateLimiterCache {
        cache: Arc::clone(&recording),
        clock: clock_at(NEXT_MINUTE),
    };
    assert_eq!(later.async_get_cache(MODEL).await, Ok(None));
    assert_eq!(
        recording.read_keys(),
        vec![
            format!("{FIXED_MINUTE}:{MODEL}"),
            format!("{NEXT_MINUTE_LABEL}:{MODEL}"),
        ]
    );
}

#[tokio::test]
async fn set_add_writes_the_minute_set_with_sixty_seconds() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache_over(&recording);
    cache
        .async_set_cache_sadd(MODEL, &[String::from(KEY_HASH)])
        .await
        .expect("the recording cache stores every set add");
    assert_eq!(
        recording.set_writes(),
        vec![SetWrite {
            key: String::from(MINUTE_KEY),
            values: vec![String::from(KEY_HASH)],
            ttl: 60,
        }]
    );
}

// two projects split the remaining capacity, one or no count keeps it, and int() truncates toward zero
#[tokio::test]
async fn division() {
    let cases: [(i64, Option<i64>, Option<usize>, Option<i64>, Option<i64>); 5] = [
        (100, Some(40), Some(2), Some(30), Some(60)),
        (100, Some(40), Some(1), Some(60), Some(60)),
        (100, Some(40), None, Some(60), Some(60)),
        (100, None, Some(4), Some(25), Some(100)),
        (99, Some(0), Some(2), Some(49), Some(99)),
    ];
    for (total, used, active, available, remaining) in cases {
        let harness = harness_with(
            FakeRouter::answering((Some(total), Some(total)), (used, used)),
            cache_counting(active),
            None,
        );
        let usage = harness.handler().check_available_usage(MODEL, None).await;
        assert_eq!(usage.available_tpm, available);
        assert_eq!(usage.available_rpm, available);
        assert_eq!(usage.model_tpm, remaining);
        assert_eq!(usage.model_rpm, remaining);
    }
}

// a spent share, a negative share and a negative weight all clamp to zero, and the model total stays as it was
#[tokio::test]
async fn clamping() {
    let cases: [(i64, i64, f64, i64); 3] = [
        (100, 140, 1.0, -40),
        (100, 100, 1.0, 0),
        (100, 0, -1.0, 100),
    ];
    for (total, used, weight, remaining) in cases {
        let harness = harness_with(
            FakeRouter::answering((Some(total), Some(total)), (Some(used), Some(used))),
            Arc::default(),
            prod_reservation(PriorityValue::Percent(weight)),
        );
        let usage = harness
            .handler()
            .check_available_usage(MODEL, Some("prod"))
            .await;
        assert_eq!(usage.available_tpm, Some(0));
        assert_eq!(usage.available_rpm, Some(0));
        assert_eq!(usage.model_tpm, Some(remaining));
    }
}

// a negative share truncates toward zero and a zero share stays zero, so both clamp to zero
#[tokio::test]
async fn zero_floor() {
    let cases: [(i64, i64, usize, i64); 2] = [(99, 100, 3, -1), (100, 100, 5, 0)];
    for (total, used, active, remaining) in cases {
        let harness = harness_with(
            FakeRouter::answering((Some(total), Some(total)), (Some(used), Some(used))),
            cache_counting(Some(active)),
            None,
        );
        let usage = harness.handler().check_available_usage(MODEL, None).await;
        assert_eq!(usage.available_tpm, Some(0));
        assert_eq!(usage.available_rpm, Some(0));
        assert_eq!(usage.model_tpm, Some(remaining));
    }
}

// every read failure, a missing router, a zero count and a non-finite share fail open to five Nones
#[tokio::test]
async fn exception_returns_nones() {
    for failure in [
        Failure::InfoRead,
        Failure::UsageRead,
        Failure::MissingRouter,
        Failure::CacheRead,
        Failure::NoActiveProjects,
        Failure::InfiniteWeight,
    ] {
        let harness = failing_harness(failure);
        let usage = harness
            .handler()
            .check_available_usage(MODEL, Some("prod"))
            .await;
        assert_eq!(usage, AvailableUsage::default());
    }
}

#[tokio::test]
async fn error_detail_complete() {
    // both shares are spent, so the token check running first is what names the rate limit type
    let harness = harness_with(
        FakeRouter::answering((Some(100), Some(100)), (Some(100), Some(100))),
        cache_counting(Some(2)),
        None,
    );
    let key = principal(PLAIN_KEY_FIXTURE);
    let error = harness
        .handler()
        .async_pre_call_hook(&request(Some(MODEL), &key))
        .await
        .expect_err("a spent token share and a spent request share both refuse the request");
    let rate_limit = rejection(error);
    let expected = format!("Key={KEY_HASH} over available TPM=0. Model TPM=0, Active keys=2");
    assert_eq!(rate_limit.message, expected);
    assert_eq!(rate_limit.rate_limit_type, Some(RateLimitDimension::Tokens));
    assert_eq!(rate_limit.model, MODEL);
    assert_eq!(rate_limit.llm_provider, "openai");
    assert_eq!(
        rate_limit.detail,
        Payload::Object(once((String::from("error"), Payload::String(expected))).collect())
    );
}

#[tokio::test]
async fn request_check_raises_requests() {
    let harness = harness_with(
        FakeRouter::answering((None, Some(100)), (Some(0), Some(100))),
        cache_counting(Some(1)),
        None,
    );
    let key = principal(PLAIN_KEY_FIXTURE);
    let error = harness
        .handler()
        .async_pre_call_hook(&request(Some(MODEL), &key))
        .await
        .expect_err("a spent request share refuses the request when no token limit is set");
    let rate_limit = rejection(error);
    assert_eq!(
        rate_limit.rate_limit_type,
        Some(RateLimitDimension::Requests)
    );
    assert_eq!(
        rate_limit.message,
        format!("Key={KEY_HASH} over available RPM=0. Model RPM=0, Active keys=1")
    );
}

#[tokio::test]
async fn post_call_headers() {
    let router =
        FakeRouter::answering((Some(100), Some(100)), (Some(40), Some(10))).with_model_info(MODEL);
    let harness = harness_with(router, cache_counting(Some(2)), None);
    let key = principal(PLAIN_KEY_FIXTURE);
    let mut headers =
        AdditionalHeaders::from([(String::from("x-previous"), Json::Text(String::from("kept")))]);
    harness
        .handler()
        .async_post_call_success_hook(&post_call("deployment-1", &key), &mut headers)
        .await;
    assert_eq!(
        harness.router.calls(),
        vec![
            String::from("model:deployment-1"),
            String::from("info:gpt-4"),
            String::from("usage:gpt-4"),
        ]
    );
    assert_eq!(headers.len(), 7);
    assert_eq!(
        headers.get(MODEL_GROUP_HEADER),
        Some(&Json::Text(String::from(MODEL)))
    );
    assert_eq!(headers.get(PROJECT_TOKENS_HEADER), Some(&number(30)));
    assert_eq!(headers.get(PROJECT_REQUESTS_HEADER), Some(&number(45)));
    assert_eq!(headers.get(MODEL_TOKENS_HEADER), Some(&number(60)));
    assert_eq!(headers.get(MODEL_REQUESTS_HEADER), Some(&number(90)));
    assert_eq!(headers.get(ACTIVE_PROJECTS_HEADER), Some(&number(2)));
    assert_eq!(
        headers.get("x-previous"),
        Some(&Json::Text(String::from("kept")))
    );
}

#[tokio::test]
async fn post_call_headers_carry_null_for_unknown_values() {
    let router = FakeRouter::default().with_model_info(MODEL);
    let key = principal(PLAIN_KEY_FIXTURE);
    let mut headers = AdditionalHeaders::new();
    harness_with(router, Arc::default(), None)
        .handler()
        .async_post_call_success_hook(&post_call("deployment-2", &key), &mut headers)
        .await;
    assert_eq!(headers.len(), 6);
    assert_eq!(
        headers.get(MODEL_GROUP_HEADER),
        Some(&Json::Text(String::from(MODEL)))
    );
    assert_eq!(headers.get(PROJECT_TOKENS_HEADER), Some(&Json::Null));
    assert_eq!(headers.get(PROJECT_REQUESTS_HEADER), Some(&Json::Null));
    assert_eq!(headers.get(MODEL_TOKENS_HEADER), Some(&Json::Null));
    assert_eq!(headers.get(MODEL_REQUESTS_HEADER), Some(&Json::Null));
    assert_eq!(headers.get(ACTIVE_PROJECTS_HEADER), Some(&Json::Null));
}

#[tokio::test]
async fn model_info_missing_writes_no_headers() {
    let harness = harness_with(
        FakeRouter::answering((Some(100), Some(100)), (Some(0), Some(0))),
        Arc::default(),
        None,
    );
    let key = principal(PLAIN_KEY_FIXTURE);
    let mut headers = AdditionalHeaders::new();
    harness
        .handler()
        .async_post_call_success_hook(&post_call("unknown", &key), &mut headers)
        .await;
    assert!(headers.is_empty());
}

#[tokio::test]
async fn post_call_without_a_router_writes_no_headers() {
    let harness = harness_without_router(FakeRouter::default());
    let key = principal(PLAIN_KEY_FIXTURE);
    let mut headers = AdditionalHeaders::new();
    harness
        .handler()
        .async_post_call_success_hook(&post_call("deployment-3", &key), &mut headers)
        .await;
    assert!(headers.is_empty());
}

#[tokio::test]
async fn missing_model_passes_and_writes_nothing() {
    let harness = harness_with(
        FakeRouter::answering((Some(100), Some(100)), (Some(0), Some(0))),
        cache_counting(Some(1)),
        None,
    );
    let key = principal(PLAIN_KEY_FIXTURE);
    harness
        .handler()
        .async_pre_call_hook(&request(None, &key))
        .await
        .expect("a request without a model passes");
    assert!(harness.router.calls().is_empty());
    assert!(harness.cache.read_keys().is_empty());
    assert!(harness.cache.set_writes().is_empty());
}

#[tokio::test]
async fn pre_call_returns_without_waiting_for_the_set_write() {
    let gated = Arc::new(GatedCache::default());
    let port: Arc<dyn MinuteSetCache> = Arc::clone(&gated);
    let router = FakeRouter::answering((Some(100), Some(100)), (Some(0), Some(0)));
    let llm_router: Option<Arc<dyn DynamicLimitRouter>> = Some(Arc::new(router));
    let handler = handler_over(port, llm_router, default_policy());
    let key = principal(PLAIN_KEY_FIXTURE);
    let call = request(Some(MODEL), &key);
    tokio::time::timeout(Duration::from_secs(5), handler.async_pre_call_hook(&call))
        .await
        .expect("the hook returns without the set write")
        .expect("the request passes");
    gated.release();
    handler.tasks.close();
    handler.tasks.wait().await;
    assert_eq!(
        gated.set_writes(),
        vec![SetWrite {
            key: String::from(MINUTE_KEY),
            values: vec![String::from(KEY_HASH)],
            ttl: 60,
        }]
    );
}

#[tokio::test]
async fn no_registration_when_both_quotas_are_unknown() {
    let harness = harness_with(FakeRouter::default(), cache_counting(Some(2)), None);
    let handler = harness.handler();
    let key = principal(PLAIN_KEY_FIXTURE);
    handler
        .async_pre_call_hook(&request(Some(MODEL), &key))
        .await
        .expect("a model group with no limits passes");
    handler.tasks.close();
    handler.tasks.wait().await;
    assert!(harness.cache.set_writes().is_empty());
}

#[tokio::test]
async fn registration_uses_the_default_key_without_an_identifier() {
    let harness = harness_with(
        FakeRouter::answering((Some(100), Some(100)), (Some(0), Some(0))),
        Arc::default(),
        None,
    );
    let handler = harness.handler();
    let key = principal(KEYLESS_FIXTURE);
    handler
        .async_pre_call_hook(&request(Some(MODEL), &key))
        .await
        .expect("a key without an identifier still passes");
    handler.tasks.close();
    handler.tasks.wait().await;
    assert_eq!(
        harness.cache.set_writes(),
        vec![SetWrite {
            key: String::from(MINUTE_KEY),
            values: vec![String::from("default_key")],
            ttl: 60,
        }]
    );
}

#[tokio::test]
async fn weight_is_one_without_a_reservation() {
    let harness = harness_with(
        FakeRouter::answering((Some(100), Some(100)), (Some(0), Some(0))),
        cache_counting(Some(1)),
        None,
    );
    let usage = harness
        .handler()
        .check_available_usage(MODEL, Some("prod"))
        .await;
    assert_eq!(usage.available_tpm, Some(100));
}

#[tokio::test]
async fn weight_is_one_for_an_unknown_priority() {
    let harness = harness_with(
        FakeRouter::answering((Some(100), Some(100)), (Some(0), Some(0))),
        cache_counting(Some(1)),
        prod_reservation(PriorityValue::Percent(0.5)),
    );
    let usage = harness
        .handler()
        .check_available_usage(MODEL, Some("other"))
        .await;
    assert_eq!(usage.available_tpm, Some(100));
}

#[tokio::test]
async fn weight_is_one_without_a_priority() {
    let harness = harness_with(
        FakeRouter::answering((Some(100), Some(100)), (Some(0), Some(0))),
        cache_counting(Some(1)),
        prod_reservation(PriorityValue::Percent(0.5)),
    );
    let usage = harness.handler().check_available_usage(MODEL, None).await;
    assert_eq!(usage.available_tpm, Some(100));
}

#[rstest]
#[case::percent_share(PriorityKind::Percent, 0.5)]
#[case::rpm_share(PriorityKind::Rpm, 50.0)]
#[case::tpm_share(PriorityKind::Tpm, 50.0)]
#[tokio::test]
async fn weight_uses_the_reserved_share(#[case] kind: PriorityKind, #[case] value: f64) {
    let harness = harness_with(
        FakeRouter::answering((Some(100), Some(100)), (Some(0), Some(0))),
        cache_counting(Some(1)),
        prod_reservation(PriorityValue::Reservation(PriorityReservation {
            kind,
            value,
        })),
    );
    let usage = harness
        .handler()
        .check_available_usage(MODEL, Some("prod"))
        .await;
    assert_eq!(usage.available_tpm, Some(50));
}

#[tokio::test]
async fn pre_call_reads_the_priority_from_key_metadata() {
    let harness = harness_with(
        FakeRouter::answering((Some(1), Some(100)), (Some(0), Some(0))),
        cache_counting(Some(1)),
        prod_reservation(PriorityValue::Percent(0.5)),
    );
    let handler = harness.handler();
    let reserved = principal(PROD_KEY_FIXTURE);
    let error = handler
        .async_pre_call_hook(&request(Some(MODEL), &reserved))
        .await
        .expect_err("half of one token truncates to zero");
    assert_eq!(
        rejection(error).rate_limit_type,
        Some(RateLimitDimension::Tokens)
    );
    let plain = principal(PLAIN_KEY_FIXTURE);
    handler
        .async_pre_call_hook(&request(Some(MODEL), &plain))
        .await
        .expect("a key with no declared priority keeps the whole token");
}

#[tokio::test]
async fn update_variables_sets_the_router() {
    let harness = harness_without_router(FakeRouter::answering(
        (Some(100), Some(100)),
        (Some(0), Some(0)),
    ));
    let mut handler = harness.handler();
    assert_eq!(
        handler.check_available_usage(MODEL, None).await,
        AvailableUsage::default()
    );
    let router: Arc<dyn DynamicLimitRouter> = Arc::new(harness.router);
    handler.update_variables(router);
    assert_eq!(
        handler
            .check_available_usage(MODEL, None)
            .await
            .available_tpm,
        Some(100)
    );
}

fn rejection(error: DynamicLimitError) -> Box<ProxyRateLimitError> {
    match error {
        DynamicLimitError::Rejected(rate_limit) => rate_limit,
        DynamicLimitError::Header(error) => panic!("unexpected header failure: {error}"),
    }
}
