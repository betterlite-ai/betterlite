#![cfg(test)]

mod support;

use betterlite_config::sections::settings::JsonValue;
use betterlite_error::rate_limit::RateLimitDimension;
use rstest::rstest;

use self::support::{
    BatchAnswer, FULL_KEY, FixedClock, LogCapture, MINUTE, NEAR_MAX, RecordingCache,
    START_OF_MINUTE, WriteFailure, answered, auth_with_hash, detail_text, every_dimension,
    final_write, fixture_with, global_count, id, minute, new_fixture, objects, principal,
    rejection, request_body, split_calls, written_ids,
};
use super::facts::GLOBAL_KEY;
use crate::legacy_admission::error::LegacyAdmissionError;
use crate::rate_limiters::legacy_rate_limit_counters::usage_cache::LegacyCacheValue;

#[tokio::test]
async fn global_cap_absent_leaves_counter_untouched() {
    let fixture = new_fixture();
    let auth = principal("{}");
    let mut request = request_body(None, "{}");
    fixture
        .admission
        .pre_call(&auth, &mut request)
        .await
        .unwrap();
    let calls = fixture.cache.calls();
    assert_eq!(split_calls(&calls).gets, vec![vec![None; 6]]);
    let (entries, ttl, local_only) = final_write(&calls);
    assert!(entries.is_empty());
    assert_eq!((ttl, local_only), (60, false));
}

