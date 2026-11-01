#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{
    LoggedUsage, PromptTokensDetails, RawUsageObject, baseline_cache_rate_keys, baseline_usage,
    cache_token_split, usage_from_spend_log,
};
use crate::tiered_pricing::Scalar;

fn raw_usage(text: &str) -> RawUsageObject {
    serde_json::from_str(text).expect("the fixture usage_object deserializes")
}

#[rstest]
#[case::missing(None)]
#[case::empty_object(Some("{}"))]
fn usage_from_spend_log_is_none_when_unavailable(#[case] text: Option<&str>) {
    let usage_object = text.map(raw_usage);
    assert!(usage_from_spend_log(usage_object.as_ref()).is_none());
}

#[test]
fn usage_from_spend_log_rebuilds_cache_fields() {
    let usage_object = raw_usage(
        r#"{
            "prompt_tokens": 100,
            "cache_read_input_tokens": 5,
            "cache_creation_input_tokens": 3,
            "prompt_tokens_details": {"cached_tokens": 5, "cache_creation_tokens": 3}
        }"#,
    );
    let usage =
        usage_from_spend_log(Some(&usage_object)).expect("a well-shaped usage_object rebuilds");
    assert_eq!(usage.prompt_tokens, 100);
    assert_eq!(usage.cache_read_input_tokens, 5);
    assert_eq!(
        usage
            .prompt_tokens_details
            .expect("details carried over")
            .cached_tokens,
        5
    );
}

