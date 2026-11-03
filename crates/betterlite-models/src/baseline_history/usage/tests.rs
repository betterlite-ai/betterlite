#![cfg(test)]

use rstest::rstest;

use super::{
    CacheCreationTokenDetails, PromptTokensDetails, RawPromptTokensDetails, Usage,
    is_usage_complete, usage_with_cache,
};
use crate::baseline_history::error::BaselineHistoryError;
use crate::decisions::{Extra, Json};

fn complete_usage(prompt: i64, completion: i64, text: i64, cached: i64, creation: i64) -> Usage {
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: prompt.checked_add(completion).expect("total fits"),
        prompt_tokens_details: Some(PromptTokensDetails {
            text_tokens: Some(text),
            cached_tokens: Some(cached),
            cache_creation_tokens: Some(creation),
            cache_creation_token_details: if creation == 0 {
                None
            } else {
                Some(CacheCreationTokenDetails {
                    ephemeral_5m_input_tokens: Some(creation),
                    ephemeral_1h_input_tokens: Some(0),
                })
            },
            cache_write_tokens: None,
            extra: Extra::default(),
        }),
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        extra: Extra::default(),
    }
}

#[rstest]
#[case::none(None, false)]
#[case::negative_prompt(Some(complete_usage(-1, 0, -1, 0, 0)), false)]
#[case::missing_details(Some(Usage { prompt_tokens: 1, completion_tokens: 0, total_tokens: 1, prompt_tokens_details: None, cache_read_input_tokens: None, cache_creation_input_tokens: None, extra: Extra::default() }), false)]
#[case::mismatched_total(Some(Usage { prompt_tokens: 5, completion_tokens: 1, total_tokens: 9, prompt_tokens_details: Some(PromptTokensDetails { text_tokens: Some(5), cached_tokens: Some(0), cache_creation_tokens: Some(0), cache_creation_token_details: None, cache_write_tokens: None, extra: Extra::default() }), cache_read_input_tokens: None, cache_creation_input_tokens: None, extra: Extra::default() }), false)]
#[case::detail_sum_mismatch(Some(Usage { prompt_tokens: 5, completion_tokens: 1, total_tokens: 6, prompt_tokens_details: Some(PromptTokensDetails { text_tokens: Some(1), cached_tokens: Some(1), cache_creation_tokens: Some(0), cache_creation_token_details: None, cache_write_tokens: None, extra: Extra::default() }), cache_read_input_tokens: None, cache_creation_input_tokens: None, extra: Extra::default() }), false)]
#[case::writes_without_split(Some(complete_usage(10, 1, 5, 0, 5)).map(|mut usage| { usage.prompt_tokens_details.as_mut().expect("details").cache_creation_token_details = None; usage }), false)]
#[case::complete_without_writes(Some(complete_usage(10, 2, 10, 0, 0)), true)]
#[case::complete_with_writes(Some(complete_usage(10, 2, 2, 3, 5)), true)]
fn is_usage_complete_matches_the_litellm_predicate(
    #[case] usage: Option<Usage>,
    #[case] expected: bool,
) {
    assert_eq!(is_usage_complete(usage.as_ref()), expected);
}

#[test]
fn usage_with_cache_rebuilds_the_prompt_split_and_keeps_completion_tokens() {
    let usage = complete_usage(100, 7, 100, 0, 0);
    let rebuilt = usage_with_cache(&usage, 100, 20, 30, 10).expect("checked rebuild");
    assert_eq!(rebuilt.prompt_tokens, 100);
    assert_eq!(rebuilt.completion_tokens, 7);
    assert_eq!(rebuilt.total_tokens, 107);
    let details = rebuilt.prompt_tokens_details.expect("details");
    assert_eq!(details.text_tokens, Some(40));
    assert_eq!(details.cached_tokens, Some(20));
    assert_eq!(details.cache_creation_tokens, Some(40));
    let split = details.cache_creation_token_details.expect("split");
    assert_eq!(split.ephemeral_5m_input_tokens, Some(30));
    assert_eq!(split.ephemeral_1h_input_tokens, Some(10));
    assert_eq!(details.cache_write_tokens, Some(40));
    assert_eq!(rebuilt.cache_read_input_tokens, Some(20));
    assert_eq!(rebuilt.cache_creation_input_tokens, Some(40));
}

#[test]
fn usage_with_cache_retains_unrelated_extra_fields() {
    let mut usage = complete_usage(10, 1, 10, 0, 0);
    usage.extra.insert(
        "cost".to_owned(),
        Json::Number(serde_json::Number::from(1_i32)),
    );
    let rebuilt = usage_with_cache(&usage, 10, 0, 0, 0).expect("checked rebuild");
    assert_eq!(rebuilt.extra.get("cost"), usage.extra.get("cost"));
}

#[rstest]
#[case::creation_only(Some(5), None, Some(5))]
#[case::write_only(None, Some(5), Some(5))]
#[case::neither(None, None, None)]
fn prompt_tokens_details_mirrors_cache_creation_and_write_tokens_on_parse(
    #[case] cache_creation_tokens: Option<i64>,
    #[case] cache_write_tokens: Option<i64>,
    #[case] expected: Option<i64>,
) {
    let raw = RawPromptTokensDetails {
        text_tokens: None,
        cached_tokens: None,
        cache_creation_tokens,
        cache_creation_token_details: None,
        cache_write_tokens,
        extra: Extra::default(),
    };
    let details = PromptTokensDetails::from(raw);
    assert_eq!(details.cache_creation_tokens, expected);
    assert_eq!(details.cache_write_tokens, expected);
}