#[tokio::test]
async fn global_cap_below_increments_local_only() {
    let fixture = answered(global_count(2), objects(&[None; 6]));
    let auth = principal("{}");
    let mut request = request_body(None, r#"{"global_max_parallel_requests": 5}"#);
    fixture
        .admission
        .pre_call(&auth, &mut request)
        .await
        .unwrap();
    let sets = split_calls(&fixture.cache.calls()).sets;
    assert_eq!(
        sets[0].0,
        vec![(String::from(GLOBAL_KEY), LegacyCacheValue::Count(3))]
    );
    assert_eq!((sets[0].1, sets[0].2), (600, true));
}

#[rstest]
#[case::no_answer(BatchAnswer::None)]
#[case::no_value(BatchAnswer::Values(vec![None]))]
#[tokio::test]
async fn global_cap_missing_counter_counts_as_one(#[case] answer: BatchAnswer) {
    let fixture = answered(answer, objects(&[None; 6]));
    let auth = principal("{}");
    let mut request = request_body(None, r#"{"global_max_parallel_requests": 2}"#);
    fixture
        .admission
        .pre_call(&auth, &mut request)
        .await
        .unwrap();
    let sets = split_calls(&fixture.cache.calls()).sets;
    assert_eq!(
        sets[0].0,
        vec![(String::from(GLOBAL_KEY), LegacyCacheValue::Count(2))]
    );
}

#[rstest]
#[case::at_the_limit(2, 2)]
#[case::above_the_limit(2, 3)]
#[tokio::test]
async fn global_cap_at_or_above_raises(#[case] cap: i64, #[case] current: i64) {
    let fixture = answered(global_count(current), objects(&[None; 6]));
    let auth = principal("{}");
    let mut request = request_body(
        Some("gpt-4"),
        &format!(r#"{{"global_max_parallel_requests": {cap}}}"#),
    );
    let error = rejection(fixture.admission.pre_call(&auth, &mut request).await);
    assert_eq!(
        error.rate_limit_type,
        Some(RateLimitDimension::ConcurrentRequests)
    );
    assert_eq!(
        detail_text(&error),
        format!(
            "Max parallel request limit reached Hit Global Limit: Limit={cap}, current: {current}"
        )
    );
    let calls = fixture.cache.calls();
    assert_eq!(split_calls(&calls).gets.len(), 1);
    assert!(split_calls(&calls).sets.is_empty());
}

#[tokio::test]
async fn global_cap_read_error_is_cache_error() {
    let fixture = answered(BatchAnswer::Failed, objects(&[None; 6]));
    let auth = principal("{}");
    let mut request = request_body(None, r#"{"global_max_parallel_requests": 2}"#);
    let error = fixture
        .admission
        .pre_call(&auth, &mut request)
        .await
        .unwrap_err();
    assert!(matches!(error, LegacyAdmissionError::Cache(_)));
}

#[tokio::test]
async fn global_cap_write_error_is_cache_error() {
    let fixture = fixture_with(
        RecordingCache::with_write_failure(WriteFailure::LocalOnly),
        FixedClock::at(START_OF_MINUTE),
    );
    let auth = principal("{}");
    let mut request = request_body(None, r#"{"global_max_parallel_requests": 5}"#);
    let error = fixture
        .admission
        .pre_call(&auth, &mut request)
        .await
        .unwrap_err();
    assert!(matches!(error, LegacyAdmissionError::Cache(_)));
    assert_eq!(split_calls(&fixture.cache.calls()).gets.len(), 1);
}

#[tokio::test]
async fn global_cap_non_integer_is_no_cap() {
    let fixture = new_fixture();
    let auth = principal("{}");
    let mut request = request_body(None, r#"{"global_max_parallel_requests": "2"}"#);
    fixture
        .admission
        .pre_call(&auth, &mut request)
        .await
        .unwrap();
    assert_eq!(
        split_calls(&fixture.cache.calls()).gets,
        vec![vec![None; 6]]
    );
}

#[tokio::test]
async fn global_cap_counter_value_counts_as_one() {
    let fixture = answered(
        BatchAnswer::Values(vec![Some(minute(4, 0, 4))]),
        objects(&[None; 6]),
    );
    let auth = principal("{}");
    let mut request = request_body(None, r#"{"global_max_parallel_requests": 5}"#);
    fixture
        .admission
        .pre_call(&auth, &mut request)
        .await
        .unwrap();
    let sets = split_calls(&fixture.cache.calls()).sets;
    assert_eq!(
        sets[0].0,
        vec![(String::from(GLOBAL_KEY), LegacyCacheValue::Count(2))]
    );
}

#[tokio::test]
async fn cache_key_ids_use_minute_and_every_part() {
    let fixture = answered(global_count(0), objects(&[None; 6]));
    let caller = auth_with_hash(FULL_KEY);
    let mut request = request_body(Some("gpt-4"), r#"{"global_max_parallel_requests": 1}"#);
    fixture
        .admission
        .pre_call(&caller.auth, &mut request)
        .await
        .unwrap();
    let reads = split_calls(&fixture.cache.calls()).gets;
    assert_eq!(
        reads[1],
        vec![
            Some(String::from(GLOBAL_KEY)),
            Some(id(&caller.hash)),
            Some(format!("{}::gpt-4::{MINUTE}::request_count", caller.hash)),
            Some(id("user-1")),
            Some(id("team-1")),
            Some(id("end-user-1")),
        ]
    );
}

#[tokio::test]
async fn cache_key_minute_is_local_and_read_once() {
    let fixture = fixture_with(
        RecordingCache::new(),
        FixedClock::at("2026-12-31T23:59:00+05:30"),
    );
    let auth = principal(r#"{"user_id":"user-1"}"#);
    let mut request = request_body(None, "{}");
    fixture
        .admission
        .pre_call(&auth, &mut request)
        .await
        .unwrap();
    let reads = split_calls(&fixture.cache.calls()).gets;
    assert_eq!(
        reads[0][3],
        Some(String::from("user-1::2026-12-31-23-59::request_count"))
    );
    assert_eq!(fixture.clock.reads(), 1);
}

#[rstest]
#[case::everything(FULL_KEY, Some("gpt-4"), true, [true, true, true, true, true, true])]
#[case::without_key(
    r#"{"user_id":"user-1","team_id":"team-1","end_user_id":"end-user-1"}"#,
    Some("gpt-4"),
    false,
    [false, false, false, true, true, true]
)]
#[case::without_model(FULL_KEY, None, true, [true, true, false, true, true, true])]
#[case::only_key(
    r#"{"api_key":"key-1"}"#,
    Some("gpt-4"),
    false,
    [false, true, true, false, false, false]
)]
#[case::empty_end_user(
    r#"{"end_user_id":""}"#,
    None,
    false,
    [false, false, false, false, false, true]
)]
#[tokio::test]
async fn cache_key_absent_parts_skip_ids(
    #[case] text: &str,
    #[case] model: Option<&str>,
    #[case] cap: bool,
    #[case] expected: [bool; 6],
) {
    let fixture = new_fixture();
    let caller = auth_with_hash(text);
    let metadata = if cap {
        r#"{"global_max_parallel_requests": 9}"#
    } else {
        "{}"
    };
    let mut request = request_body(model, metadata);
    fixture
        .admission
        .pre_call(&caller.auth, &mut request)
        .await
        .unwrap();
    let keys = split_calls(&fixture.cache.calls())
        .gets
        .pop()
        .expect("the six-key read runs");
    assert_eq!(
        keys.iter().map(Option::is_some).collect::<Vec<bool>>(),
        expected
    );
}

#[rstest]
#[case::session_with_alias(
    r#"{"api_key":"key-1","is_session_token":true,"key_alias":"alias-1"}"#,
    Some("alias-1")
)]
#[case::session_without_alias(r#"{"api_key":"key-1","is_session_token":true}"#, None)]
#[case::plain_key_with_alias(r#"{"api_key":"key-1","key_alias":"alias-1"}"#, None)]
#[tokio::test]
async fn cache_key_session_token_logs_under_its_alias(
    #[case] text: &str,
    #[case] alias: Option<&str>,
) {
    let fixture = new_fixture();
    let caller = auth_with_hash(text);
    let mut request = request_body(Some("gpt-4"), "{}");
    fixture
        .admission
        .pre_call(&caller.auth, &mut request)
        .await
        .unwrap();
    let logged = alias.unwrap_or(caller.hash.as_str());
    let keys = split_calls(&fixture.cache.calls())
        .gets
        .pop()
        .expect("the six-key read runs");
    assert_eq!(keys[1], Some(id(logged)));
}

#[tokio::test]
async fn key_limit_defaults_are_unlimited() {
    let current = minute(NEAR_MAX, 0, NEAR_MAX);
    let fixture = answered(
        BatchAnswer::None,
        objects(&[None, Some(current), None, None, None, None]),
    );
    let caller = auth_with_hash(r#"{"api_key":"key-1"}"#);
    let mut request = request_body(None, "{}");
    fixture
        .admission
        .pre_call(&caller.auth, &mut request)
        .await
        .unwrap();
    let sets = split_calls(&fixture.cache.calls()).sets;
    assert_eq!(
        sets[0].0,
        vec![(id(&caller.hash), minute(i64::MAX, 0, i64::MAX))]
    );
    assert_eq!((sets[0].1, sets[0].2), (60, true));
}

#[rstest]
#[case::max_parallel(
    r#"{"api_key":"key-1","max_parallel_requests":0}"#,
    RateLimitDimension::ConcurrentRequests
)]
#[case::tpm(r#"{"api_key":"key-1","tpm_limit":0}"#, RateLimitDimension::Tokens)]
#[case::rpm(r#"{"api_key":"key-1","rpm_limit":0}"#, RateLimitDimension::Requests)]
#[tokio::test]
async fn key_limit_zero_is_enforced(#[case] text: &str, #[case] dimension: RateLimitDimension) {
    let fixture = new_fixture();
    let caller = auth_with_hash(text);
    let mut request = request_body(None, "{}");
    let error = rejection(fixture.admission.pre_call(&caller.auth, &mut request).await);
    assert_eq!(error.rate_limit_type, Some(dimension));
    assert!(detail_text(&error).contains("Hit limit for key"));
}