#[test]
fn usage_from_spend_log_warns_and_returns_none_on_a_malformed_record() {
    let usage_object = raw_usage(r#"{"prompt_tokens": "not-a-number"}"#);
    assert!(usage_from_spend_log(Some(&usage_object)).is_none());
}

fn usage_with_details(details: PromptTokensDetails) -> LoggedUsage {
    LoggedUsage {
        prompt_tokens: 0,
        cache_read_input_tokens: details.cached_tokens,
        cache_creation_input_tokens: if details.cache_creation_tokens != 0 {
            details.cache_creation_tokens
        } else {
            details.cache_write_tokens
        },
        prompt_tokens_details: Some(details),
    }
}

#[test]
fn cache_token_split_is_zero_without_details() {
    assert_eq!(cache_token_split(&LoggedUsage::default()), (0, 0));
}

#[test]
fn cache_token_split_prefers_cache_creation_tokens_over_cache_write_tokens() {
    let usage = usage_with_details(PromptTokensDetails {
        cached_tokens: 7,
        cache_creation_tokens: 4,
        cache_write_tokens: 9,
        text_tokens: 0,
        audio_tokens: 0,
        image_tokens: 0,
        video_tokens: 0,
        cache_creation_token_details: None,
    });
    assert_eq!(cache_token_split(&usage), (7, 4));
}

#[test]
fn cache_token_split_falls_back_to_cache_write_tokens_when_creation_is_zero() {
    let usage = usage_with_details(PromptTokensDetails {
        cached_tokens: 7,
        cache_creation_tokens: 0,
        cache_write_tokens: 9,
        text_tokens: 0,
        audio_tokens: 0,
        image_tokens: 0,
        video_tokens: 0,
        cache_creation_token_details: None,
    });
    assert_eq!(cache_token_split(&usage), (7, 9));
}

#[test]
fn cache_token_split_passes_a_negative_read_count_through() {
    let usage = usage_with_details(PromptTokensDetails {
        cached_tokens: -3,
        cache_creation_tokens: 0,
        cache_write_tokens: 0,
        text_tokens: 0,
        audio_tokens: 0,
        image_tokens: 0,
        video_tokens: 0,
        cache_creation_token_details: None,
    });
    assert_eq!(cache_token_split(&usage), (-3, 0));
}

fn model_info(entries: &[(&str, Scalar)]) -> BTreeMap<String, Scalar> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

#[test]
fn baseline_cache_rate_keys_prices_both_buckets_without_a_baseline_model() {
    assert_eq!(baseline_cache_rate_keys(None), (true, true));
}

#[test]
fn baseline_cache_rate_keys_is_false_for_absent_rate_fields() {
    let info = model_info(&[]);
    assert_eq!(baseline_cache_rate_keys(Some(&info)), (false, false));
}

#[rstest]
#[case::zero_rate_is_falsy(Scalar::Number(serde_json::Number::from(0_i32)), false)]
#[case::nonzero_rate_is_truthy(Scalar::Number(serde_json::Number::from_f64(0.01).expect("a finite rate")), true)]
#[case::false_bool_is_falsy(Scalar::Bool(false), false)]
#[case::true_bool_is_truthy(Scalar::Bool(true), true)]
#[case::empty_string_is_falsy(Scalar::Text(String::new()), false)]
fn baseline_cache_rate_keys_reads_truthiness_from_the_raw_scalar(
    #[case] rate: Scalar,
    #[case] expected: bool,
) {
    let info = model_info(&[("cache_read_input_token_cost", rate)]);
    assert_eq!(baseline_cache_rate_keys(Some(&info)), (expected, false));
}

#[test]
fn baseline_usage_leaves_usage_unchanged_without_positive_cache_tokens() {
    let usage = usage_with_details(PromptTokensDetails::default());
    let normalized = baseline_usage(usage.clone(), None);
    assert_eq!(normalized.prompt_tokens, usage.prompt_tokens);
    assert_eq!(
        normalized.cache_read_input_tokens,
        usage.cache_read_input_tokens
    );
    assert_eq!(
        normalized.cache_creation_input_tokens,
        usage.cache_creation_input_tokens
    );
}

#[test]
fn baseline_usage_leaves_usage_unchanged_when_both_buckets_stay_priced() {
    let usage = usage_with_details(PromptTokensDetails {
        cached_tokens: 5,
        cache_creation_tokens: 0,
        cache_write_tokens: 3,
        text_tokens: 0,
        audio_tokens: 0,
        image_tokens: 0,
        video_tokens: 0,
        cache_creation_token_details: None,
    });
    let normalized = baseline_usage(usage.clone(), None);
    assert_eq!(
        normalized.cache_read_input_tokens,
        usage.cache_read_input_tokens
    );
    assert_eq!(
        normalized.cache_creation_input_tokens,
        usage.cache_creation_input_tokens
    );
}

fn ttl_detail() -> Scalar {
    serde_json::from_str(r#"{"ttl": "5m"}"#).expect("the fixture ttl detail deserializes")
}

#[test]
fn baseline_usage_converts_an_unpriced_read_bucket_to_ordinary_input() {
    let usage = LoggedUsage {
        prompt_tokens: 20,
        cache_read_input_tokens: 5,
        cache_creation_input_tokens: 3,
        prompt_tokens_details: Some(PromptTokensDetails {
            cached_tokens: 5,
            cache_creation_tokens: 0,
            cache_write_tokens: 3,
            text_tokens: 0,
            audio_tokens: 0,
            image_tokens: 0,
            video_tokens: 0,
            cache_creation_token_details: Some(ttl_detail()),
        }),
    };
    let baseline_info = model_info(&[(
        "cache_creation_input_token_cost",
        Scalar::Number(serde_json::Number::from(1_i32)),
    )]);
    let normalized = baseline_usage(usage, Some(&baseline_info));
    assert_eq!(normalized.cache_read_input_tokens, 0);
    assert_eq!(normalized.cache_creation_input_tokens, 3);
    let details = normalized
        .prompt_tokens_details
        .expect("details remain present");
    assert_eq!(details.cached_tokens, 0);
    assert_eq!(details.cache_write_tokens, 3);
    assert_eq!(details.text_tokens, 17);
    assert!(details.cache_creation_token_details.is_some());
}

#[test]
fn baseline_usage_clears_ttl_detail_when_no_writes_remain() {
    let usage = LoggedUsage {
        prompt_tokens: 10,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 4,
        prompt_tokens_details: Some(PromptTokensDetails {
            cached_tokens: 0,
            cache_creation_tokens: 0,
            cache_write_tokens: 4,
            text_tokens: 0,
            audio_tokens: 0,
            image_tokens: 0,
            video_tokens: 0,
            cache_creation_token_details: Some(ttl_detail()),
        }),
    };
    let baseline_info = model_info(&[]);
    let normalized = baseline_usage(usage, Some(&baseline_info));
    assert_eq!(normalized.cache_creation_input_tokens, 0);
    let details = normalized
        .prompt_tokens_details
        .expect("details remain present");
    assert_eq!(details.cache_write_tokens, 0);
    assert!(details.cache_creation_token_details.is_none());
}
