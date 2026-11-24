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
use tokio::time::timeout;

use crate::support::{
    FIXED_MINUTE, Failure, FakeRouter, Fault, KEY_HASH, KEYLESS_FIXTURE, MINUTE_KEY, MODEL,
    NEXT_MINUTE, NEXT_MINUTE_LABEL, NUMBER_KEY_FIXTURE, PLAIN_KEY_FIXTURE, PROD_KEY_FIXTURE,
    RecordingCache, SetWrite, cache_counting, cache_over, clock_at, default_policy,
    failing_harness, handler_over, harness_with, harness_without_router, number, post_call,
    principal, prod_reservation, request,
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
        cache: Arc::<RecordingCache>::clone(&recording),
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

// two projects split the remaining capacity between them
#[tokio::test]
async fn division() {
    let harness = harness_with(
        FakeRouter::answering((Some(100), Some(100)), (Some(40), Some(40))),
        cache_counting(Some(2)),
        None,
    );
    let usage = harness.handler().check_available_usage(MODEL, None).await;
    assert_eq!(usage.available_tpm, Some(30));
    assert_eq!(usage.available_rpm, Some(30));
    assert_eq!(usage.model_tpm, Some(60));
    assert_eq!(usage.model_rpm, Some(60));
}

// one project or no count keeps the remaining capacity, an unknown usage leaves the total, and int() truncates
#[rstest]
#[case::one_project(100, Some(40), Some(1), Some(60), Some(60))]
#[case::no_count(100, Some(40), None, Some(60), Some(60))]
#[case::unknown_usage(100, None, Some(4), Some(25), Some(100))]
#[case::truncation(99, Some(0), Some(2), Some(49), Some(99))]
#[case::zero_share(100, Some(100), Some(5), Some(0), Some(0))]
#[tokio::test]
async fn share_shapes(
    #[case] total: i64,
    #[case] used: Option<i64>,
    #[case] active: Option<usize>,
    #[case] available: Option<i64>,
    #[case] remaining: Option<i64>,
) {
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

// a share spent past the limit clamps to zero and the model total stays as it was
#[tokio::test]
async fn clamping() {
    let harness = harness_with(
        FakeRouter::answering((Some(100), Some(100)), (Some(140), Some(140))),
        Arc::default(),
        Some(prod_reservation(PriorityValue::Percent(1.0))),
    );
    let usage = harness
        .handler()
        .check_available_usage(MODEL, Some("prod"))
        .await;
    assert_eq!(usage.available_tpm, Some(0));
    assert_eq!(usage.available_rpm, Some(0));
    assert_eq!(usage.model_tpm, Some(-40));
}

// an exact spend and a negative weight also clamp to zero
#[rstest]
#[case::spent(100, 100, 1.0_f64, 0)]
#[case::negative_weight(100, 0, -1.0_f64, 100)]
#[tokio::test]
async fn clamping_shapes(
    #[case] total: i64,
    #[case] used: i64,
    #[case] weight: f64,
    #[case] remaining: i64,
) {
    let harness = harness_with(
        FakeRouter::answering((Some(total), Some(total)), (Some(used), Some(used))),
        Arc::default(),
        Some(prod_reservation(PriorityValue::Percent(weight))),
    );
    let usage = harness
        .handler()
        .check_available_usage(MODEL, Some("prod"))
        .await;
    assert_eq!(usage.available_tpm, Some(0));
    assert_eq!(usage.available_rpm, Some(0));
    assert_eq!(usage.model_tpm, Some(remaining));
}

// a negative share truncates toward zero, so the clamp turns it into zero
#[tokio::test]
async fn zero_floor() {
    let harness = harness_with(
        FakeRouter::answering((Some(99), Some(99)), (Some(100), Some(100))),
        cache_counting(Some(3)),
        None,
    );
    let usage = harness.handler().check_available_usage(MODEL, None).await;
    assert_eq!(usage.available_tpm, Some(0));
    assert_eq!(usage.available_rpm, Some(0));
    assert_eq!(usage.model_tpm, Some(-1));
}

// int() has no bound in Python and i64 does: a share past the bound stops at it instead of failing open
#[tokio::test]
async fn overflow_saturates() {
    let harness = harness_with(
        FakeRouter::answering((Some(i64::MAX), Some(i64::MAX)), (Some(0), Some(0))),
        Arc::default(),
        Some(prod_reservation(PriorityValue::Percent(2.0))),
    );
    let usage = harness
        .handler()
        .check_available_usage(MODEL, Some("prod"))
        .await;
    assert_eq!(usage.available_tpm, Some(i64::MAX));
    assert_eq!(usage.available_rpm, Some(i64::MAX));
}

// a share far below the bound is negative, so the clamp zeroes it and the model total keeps its value
#[tokio::test]
async fn negative_overflow_clamps_to_zero() {
    let harness = harness_with(
        FakeRouter::answering((Some(i64::MIN), Some(i64::MIN)), (Some(0), Some(0))),
        Arc::default(),
        Some(prod_reservation(PriorityValue::Percent(2.0))),
    );
    let usage = harness
        .handler()
        .check_available_usage(MODEL, Some("prod"))
        .await;
    assert_eq!(usage.available_tpm, Some(0));
    assert_eq!(usage.available_rpm, Some(0));
    assert_eq!(usage.model_tpm, Some(i64::MIN));
}

// a failed step answers five Nones instead of failing the request
#[tokio::test]
async fn exception_returns_nones() {
    let harness = failing_harness(Failure::InfoRead);
    let usage = harness
        .handler()
        .check_available_usage(MODEL, Some("prod"))
        .await;
    assert_eq!(usage, AvailableUsage::default());
}

// every read failure, a missing router, a zero count and a non-finite share fail open to five Nones
#[rstest]
#[case::info_read(Failure::InfoRead)]
#[case::usage_read(Failure::UsageRead)]
#[case::missing_router(Failure::MissingRouter)]
#[case::cache_read(Failure::CacheRead)]
#[case::no_active_projects(Failure::NoActiveProjects)]
#[case::infinite_weight(Failure::InfiniteWeight)]
#[tokio::test]
async fn every_failed_step_returns_nones(#[case] failure: Failure) {
    let harness = failing_harness(failure);
    let usage = harness
        .handler()
        .check_available_usage(MODEL, Some("prod"))
        .await;
    assert_eq!(usage, AvailableUsage::default());
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
    let handler = harness.handler();
    let error = handler
        .async_pre_call_hook(&request(Some(MODEL), &key))
        .await
        .expect_err("a spent token share and a spent request share both refuse the request");
    handler.tasks.close();
    handler.tasks.wait().await;
    // the refused key never joins the minute set, so it cannot count itself as an active project
    assert!(harness.cache.set_writes().is_empty());
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

// an f-string renders a value LiteLLM could not compute as the word None, and the detail keeps that
#[tokio::test]
async fn error_detail_names_unknown_values() {
    let harness = harness_with(
        FakeRouter::answering((Some(0), None), (Some(0), Some(0))),
        Arc::default(),
        None,
    );
    let key = principal(PLAIN_KEY_FIXTURE);
    let error = harness
        .handler()
        .async_pre_call_hook(&request(Some(MODEL), &key))
        .await
        .expect_err("a spent token share with no request limit refuses the request");
    assert_eq!(
        rejection(error).message,
        format!("Key={KEY_HASH} over available TPM=0. Model TPM=0, Active keys=None")
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

// the post-call hook reads the priority from the key, so a reserved key gets half the model capacity
#[tokio::test]
async fn post_call_headers_use_the_key_priority() {
    let router =
        FakeRouter::answering((Some(100), Some(100)), (Some(0), Some(0))).with_model_info(MODEL);
    let harness = harness_with(
        router,
        cache_counting(Some(1)),
        Some(prod_reservation(PriorityValue::Percent(0.5))),
    );
    let key = principal(PROD_KEY_FIXTURE);
    let mut headers = AdditionalHeaders::new();
    harness
        .handler()
        .async_post_call_success_hook(&post_call("deployment-prod", &key), &mut headers)
        .await;
    assert_eq!(headers.get(PROJECT_TOKENS_HEADER), Some(&number(50)));
    assert_eq!(headers.get(PROJECT_REQUESTS_HEADER), Some(&number(50)));
    assert_eq!(headers.get(MODEL_TOKENS_HEADER), Some(&number(100)));
    assert_eq!(headers.get(ACTIVE_PROJECTS_HEADER), Some(&number(1)));
}

// the six names as LiteLLM spells them, so renaming a constant fails here rather than on the wire
#[test]
fn header_names_match_litellm() {
    assert_eq!(MODEL_GROUP_HEADER, "x-litellm-model_group");
    assert_eq!(
        PROJECT_TOKENS_HEADER,
        "x-ratelimit-remaining-litellm-project-tokens"
    );
    assert_eq!(
        PROJECT_REQUESTS_HEADER,
        "x-ratelimit-remaining-litellm-project-requests"
    );
    assert_eq!(MODEL_TOKENS_HEADER, "x-ratelimit-remaining-model-tokens");
    assert_eq!(
        MODEL_REQUESTS_HEADER,
        "x-ratelimit-remaining-model-requests"
    );
    assert_eq!(
        ACTIVE_PROJECTS_HEADER,
        "x-ratelimit-current-active-projects"
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
    let gated = Arc::new(RecordingCache::new(Fault::None, Some(Arc::default())));
    let port: Arc<dyn MinuteSetCache> = Arc::<RecordingCache>::clone(&gated);
    let router = FakeRouter::answering((Some(100), Some(100)), (Some(0), Some(0)));
    let llm_router: Option<Arc<dyn DynamicLimitRouter>> = Some(Arc::new(router));
    let handler = handler_over(port, llm_router, default_policy());
    let key = principal(PLAIN_KEY_FIXTURE);
    let call = request(Some(MODEL), &key);
    timeout(Duration::from_secs(5), handler.async_pre_call_hook(&call))
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

// the set add fails inside its task: the request already passed, and the failed write is still attempted
#[tokio::test]
async fn failed_set_write_does_not_refuse_the_request() {
    let cache = RecordingCache::new(Fault::Write, None);
    let port: Arc<dyn MinuteSetCache> = Arc::new(cache.clone());
    let router = FakeRouter::answering((Some(100), Some(100)), (Some(0), Some(0)));
    let llm_router: Option<Arc<dyn DynamicLimitRouter>> = Some(Arc::new(router));
    let handler = handler_over(port, llm_router, default_policy());
    let key = principal(PLAIN_KEY_FIXTURE);
    handler
        .async_pre_call_hook(&request(Some(MODEL), &key))
        .await
        .expect("a failed registration write does not refuse the request");
    handler.tasks.close();
    handler.tasks.wait().await;
    assert_eq!(
        cache.set_writes(),
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
async fn registration_runs_when_one_quota_is_known() {
    let harness = harness_with(
        FakeRouter::answering((Some(100), None), (Some(0), Some(0))),
        Arc::default(),
        None,
    );
    let handler = harness.handler();
    let key = principal(PLAIN_KEY_FIXTURE);
    handler
        .async_pre_call_hook(&request(Some(MODEL), &key))
        .await
        .expect("a model group with only a token limit passes");
    handler.tasks.close();
    handler.tasks.wait().await;
    assert_eq!(harness.cache.set_writes().len(), 1);
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
        Some(prod_reservation(PriorityValue::Percent(0.5))),
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
        Some(prod_reservation(PriorityValue::Percent(0.5))),
    );
    let usage = harness.handler().check_available_usage(MODEL, None).await;
    assert_eq!(usage.available_tpm, Some(100));
}

#[rstest]
#[case::percent_share(PriorityKind::Percent, 0.5_f64)]
#[case::rpm_share(PriorityKind::Rpm, 50.0_f64)]
#[case::tpm_share(PriorityKind::Tpm, 50.0_f64)]
#[tokio::test]
async fn weight_uses_the_reserved_share(#[case] kind: PriorityKind, #[case] value: f64) {
    let harness = harness_with(
        FakeRouter::answering((Some(100), Some(100)), (Some(0), Some(0))),
        cache_counting(Some(1)),
        Some(prod_reservation(PriorityValue::Reservation(
            PriorityReservation { kind, value },
        ))),
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
        Some(prod_reservation(PriorityValue::Percent(0.5))),
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

// LiteLLM documents the priority as a string, so a number counts as no priority and the weight stays 1
#[tokio::test]
async fn a_non_text_priority_keeps_the_default_weight() {
    let harness = harness_with(
        FakeRouter::answering((Some(1), Some(100)), (Some(0), Some(0))),
        cache_counting(Some(1)),
        Some(prod_reservation(PriorityValue::Percent(0.5))),
    );
    let key = principal(NUMBER_KEY_FIXTURE);
    harness
        .handler()
        .async_pre_call_hook(&request(Some(MODEL), &key))
        .await
        .expect("a number priority names no reservation, so the whole token stays available");
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
        DynamicLimitError::Header(source) => panic!("unexpected header failure: {source}"),
    }
}
