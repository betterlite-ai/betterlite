use std::sync::Arc;

use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::model_rate_limits::error::{ModelRateLimitError, RateLimitChannel, RateLimitExceeded};
use crate::model_rate_limits::service::ModelRateLimits;

use super::support::{
    FakeCache, MINUTE, RemoteMode, check_async, exceeded, number, read_count, request, rpm_cache,
    rpm_key, run_case_table, service, spec, tpm_cache, tpm_key, with_extra, with_info, with_param,
};

pub(super) fn run(name: &str) {
    run_case_table(name, &CASES);
}

const CASES: [(&str, fn()); 17] = [
    ("tpm_one_below_limit_allows", || {
        super::runtime().block_on(tpm_one_below_limit_allows());
    }),
    ("tpm_over_limit_by_one_rejects", || {
        super::runtime().block_on(tpm_over_limit_by_one_rejects());
    }),
    ("rpm_at_limit_allows", || {
        super::runtime().block_on(rpm_at_limit_allows());
    }),
    ("rpm_over_limit_by_one_rejects", || {
        super::runtime().block_on(rpm_over_limit_by_one_rejects());
    }),
    ("zero_tpm_limit_rejects_zero_usage", || {
        super::runtime().block_on(zero_tpm_limit_rejects_zero_usage());
    }),
    ("negative_tpm_limit_rejects_zero_usage", || {
        super::runtime().block_on(negative_tpm_limit_rejects_zero_usage());
    }),
    ("missing_remote_value_allows", || {
        super::runtime().block_on(missing_remote_value_allows());
    }),
    ("non_numeric_limit_names_the_field", || {
        non_numeric_limit_names_the_field();
    }),
    ("top_level_limit_wins", || {
        super::runtime().block_on(top_level_limit_wins());
    }),
    ("params_limit_beats_model_info", || {
        super::runtime().block_on(params_limit_beats_model_info());
    }),
    ("model_info_limit_used_last", || {
        super::runtime().block_on(model_info_limit_used_last());
    }),
    ("missing_model_id_counts_under_none", || {
        super::runtime().block_on(missing_model_id_counts_under_none());
    }),
    ("empty_model_id_builds_keys", || {
        super::runtime().block_on(empty_model_id_builds_keys());
    }),
    ("breaker_open_keeps_local_below_limit", || {
        super::runtime().block_on(breaker_open_keeps_local_below_limit());
    }),
    ("sync_facade_ignores_remote", || {
        super::runtime().block_on(sync_facade_ignores_remote());
    }),
    ("coexist_warning_needs_both_limit_kinds", || {
        coexist_warning_needs_both_limit_kinds();
    }),
    (
        "error_constants_match_litellm",
        error_constants_match_litellm,
    ),
];

fn model() -> &'static str {
    "openai/gpt-4o"
}

async fn tpm_one_below_limit_allows() {
    let cache = Arc::new(FakeCache::new().seed_local(&tpm_key(model()), 4));
    let deployment = with_param(spec("gpt-4o", "a", model()), "tpm", 5);
    check_async(&cache, &deployment).await.unwrap();
}

async fn tpm_over_limit_by_one_rejects() {
    let cache = Arc::new(FakeCache::new().seed_local(&tpm_key(model()), 6));
    let deployment = with_param(spec("gpt-4o", "a", model()), "tpm", 5);
    let error = check_async(&cache, &deployment).await.unwrap_err();
    assert_eq!(exceeded(error).current, 6);
}

async fn rpm_at_limit_allows() {
    let cache = Arc::new(rpm_cache(1, 1));
    let deployment = with_param(spec("gpt-4o", "a", model()), "rpm", 2);
    check_async(&cache, &deployment).await.unwrap();
    assert_eq!(cache.local_value(&rpm_key(model())), Some(2));
    assert_eq!(cache.remote_value(&rpm_key(model())), Some(2));
}

async fn rpm_over_limit_by_one_rejects() {
    let cache = Arc::new(rpm_cache(2, 2));
    let deployment = with_param(spec("gpt-4o", "a", model()), "rpm", 2);
    let error = check_async(&cache, &deployment).await.unwrap_err();
    assert_eq!(exceeded(error).current, 3);
    assert_eq!(cache.local_value(&rpm_key(model())), Some(3));
}

async fn zero_tpm_limit_rejects_zero_usage() {
    let cache = Arc::new(FakeCache::new().seed_local(&tpm_key(model()), 0));
    let deployment = with_param(spec("gpt-4o", "a", model()), "tpm", 0);
    let error = check_async(&cache, &deployment).await.unwrap_err();
    assert_eq!(exceeded(error).limit, 0);
}

async fn negative_tpm_limit_rejects_zero_usage() {
    let cache = Arc::new(FakeCache::new().seed_local(&tpm_key(model()), 0));
    let deployment = with_param(spec("gpt-4o", "a", model()), "tpm", -1);
    let error = check_async(&cache, &deployment).await.unwrap_err();
    assert_eq!(exceeded(error).limit, -1);
}