#[tokio::test]
async fn key_limit_throttle_halves_the_key_limits() {
    let current = minute(0, 50, 0);
    let fixture = answered(
        BatchAnswer::None,
        objects(&[None, Some(current), None, None, None, None]),
    );
    let caller = auth_with_hash(
        r#"{"api_key":"key-1","tpm_limit":100,"rpm_limit":100,"budget_throttle_pct":0.5}"#,
    );
    let mut request = request_body(None, "{}");
    let error = rejection(fixture.admission.pre_call(&caller.auth, &mut request).await);
    assert_eq!(error.rate_limit_type, Some(RateLimitDimension::Tokens));
}

#[tokio::test]
async fn key_limit_without_percentage_keeps_the_limit() {
    let current = minute(0, 50, 0);
    let fixture = answered(
        BatchAnswer::None,
        objects(&[None, Some(current), None, None, None, None]),
    );
    let caller = auth_with_hash(r#"{"api_key":"key-1","tpm_limit":100,"rpm_limit":100}"#);
    let mut request = request_body(None, "{}");
    fixture
        .admission
        .pre_call(&caller.auth, &mut request)
        .await
        .unwrap();
    let sets = split_calls(&fixture.cache.calls()).sets;
    assert_eq!(sets[0].0, vec![(id(&caller.hash), minute(1, 50, 1))]);
}

#[tokio::test]
async fn key_limit_skips_without_api_key() {
    let fixture = new_fixture();
    let auth = principal(r#"{"user_id":"user-1"}"#);
    let mut request = request_body(None, "{}");
    fixture
        .admission
        .pre_call(&auth, &mut request)
        .await
        .unwrap();
    let sets = split_calls(&fixture.cache.calls()).sets;
    assert_eq!(sets.len(), 2);
    assert_eq!(sets[0].0, vec![(id("user-1"), minute(1, 0, 1))]);
}

#[tokio::test]
async fn key_limit_bare_count_is_a_missing_counter() {
    let fixture = answered(
        BatchAnswer::None,
        objects(&[
            None,
            Some(LegacyCacheValue::Count(9)),
            None,
            None,
            None,
            None,
        ]),
    );
    let caller = auth_with_hash(r#"{"api_key":"key-1","tpm_limit":10}"#);
    let mut request = request_body(None, "{}");
    fixture
        .admission
        .pre_call(&caller.auth, &mut request)
        .await
        .unwrap();
    let sets = split_calls(&fixture.cache.calls()).sets;
    assert_eq!(sets[0].0, vec![(id(&caller.hash), minute(1, 0, 1))]);
}

#[tokio::test]
async fn model_limit_absent_maps_skip_the_check() {
    let current = minute(0, 0, 0);
    let fixture = answered(
        BatchAnswer::None,
        objects(&[None, None, Some(current), None, None, None]),
    );
    let caller = auth_with_hash(r#"{"api_key":"key-1"}"#);
    let mut request = request_body(Some("gpt-4"), "{}");
    fixture
        .admission
        .pre_call(&caller.auth, &mut request)
        .await
        .unwrap();
    assert_eq!(split_calls(&fixture.cache.calls()).sets.len(), 2);
    assert!(request.metadata.is_empty());
}

#[rstest]
#[case::tpm_only(
    r#"{"api_key":"key-1","metadata":{"model_tpm_limit":{"gpt-4":10}}}"#,
    minute(0, 10, 0),
    RateLimitDimension::Tokens
)]
#[case::rpm_only(
    r#"{"api_key":"key-1","metadata":{"model_rpm_limit":{"gpt-4":10}}}"#,
    minute(0, 0, 10),
    RateLimitDimension::Requests
)]
#[tokio::test]
async fn model_limit_either_map_runs_the_check(
    #[case] text: &str,
    #[case] current: LegacyCacheValue,
    #[case] dimension: RateLimitDimension,
) {
    let fixture = answered(
        BatchAnswer::None,
        objects(&[None, None, Some(current), None, None, None]),
    );
    let caller = auth_with_hash(text);
    let mut request = request_body(Some("gpt-4"), "{}");
    let error = rejection(fixture.admission.pre_call(&caller.auth, &mut request).await);
    assert_eq!(error.rate_limit_type, Some(dimension));
    assert!(detail_text(&error).contains("rate limit type = model_per_key"));
}

