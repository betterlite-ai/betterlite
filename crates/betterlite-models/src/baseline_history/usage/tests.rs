#![cfg(test)]

use rstest::rstest;

use super::{
    CacheCreationTokenDetails, PromptTokensDetails, Usage, is_usage_complete, usage_with_cache,
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
            extra: Extra::default(),
        }),
        extra: Extra::default(),
    }
}

#[rstest]
#[case::none(None, false)]
#[case::negative_prompt(Some(complete_usage(-1, 0, -1, 0, 0)), false)]
#[case::missing_details(Some(Usage { prompt_tokens: 1, completion_tokens: 0, total_tokens: 1, prompt_tokens_details: None, extra: Extra::default() }), false)]
#[case::mismatched_total(Some(Usage { prompt_tokens: 5, completion_tokens: 1, total_tokens: 9, prompt_tokens_details: Some(PromptTokensDetails { text_tokens: Some(5), cached_tokens: Some(0), cache_creation_tokens: Some(0), cache_creation_token_details: None, extra: Extra::default() }), extra: Extra::default() }), false)]
#[case::detail_sum_mismatch(Some(Usage { prompt_tokens: 5, completion_tokens: 1, total_tokens: 6, prompt_tokens_details: Some(PromptTokensDetails { text_tokens: Some(1), cached_tokens: Some(1), cache_creation_tokens: Some(0), cache_creation_token_details: None, extra: Extra::default() }), extra: Extra::default() }), false)]
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
    assert_eq!(
        details.extra.get("cache_write_tokens"),
        Some(&Json::Number(serde_json::Number::from(40_i64)))
    );
    assert_eq!(
        rebuilt.extra.get("cache_read_input_tokens"),
        Some(&Json::Number(serde_json::Number::from(20_i64)))
    );
    assert_eq!(
        rebuilt.extra.get("cache_creation_input_tokens"),
        Some(&Json::Number(serde_json::Number::from(40_i64)))
    );
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

#[test]
fn usage_with_cache_propagates_a_typed_error_instead_of_wrapping_the_total() {
    let usage = complete_usage(1, 5, 1, 0, 0);
    let error =
        usage_with_cache(&usage, i64::MAX, 0, 0, 0).expect_err("total_tokens overflows i64");
    assert_eq!(error, BaselineHistoryError::CounterOverflow);
}