async fn missing_remote_value_allows() {
    let cache = Arc::new(FakeCache::new());
    let check = service(Arc::clone(&cache), None);
    let deployment = with_param(spec("gpt-4o", "a", model()), "tpm", 5);
    check
        .async_pre_call_check(&deployment, &mut request())
        .await
        .unwrap();
    assert_eq!(
        cache.remote_reads.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

fn non_numeric_limit_names_the_field() {
    let check = service(Arc::new(FakeCache::new()), None);
    let deployment = with_info(
        spec("gpt-4o", "a", model()),
        "tpm",
        Json::Text(String::from("5")),
    );
    assert!(matches!(
        check.pre_call_check(&deployment),
        Err(ModelRateLimitError::Limit { field: "tpm" })
    ));
}

async fn top_level_limit_wins() {
    let cache = Arc::new(FakeCache::new().seed_local(&tpm_key(model()), 1));
    let check = service(Arc::clone(&cache), None);
    let deployment = with_info(
        with_param(
            with_extra(
                spec("gpt-4o", "a", model()),
                "tpm",
                Json::Number(Number::from(1_i64)),
            ),
            "tpm",
            99,
        ),
        "tpm",
        Json::Number(Number::from(99_i64)),
    );
    let error = check
        .async_pre_call_check(&deployment, &mut request())
        .await
        .unwrap_err();
    assert_eq!(exceeded(error).limit, 1);
}

async fn params_limit_beats_model_info() {
    let cache = Arc::new(FakeCache::new().seed_local(&tpm_key(model()), 2));
    let param = with_param(spec("gpt-4o", "a", model()), "tpm", 2);
    let deployment = with_info(param, "tpm", number(9));
    let error = check_async(&cache, &deployment).await.unwrap_err();
    assert_eq!(exceeded(error).limit, 2);
}

async fn model_info_limit_used_last() {
    let cache = Arc::new(FakeCache::new().seed_local(&tpm_key(model()), 3));
    let deployment = with_info(spec("gpt-4o", "a", model()), "tpm", number(3));
    let error = check_async(&cache, &deployment).await.unwrap_err();
    assert_eq!(exceeded(error).limit, 3);
}

async fn missing_model_id_counts_under_none() {
    let key = format!("None:{}:tpm:{MINUTE}", model());
    let cache = Arc::new(FakeCache::new().seed_local(&key, 9));
    let check = service(Arc::clone(&cache), None);
    let mut deployment = with_param(spec("gpt-4o", "a", model()), "tpm", 5);
    deployment.model_info.shift_remove("id");
    let error = check
        .async_pre_call_check(&deployment, &mut request())
        .await
        .unwrap_err();
    assert_eq!(exceeded(error).current, 9);
}

async fn empty_model_id_builds_keys() {
    let key = format!(":{}:tpm:{MINUTE}", model());
    let cache = Arc::new(FakeCache::new().seed_local(&key, 5));
    let check = service(Arc::clone(&cache), None);
    let deployment = with_param(spec("gpt-4o", "", model()), "tpm", 5);
    let error = check
        .async_pre_call_check(&deployment, &mut request())
        .await
        .unwrap_err();
    assert_eq!(exceeded(error).model_id, "");
}

async fn breaker_open_keeps_local_below_limit() {
    let cache = Arc::new(tpm_cache(4, 100).mode(RemoteMode::Breaker));
    let deployment = with_param(spec("gpt-4o", "a", model()), "tpm", 5);
    check_async(&cache, &deployment).await.unwrap();
    assert_eq!(read_count(&cache), 1);
}

async fn sync_facade_ignores_remote() {
    let cache = Arc::new(FakeCache::new().seed_remote(&tpm_key(model()), 10));
    let check = service(Arc::clone(&cache), None);
    let deployment = with_param(spec("gpt-4o", "a", model()), "tpm", 5);
    check.pre_call_check(&deployment).unwrap();
    assert_eq!(
        cache.remote_reads.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

fn coexist_warning_needs_both_limit_kinds() {
    let check = ModelRateLimits::new(Arc::new(FakeCache::new()), None, super::support::clock());
    let io_only = with_info(
        spec("gpt-4o", "a", model()),
        "itpm",
        Json::Number(Number::from(10_i64)),
    );
    assert!(!check.logs_coexist_warning(&io_only).unwrap());
    let tpm_only = with_param(spec("gpt-4o", "b", model()), "tpm", 5);
    assert!(!check.logs_coexist_warning(&tpm_only).unwrap());
    let both = with_param(io_only.clone(), "tpm", 5);
    assert!(check.logs_coexist_warning(&both).unwrap());
    assert!(!check.logs_coexist_warning(&both).unwrap());
}

fn error_constants_match_litellm() {
    let exceeded = RateLimitExceeded {
        channel: RateLimitChannel::Tpm,
        limit: 5,
        current: 5,
        model_id: String::from("a"),
        model: String::from(model()),
        model_group: String::from("gpt-4o"),
        num_retries: Some(0),
    };
    assert_eq!(RateLimitExceeded::STATUS_CODE, 429);
    assert_eq!(RateLimitExceeded::CODE, "429");
    assert_eq!(RateLimitExceeded::ERROR_TYPE, "throttling_error");
    assert_eq!(RateLimitExceeded::RETRY_AFTER_SECONDS, 60);
    assert_eq!(RateLimitExceeded::LLM_PROVIDER, "");
    assert_eq!(
        format!("litellm.RateLimitError: {exceeded}"),
        "litellm.RateLimitError: Model rate limit exceeded. TPM limit=5, current usage=5"
    );
}