#[tokio::test]
async fn model_limit_zero_is_unlimited() {
    let current = minute(0, 500, 500);
    let fixture = answered(
        BatchAnswer::None,
        objects(&[None, None, Some(current), None, None, None]),
    );
    let caller = auth_with_hash(
        r#"{"api_key":"key-1","metadata":{"model_tpm_limit":{"gpt-4":0},"model_rpm_limit":{"gpt-4":0}}}"#,
    );
    let mut request = request_body(Some("gpt-4"), "{}");
    fixture
        .admission
        .pre_call(&caller.auth, &mut request)
        .await
        .unwrap();
    let sets = split_calls(&fixture.cache.calls()).sets;
    assert_eq!(
        sets[1].0,
        vec![
            (id(&caller.hash), minute(1, 0, 1)),
            (
                format!("{}::gpt-4::{MINUTE}::request_count", caller.hash),
                minute(1, 500, 501)
            ),
        ]
    );
}

#[tokio::test]
async fn model_limit_missing_model_is_unlimited() {
    let current = minute(0, 500, 0);
    let fixture = answered(
        BatchAnswer::None,
        objects(&[None, None, Some(current), None, None, None]),
    );
    let caller =
        auth_with_hash(r#"{"api_key":"key-1","metadata":{"model_tpm_limit":{"other":1}}}"#);
    let mut request = request_body(Some("gpt-4"), "{}");
    fixture
        .admission
        .pre_call(&caller.auth, &mut request)
        .await
        .unwrap();
    assert_eq!(
        request.metadata.get("litellm-key-remaining-tokens-gpt-4"),
        Some(&JsonValue::Null)
    );
    assert_eq!(
        request.metadata.get("litellm-key-remaining-requests-gpt-4"),
        Some(&JsonValue::Null)
    );
    let sets = split_calls(&fixture.cache.calls()).sets;
    assert_eq!(sets.len(), 3);
    assert_eq!(sets[1].0[1].1, minute(1, 500, 1));
}

#[tokio::test]
async fn model_limit_empty_team_map_counts_as_present() {
    let fixture = new_fixture();
    let caller = auth_with_hash(
        r#"{"api_key":"key-1","team_metadata":{"model_tpm_limit":{},"model_rpm_limit":{}}}"#,
    );
    let mut request = request_body(Some("gpt-4"), "{}");
    fixture
        .admission
        .pre_call(&caller.auth, &mut request)
        .await
        .unwrap();
    let sets = split_calls(&fixture.cache.calls()).sets;
    assert_eq!(sets.len(), 3);
    assert_eq!(
        sets[1].0[1].0,
        format!("{}::gpt-4::{MINUTE}::request_count", caller.hash)
    );
    assert_eq!(
        request.metadata.get("litellm-key-remaining-tokens-gpt-4"),
        Some(&JsonValue::Null)
    );
}

#[rstest]
#[case::no_api_key(
    r#"{"team_metadata":{"model_tpm_limit":{"gpt-4":10}}}"#,
    Some("gpt-4"),
    false
)]
#[case::no_model(
    r#"{"api_key":"key-1","metadata":{"model_tpm_limit":{"gpt-4":10}}}"#,
    None,
    true
)]
#[tokio::test]
async fn model_limit_id_uses_none_text(
    #[case] text: &str,
    #[case] model: Option<&str>,
    #[case] has_key: bool,
) {
    let fixture = new_fixture();
    let caller = auth_with_hash(text);
    let mut request = request_body(model, "{}");
    fixture
        .admission
        .pre_call(&caller.auth, &mut request)
        .await
        .unwrap();
    let prefix = if has_key {
        caller.hash.as_str()
    } else {
        "None"
    };
    let expected = format!(
        "{prefix}::{}::{MINUTE}::request_count",
        model.unwrap_or("None")
    );
    let calls = fixture.cache.calls();
    assert!(written_ids(&calls).contains(&expected));
    let keys = split_calls(&calls)
        .gets
        .pop()
        .expect("the six-key read runs");
    assert_eq!(keys[2], None);
}