#[rstest]
#[case::write_wins(Some(20), Some(10), Some(5), Some(20))]
#[case::creation_beats_nested(None, Some(20), Some(5), Some(20))]
#[case::nested_alone_counts(None, None, Some(20), Some(20))]
#[case::nested_float_is_ignored(None, None, None, None)]
fn prompt_tokens_details_takes_the_nested_creation_count_last(
    #[case] cache_write_tokens: Option<i64>,
    #[case] cache_creation_tokens: Option<i64>,
    #[case] nested_value: Option<i64>,
    #[case] expected: Option<i64>,
) {
    let mut extra = Extra::default();
    match nested_value {
        Some(value) => {
            extra.insert(
                "cache_creation_input_tokens".to_owned(),
                Json::Number(serde_json::Number::from(value)),
            );
        }
        None => {
            extra.insert(
                "cache_creation_input_tokens".to_owned(),
                Json::Number("20.0".parse().expect("JSON float")),
            );
        }
    }
    let raw = RawPromptTokensDetails {
        text_tokens: None,
        cached_tokens: None,
        cache_creation_tokens,
        cache_creation_token_details: None,
        cache_write_tokens,
        extra,
    };
    let original_extra = raw.extra.clone();
    let details = PromptTokensDetails::from(raw);
    assert_eq!(details.extra, original_extra);
    assert_eq!(details.cache_creation_tokens, expected);
    assert_eq!(details.cache_write_tokens, expected);
}

#[test]
fn usage_copies_top_level_cache_counters_into_prompt_details() {
    let raw = serde_json::json!({
        "prompt_tokens": 100,
        "completion_tokens": 10,
        "total_tokens": 110,
        "prompt_cache_hit_tokens": 5,
        "cache_read_input_tokens": 20,
        "cache_creation_input_tokens": 20,
        "prompt_tokens_details": {
            "text_tokens": 80,
            "cached_tokens": 0,
            "cache_creation_tokens": 0
        }
    });
    let usage: Usage = serde_json::from_value(raw).expect("usage parses");
    let details = usage.prompt_tokens_details.expect("details");
    assert_eq!(details.cached_tokens, Some(20));
    assert_eq!(details.cache_creation_tokens, Some(20));
    assert_eq!(details.cache_write_tokens, Some(20));
    assert_eq!(usage.cache_read_input_tokens, Some(20));
    assert_eq!(usage.cache_creation_input_tokens, Some(20));
    assert_eq!(
        usage.extra.get("prompt_cache_hit_tokens"),
        Some(&Json::Number(serde_json::Number::from(5)))
    );
}

#[test]
fn usage_with_cache_propagates_a_typed_error_instead_of_wrapping_the_total() {
    let usage = complete_usage(1, 5, 1, 0, 0);
    let error =
        usage_with_cache(&usage, i64::MAX, 0, 0, 0).expect_err("total_tokens overflows i64");
    assert_eq!(error, BaselineHistoryError::CounterOverflow);
}

#[rstest]
#[case::hit("prompt_cache_hit_tokens", Some(20), None)]
#[case::read("cache_read_input_tokens", Some(20), None)]
#[case::creation("cache_creation_input_tokens", None, Some(20))]
fn top_level_counters_create_missing_prompt_details(
    #[case] field: &str,
    #[case] cached: Option<i64>,
    #[case] creation: Option<i64>,
) {
    let input =
        format!(r#"{{"prompt_tokens":100,"completion_tokens":0,"total_tokens":100,"{field}":20}}"#);
    let usage: Usage = serde_json::from_str(&input).expect("usage parses");
    let details = usage.prompt_tokens_details.expect("details created");
    assert_eq!(details.cached_tokens, cached);
    assert_eq!(details.cache_write_tokens, creation);
    assert_eq!(details.cache_creation_tokens, creation);
}

#[test]
fn usage_without_any_creation_count_is_incomplete() {
    let input = r#"{"prompt_tokens":10,"completion_tokens":0,"total_tokens":10,
        "prompt_tokens_details":{"text_tokens":10,"cached_tokens":0}}"#;
    let usage: Usage = serde_json::from_str(input).expect("usage parses");
    assert!(!is_usage_complete(Some(&usage)));
}

#[rstest]
#[case::zero("0", Some(0))]
#[case::float("20.5", Some(7))]
#[case::text("\"20\"", Some(7))]
#[case::boolean("true", Some(7))]
#[case::null("null", Some(7))]
fn prompt_cache_hit_only_copies_integers(#[case] value: &str, #[case] expected: Option<i64>) {
    let input = format!(
        r#"{{"prompt_tokens":10,"completion_tokens":0,"total_tokens":10,
            "prompt_cache_hit_tokens":{value},"prompt_tokens_details":{{"cached_tokens":7}}}}"#
    );
    let usage: Usage = serde_json::from_str(&input).expect("usage parses");
    let details = usage.prompt_tokens_details.expect("details remain");
    assert_eq!(details.cached_tokens, expected);
    assert!(usage.extra.contains_key("prompt_cache_hit_tokens"));
}