#[tokio::test]
async fn remaining_metadata_counts_from_the_returned_counters() {
    let current = minute(0, 100, 20);
    let fixture = answered(
        BatchAnswer::None,
        objects(&[None, None, Some(current), None, None, None]),
    );
    let caller = auth_with_hash(
        r#"{"api_key":"key-1","metadata":{"model_tpm_limit":{"gpt-4":1000},"model_rpm_limit":{"gpt-4":500}}}"#,
    );
    let mut request = request_body(Some("gpt-4"), "{}");
    fixture
        .admission
        .pre_call(&caller.auth, &mut request)
        .await
        .unwrap();
    assert_eq!(
        request
            .metadata
            .get("litellm-key-remaining-tokens-gpt-4")
            .and_then(JsonValue::as_i64),
        Some(900)
    );
    assert_eq!(
        request
            .metadata
            .get("litellm-key-remaining-requests-gpt-4")
            .and_then(JsonValue::as_i64),
        Some(479)
    );
}

#[tokio::test]
async fn remaining_metadata_zero_limit_is_negative() {
    let current = minute(0, 100, 20);
    let fixture = answered(
        BatchAnswer::None,
        objects(&[None, None, Some(current), None, None, None]),
    );
    let caller = auth_with_hash(
        r#"{"api_key":"key-1","metadata":{"model_tpm_limit":{"gpt-4":0},"model_rpm_limit":{"gpt-4":0}}}"#,
    );
    let mut request = request_body(Some("gpt-4"), "{}");
    fixture
        .admission
        .pre_call(&caller.auth, &mut request)
        .await
        .unwrap();
    assert_eq!(
        request
            .metadata
            .get("litellm-key-remaining-tokens-gpt-4")
            .and_then(JsonValue::as_i64),
        Some(-100)
    );
    assert_eq!(
        request
            .metadata
            .get("litellm-key-remaining-requests-gpt-4")
            .and_then(JsonValue::as_i64),
        Some(-21)
    );
}

#[tokio::test]
async fn remaining_metadata_null_when_the_limit_is_absent() {
    let fixture = new_fixture();
    let caller =
        auth_with_hash(r#"{"api_key":"key-1","metadata":{"model_tpm_limit":{"gpt-4":1000}}}"#);
    let mut request = request_body(Some("gpt-4"), "{}");
    fixture
        .admission
        .pre_call(&caller.auth, &mut request)
        .await
        .unwrap();
    assert_eq!(
        request.metadata.get("litellm-key-remaining-tokens-gpt-4"),
        Some(&JsonValue::from(1000_i64))
    );
    assert_eq!(
        request.metadata.get("litellm-key-remaining-requests-gpt-4"),
        Some(&JsonValue::Null)
    );
}

#[tokio::test]
async fn remaining_metadata_uses_none_text_without_model() {
    let fixture = new_fixture();
    let caller =
        auth_with_hash(r#"{"api_key":"key-1","metadata":{"model_tpm_limit":{"gpt-4":10}}}"#);
    let mut request = request_body(None, "{}");
    fixture
        .admission
        .pre_call(&caller.auth, &mut request)
        .await
        .unwrap();
    assert_eq!(
        request.metadata.get("litellm-key-remaining-tokens-None"),
        Some(&JsonValue::Null)
    );
    assert_eq!(
        request.metadata.get("litellm-key-remaining-requests-None"),
        Some(&JsonValue::Null)
    );
}

#[tokio::test]
async fn user_limit_empty_user_id_is_checked() {
    let fixture = new_fixture();
    let auth = principal(r#"{"user_id":"","user_rpm_limit":0}"#);
    let mut request = request_body(None, "{}");
    let error = rejection(fixture.admission.pre_call(&auth, &mut request).await);
    assert_eq!(error.rate_limit_type, Some(RateLimitDimension::Requests));
    assert!(detail_text(&error).contains("Hit limit for user"));
}

#[tokio::test]
async fn user_limit_defaults_are_unlimited() {
    let current = minute(NEAR_MAX, 0, NEAR_MAX);
    let fixture = answered(
        BatchAnswer::None,
        objects(&[None, None, None, Some(current), None, None]),
    );
    let auth = principal(r#"{"user_id":"user-1"}"#);
    let mut request = request_body(None, "{}");
    fixture
        .admission
        .pre_call(&auth, &mut request)
        .await
        .unwrap();
    let sets = split_calls(&fixture.cache.calls()).sets;
    assert_eq!(
        sets[0].0,
        vec![(id("user-1"), minute(i64::MAX, 0, i64::MAX))]
    );
}

#[tokio::test]
async fn user_limit_is_not_throttled() {
    let current = minute(0, 60, 0);
    let fixture = answered(
        BatchAnswer::None,
        objects(&[None, None, None, Some(current), None, None]),
    );
    let auth = principal(r#"{"user_id":"user-1","user_tpm_limit":100,"budget_throttle_pct":0.5}"#);
    let mut request = request_body(None, "{}");
    fixture
        .admission
        .pre_call(&auth, &mut request)
        .await
        .unwrap();
    let sets = split_calls(&fixture.cache.calls()).sets;
    assert_eq!(sets[0].0, vec![(id("user-1"), minute(1, 60, 1))]);
}

#[tokio::test]
async fn team_limit_zero_is_enforced() {
    let fixture = new_fixture();
    let auth = principal(r#"{"team_id":"team-1","team_tpm_limit":0}"#);
    let mut request = request_body(None, "{}");
    let error = rejection(fixture.admission.pre_call(&auth, &mut request).await);
    assert_eq!(error.rate_limit_type, Some(RateLimitDimension::Tokens));
    assert!(detail_text(&error).contains("Hit limit for team"));
}

#[tokio::test]
async fn team_limit_defaults_are_unlimited() {
    let current = minute(NEAR_MAX, 0, NEAR_MAX);
    let fixture = answered(
        BatchAnswer::None,
        objects(&[None, None, None, None, Some(current), None]),
    );
    let auth = principal(r#"{"team_id":"team-1"}"#);
    let mut request = request_body(None, "{}");
    fixture
        .admission
        .pre_call(&auth, &mut request)
        .await
        .unwrap();
    let sets = split_calls(&fixture.cache.calls()).sets;
    assert_eq!(
        sets[0].0,
        vec![(id("team-1"), minute(i64::MAX, 0, i64::MAX))]
    );
}

#[rstest]
#[case::rpm(
    r#"{"end_user_id":"end-user-1","end_user_rpm_limit":0}"#,
    RateLimitDimension::Requests
)]
#[case::tpm(
    r#"{"end_user_id":"end-user-1","end_user_tpm_limit":0}"#,
    RateLimitDimension::Tokens
)]
#[tokio::test]
async fn end_user_limit_truthy_runs_as_customer(
    #[case] text: &str,
    #[case] dimension: RateLimitDimension,
) {
    let fixture = new_fixture();
    let auth = principal(text);
    let mut request = request_body(None, "{}");
    let error = rejection(fixture.admission.pre_call(&auth, &mut request).await);
    assert_eq!(error.rate_limit_type, Some(dimension));
    assert!(detail_text(&error).contains("Hit limit for customer"));
}

#[tokio::test]
async fn end_user_limit_empty_id_skips_the_check() {
    let fixture = new_fixture();
    let auth = principal(r#"{"end_user_id":"","end_user_rpm_limit":0}"#);
    let mut request = request_body(None, "{}");
    fixture
        .admission
        .pre_call(&auth, &mut request)
        .await
        .unwrap();
    let keys = split_calls(&fixture.cache.calls())
        .gets
        .pop()
        .expect("the six-key read runs");
    assert_eq!(keys[5], Some(id("")));
    let (entries, ttl, local_only) = final_write(&fixture.cache.calls());
    assert!(entries.is_empty());
    assert_eq!((ttl, local_only), (60, false));
}

#[tokio::test]
async fn end_user_limit_defaults_are_unlimited() {
    let current = minute(NEAR_MAX, 0, NEAR_MAX);
    let fixture = answered(
        BatchAnswer::None,
        objects(&[None, None, None, None, None, Some(current)]),
    );
    let auth = principal(r#"{"end_user_id":"end-user-1"}"#);
    let mut request = request_body(None, "{}");
    fixture
        .admission
        .pre_call(&auth, &mut request)
        .await
        .unwrap();
    let sets = split_calls(&fixture.cache.calls()).sets;
    assert_eq!(
        sets[0].0,
        vec![(id("end-user-1"), minute(i64::MAX, 0, i64::MAX))]
    );
}

#[tokio::test]
async fn order_runs_each_dimension_then_writes_the_batch() {
    let current = minute(1, 0, 1);
    let fixture = answered(global_count(1), every_dimension(current));
    let caller = auth_with_hash(
        r#"{"api_key":"key-1","user_id":"user-1","team_id":"team-1","end_user_id":"end-user-1","metadata":{"model_tpm_limit":{"gpt-4":1000}}}"#,
    );
    let mut request = request_body(Some("gpt-4"), r#"{"global_max_parallel_requests": 5}"#);
    fixture
        .admission
        .pre_call(&caller.auth, &mut request)
        .await
        .unwrap();
    assert_eq!(
        fixture.console.printed(),
        vec![String::from("Inside Max Parallel Request Pre-Call Hook")]
    );
    let calls = fixture.cache.calls();
    assert_eq!(split_calls(&calls).gets.len(), 2);
    let sets = split_calls(&calls).sets;
    assert_eq!(sets.len(), 7);
    assert_eq!(
        sets[0].0,
        vec![(String::from(GLOBAL_KEY), LegacyCacheValue::Count(2))]
    );
    assert_eq!(sets[1].0, vec![(id(&caller.hash), minute(2, 0, 2))]);
    let lengths: Vec<usize> = sets.iter().map(|(entries, _, _)| entries.len()).collect();
    assert_eq!(lengths, vec![1, 1, 2, 3, 4, 5, 5]);
}

#[tokio::test]
async fn order_accumulates_every_pair_into_the_final_write() {
    let current = minute(1, 0, 1);
    let fixture = answered(global_count(1), every_dimension(current));
    let caller = auth_with_hash(
        r#"{"api_key":"key-1","user_id":"user-1","team_id":"team-1","end_user_id":"end-user-1","metadata":{"model_tpm_limit":{"gpt-4":1000}}}"#,
    );
    let mut request = request_body(Some("gpt-4"), r#"{"global_max_parallel_requests": 5}"#);
    fixture
        .admission
        .pre_call(&caller.auth, &mut request)
        .await
        .unwrap();
    let sets = split_calls(&fixture.cache.calls()).sets;
    assert_eq!(sets[5].0, sets[6].0, "the final write repeats every pair");
    let (entries, ttl, local_only) = final_write(&fixture.cache.calls());
    assert_eq!(
        entries
            .iter()
            .map(|(counter_id, _)| counter_id.clone())
            .collect::<Vec<String>>(),
        vec![
            id(&caller.hash),
            format!("{}::gpt-4::{MINUTE}::request_count", caller.hash),
            id("user-1"),
            id("team-1"),
            id("end-user-1"),
        ]
    );
    assert_eq!((ttl, local_only), (60, false));
    assert!(
        sets[..6]
            .iter()
            .all(|(_, written_ttl, written_local_only)| {
                *written_ttl == 60 && *written_local_only
            }),
        "every check writes the accumulated list locally under ttl 60"
    );
}

#[tokio::test]
async fn order_later_raise_keeps_earlier_counters() {
    let current = minute(0, 10, 10);
    let fixture = answered(
        global_count(0),
        objects(&[None, None, Some(current), None, None, None]),
    );
    let caller = auth_with_hash(
        r#"{"api_key":"key-1","user_id":"user-1","metadata":{"model_rpm_limit":{"gpt-4":10}}}"#,
    );
    let mut request = request_body(Some("gpt-4"), r#"{"global_max_parallel_requests": 1}"#);
    let error = rejection(fixture.admission.pre_call(&caller.auth, &mut request).await);
    assert_eq!(error.rate_limit_type, Some(RateLimitDimension::Requests));
    let sets = split_calls(&fixture.cache.calls()).sets;
    assert_eq!(sets.len(), 2);
    assert_eq!(
        sets[0].0,
        vec![(String::from(GLOBAL_KEY), LegacyCacheValue::Count(1))]
    );
    assert_eq!(sets[1].0, vec![(id(&caller.hash), minute(1, 0, 1))]);
}

#[tokio::test]
async fn final_write_carries_every_pair_with_ttl_60() {
    let current = minute(0, 0, 0);
    let fixture = answered(
        BatchAnswer::None,
        objects(&[None, Some(current), None, None, None, None]),
    );
    let caller = auth_with_hash(FULL_KEY);
    let mut request = request_body(Some("gpt-4"), "{}");
    fixture
        .admission
        .pre_call(&caller.auth, &mut request)
        .await
        .unwrap();
    let calls = fixture.cache.calls();
    let (entries, ttl, local_only) = final_write(&calls);
    assert_eq!((ttl, local_only), (60, false));
    assert_eq!(
        entries
            .iter()
            .map(|(counter_id, _)| counter_id.clone())
            .collect::<Vec<String>>(),
        vec![
            id(&caller.hash),
            id("user-1"),
            id("team-1"),
            id("end-user-1"),
        ]
    );
    let sets = split_calls(&calls).sets;
    assert_eq!(sets.len(), 5);
}

#[tokio::test]
async fn final_write_empty_list_still_sent() {
    let fixture = new_fixture();
    let auth = principal("{}");
    let mut request = request_body(None, "{}");
    fixture
        .admission
        .pre_call(&auth, &mut request)
        .await
        .unwrap();
    let calls = fixture.cache.calls();
    assert_eq!(calls.len(), 2);
    let (entries, ttl, local_only) = final_write(&calls);
    assert!(entries.is_empty());
    assert_eq!((ttl, local_only), (60, false));
}

#[tokio::test]
async fn final_write_failure_is_logged_and_admitted() {
    let logs = LogCapture::start();
    let fixture = fixture_with(
        RecordingCache::with_write_failure(WriteFailure::RemoteOnly),
        FixedClock::at(START_OF_MINUTE),
    );
    let auth = principal("{}");
    let mut request = request_body(None, "{}");
    fixture
        .admission
        .pre_call(&auth, &mut request)
        .await
        .unwrap();
    let (_, ttl, local_only) = final_write(&fixture.cache.calls());
    assert_eq!((ttl, local_only), (60, false));
    assert!(
        logs.text()
            .contains("LiteLLM Cache: exception in async add_cache")
    );
}
