#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{CounterFailed, FailingCounter, FixedCounter, json, rendered, usage};
use crate::converse_usage::cache_details::{CacheCreationTokenDetails, parse_cache_details};
use crate::converse_usage::error::UsageError;
use crate::converse_usage::reasoning::{
    thinking_tokens_from_fields, transform_reasoning_content, transform_thinking_blocks,
};
use crate::converse_usage::transform::{transform_usage, usage_from_batch_output};
use crate::converse_usage::wire::is_converse_usage_shape;

#[rstest]
#[case::cache_read_write_tokens(cache_read_write_tokens)]
#[case::cache_detail_ttl(cache_detail_ttl)]
#[case::additional_thinking_tokens(additional_thinking_tokens)]
#[case::batch_usage_shape(batch_usage_shape)]
#[case::negative_or_missing_counts(negative_or_missing_counts)]
#[case::total_token_normalization(total_token_normalization)]
#[case::reasoning_and_thinking_blocks(reasoning_and_thinking_blocks)]
fn converse_usage_cases(#[case] check: fn()) {
    check();
}

fn at<'value>(value: &'value Json, path: &[&str]) -> &'value Json {
    let mut current = value;
    for key in path {
        let Json::Object(fields) = current else {
            panic!("{key} is not inside an object");
        };
        current = fields
            .get(*key)
            .unwrap_or_else(|| panic!("the object has no {key}"));
    }
    current
}

fn has_key(value: &Json, key: &str) -> bool {
    match value {
        Json::Object(fields) => fields.contains_key(key),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Null => false,
    }
}

fn list(value: &Json) -> &[Json] {
    let Json::List(items) = value else {
        panic!("the fixture is not a list");
    };
    items
}

fn plain(source: &str) -> Json {
    let counter = FixedCounter(0);
    rendered(&transform_usage(&usage(source), None, false, None, &counter).unwrap())
}

fn estimated(source: &str, content: &str, estimate: usize) -> Json {
    let counter = FixedCounter(estimate);
    rendered(&transform_usage(&usage(source), Some(content), false, None, &counter).unwrap())
}

fn provided(source: &str, thinking_ran: bool, provider: Option<i64>) -> Json {
    let counter = FixedCounter(0);
    rendered(&transform_usage(&usage(source), None, thinking_ran, provider, &counter).unwrap())
}

fn batch(source: &str) -> Json {
    rendered(&usage_from_batch_output(&usage(source)).unwrap())
}

fn transform_error(source: &str) -> UsageError<CounterFailed> {
    transform_usage(&usage(source), None, false, None, &FixedCounter(0)).unwrap_err()
}

fn cache_read_write_tokens() {
    assert_eq!(
        plain(
            r#"{"inputTokens": 10, "outputTokens": 5, "cacheReadInputTokens": 2, "cacheReadInputTokenCount": 99, "cacheWriteInputTokens": 3, "cacheWriteInputTokenCount": 88}"#
        ),
        json(
            r#"{
                "completion_tokens": 5,
                "prompt_tokens": 15,
                "total_tokens": 20,
                "completion_tokens_details": {"accepted_prediction_tokens": null, "audio_tokens": null, "image_tokens": null, "reasoning_tokens": 0, "rejected_prediction_tokens": null, "text_tokens": 5, "video_tokens": null},
                "prompt_tokens_details": {"audio_tokens": null, "cache_creation_tokens": 3, "cache_write_tokens": 3, "cached_tokens": 2, "image_tokens": null, "text_tokens": 10, "video_tokens": null},
                "cache_creation_input_tokens": 3,
                "cache_read_input_tokens": 2
            }"#
        )
    );
    cache_alias_edges();
}

fn cache_alias_edges() {
    let fallback = plain(
        r#"{"inputTokens": 1, "outputTokens": 1, "cacheReadInputTokens": true, "cacheReadInputTokenCount": 7, "cacheWriteInputTokens": "5", "cacheWriteInputTokenCount": 4}"#,
    );
    assert_eq!(at(&fallback, &["prompt_tokens"]), &json("12"));
    assert_eq!(
        at(&fallback, &["prompt_tokens_details", "cached_tokens"]),
        &json("7")
    );
    assert_eq!(
        at(
            &fallback,
            &["prompt_tokens_details", "cache_creation_tokens"]
        ),
        &json("4")
    );
    let zero = plain(
        r#"{"inputTokens": 1, "outputTokens": 1, "cacheReadInputTokens": 0, "cacheReadInputTokenCount": 9}"#,
    );
    assert_eq!(
        at(&zero, &["prompt_tokens_details", "cached_tokens"]),
        &json("0")
    );
    let negative = plain(
        r#"{"inputTokens": 1, "outputTokens": 1, "cacheReadInputTokens": -2, "cacheWriteInputTokens": 3.9}"#,
    );
    assert_eq!(at(&negative, &["cache_read_input_tokens"]), &json("-2"));
    assert_eq!(at(&negative, &["cache_creation_input_tokens"]), &json("3"));
    assert_eq!(at(&negative, &["prompt_tokens"]), &json("2"));
}

fn cache_detail_ttl() {
    assert_eq!(
        plain(
            r#"{"inputTokens": 10, "outputTokens": 5, "cacheWriteInputTokenCount": 3, "cacheDetails": [{"inputTokens": 1, "ttl": "5m"}, {"inputTokens": 2, "ttl": "1h"}]}"#
        ),
        json(
            r#"{
                "completion_tokens": 5,
                "prompt_tokens": 13,
                "total_tokens": 18,
                "completion_tokens_details": {"accepted_prediction_tokens": null, "audio_tokens": null, "image_tokens": null, "reasoning_tokens": 0, "rejected_prediction_tokens": null, "text_tokens": 5, "video_tokens": null},
                "prompt_tokens_details": {"audio_tokens": null, "cache_creation_tokens": 3, "cache_write_tokens": 3, "cached_tokens": 0, "image_tokens": null, "text_tokens": 10, "video_tokens": null, "cache_creation_token_details": {"ephemeral_5m_input_tokens": 1, "ephemeral_1h_input_tokens": 2}},
                "cache_creation_input_tokens": 3,
                "cache_read_input_tokens": 0
            }"#
        )
    );
    let partial = plain(
        r#"{"inputTokens": 1, "outputTokens": 1, "cacheWriteInputTokens": 9, "cacheDetails": [{"inputTokens": 5, "ttl": "5m"}]}"#,
    );
    assert!(!has_key(
        at(&partial, &["prompt_tokens_details"]),
        "cache_creation_token_details"
    ));
    let unknown_only = plain(
        r#"{"inputTokens": 1, "outputTokens": 1, "cacheWriteInputTokens": 0, "cacheDetails": [{"inputTokens": 5, "ttl": "2h"}, {"inputTokens": 7}]}"#,
    );
    assert_eq!(
        at(
            &unknown_only,
            &["prompt_tokens_details", "cache_creation_token_details"]
        ),
        &json(r#"{"ephemeral_5m_input_tokens": 0, "ephemeral_1h_input_tokens": 0}"#)
    );
    cache_detail_edges();
}

fn cache_detail_edges() {
    let _missing = parse_cache_details(
        &usage(r#"{"inputTokens": 1, "outputTokens": 1, "cacheWriteInputTokens": 0, "cacheDetails": [{"ttl": "1h"}]}"#),
    )
    .unwrap_err();
    let empty = usage(r#"{"inputTokens": 1, "outputTokens": 1, "cacheDetails": []}"#);
    assert_eq!(parse_cache_details(&empty).unwrap(), None);
    let null = usage(r#"{"inputTokens": 1, "outputTokens": 1, "cacheDetails": null}"#);
    assert_eq!(parse_cache_details(&null).unwrap(), None);
    let excessive = usage(
        r#"{"inputTokens": 1, "outputTokens": 1, "cacheWriteInputTokens": 3, "cacheDetails": [{"inputTokens": 3, "ttl": "5m"}, {"inputTokens": 1, "ttl": "1h"}]}"#,
    );
    assert_eq!(parse_cache_details(&excessive).unwrap(), None);
    let negative = usage(
        r#"{"inputTokens": 1, "outputTokens": 1, "cacheWriteInputTokens": -1, "cacheDetails": [{"inputTokens": -1, "ttl": "5m"}]}"#,
    );
    assert_eq!(
        parse_cache_details(&negative).unwrap(),
        Some(CacheCreationTokenDetails {
            ephemeral_5m_input_tokens: -1,
            ephemeral_1h_input_tokens: 0,
        })
    );
    let fractional = usage(
        r#"{"inputTokens": 1, "outputTokens": 1, "cacheWriteInputTokens": 3, "cacheDetails": [{"inputTokens": 1.5, "ttl": "5m"}, {"inputTokens": 1.5, "ttl": "1h"}]}"#,
    );
    let _fractional = parse_cache_details(&fractional).unwrap_err();
}

fn additional_thinking_tokens() {
    assert_eq!(
        thinking_tokens_from_fields(&json(
            r#"{"usage": {"output_tokens_details": {"thinking_tokens": 4}}}"#
        )),
        Some(4)
    );
    assert_eq!(
        thinking_tokens_from_fields(&json(
            r#"{"usage": {"output_tokens_details": {"thinking_tokens": true}}}"#
        )),
        Some(1)
    );
    assert_eq!(
        thinking_tokens_from_fields(&json(
            r#"{"usage": {"output_tokens_details": {"thinking_tokens": "4"}}}"#
        )),
        None
    );
    assert_eq!(
        thinking_tokens_from_fields(&json(r#"{"usage": {"output_tokens_details": 5}}"#)),
        None
    );
    assert_eq!(thinking_tokens_from_fields(&json(r#"{"usage": 5}"#)), None);
    assert_eq!(thinking_tokens_from_fields(&json("[]")), None);
    reasoning_estimate_branches();
    reasoning_estimate_edges();
    reasoning_provider_branches();
    reasoning_thinking_ran();
    reasoning_counter_failure();
}

fn reasoning_tokens(value: &Json) -> &Json {
    at(value, &["completion_tokens_details", "reasoning_tokens"])
}

fn text_tokens(value: &Json) -> &Json {
    at(value, &["completion_tokens_details", "text_tokens"])
}

fn reasoning_estimate_branches() {
    assert_eq!(
        estimated(
            r#"{"inputTokens": 1, "outputTokens": 10}"#,
            "hello world",
            2
        ),
        json(
            r#"{
                "completion_tokens": 10,
                "prompt_tokens": 1,
                "total_tokens": 11,
                "completion_tokens_details": {"accepted_prediction_tokens": null, "audio_tokens": null, "image_tokens": null, "reasoning_tokens": 2, "rejected_prediction_tokens": null, "text_tokens": 8, "video_tokens": null},
                "prompt_tokens_details": {"audio_tokens": null, "cache_creation_tokens": 0, "cache_write_tokens": 0, "cached_tokens": 0, "image_tokens": null, "text_tokens": 1, "video_tokens": null},
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0
            }"#
        )
    );
}

fn reasoning_estimate_edges() {
    let over = estimated(
        r#"{"inputTokens": 1, "outputTokens": 10}"#,
        "hello world",
        20,
    );
    assert_eq!(reasoning_tokens(&over), &json("20"));
    assert_eq!(text_tokens(&over), &json("-10"));
    let empty = estimated(r#"{"inputTokens": 1, "outputTokens": 10}"#, "", 9);
    assert_eq!(reasoning_tokens(&empty), &json("0"));
}

fn reasoning_provider_branches() {
    let provider_wins = {
        let counter = FixedCounter(2);
        rendered(
            &transform_usage(
                &usage(r#"{"inputTokens": 1, "outputTokens": 10}"#),
                Some("hello world"),
                false,
                Some(4),
                &counter,
            )
            .unwrap(),
        )
    };
    assert_eq!(reasoning_tokens(&provider_wins), &json("4"));
    assert_eq!(text_tokens(&provider_wins), &json("6"));
    let capped = provided(r#"{"inputTokens": 1, "outputTokens": 10}"#, false, Some(40));
    assert_eq!(reasoning_tokens(&capped), &json("10"));
    assert_eq!(text_tokens(&capped), &json("0"));
    let negative = provided(r#"{"inputTokens": 1, "outputTokens": 10}"#, false, Some(-4));
    assert_eq!(reasoning_tokens(&negative), &json("0"));
    assert_eq!(text_tokens(&negative), &json("10"));
}

fn reasoning_thinking_ran() {
    let thinking_zero = provided(r#"{"inputTokens": 1, "outputTokens": 10}"#, true, Some(0));
    assert_eq!(
        at(
            &thinking_zero,
            &["completion_tokens_details", "reasoning_tokens"]
        ),
        &Json::Null
    );
    assert_eq!(
        at(
            &thinking_zero,
            &["completion_tokens_details", "text_tokens"]
        ),
        &Json::Null
    );
    let thinking_plain = provided(r#"{"inputTokens": 1, "outputTokens": 10}"#, true, None);
    assert_eq!(
        at(
            &thinking_plain,
            &["completion_tokens_details", "reasoning_tokens"]
        ),
        &Json::Null
    );
}

fn reasoning_counter_failure() {
    let failed = transform_usage(
        &usage(r#"{"inputTokens": 1, "outputTokens": 10}"#),
        Some("hello world"),
        false,
        None,
        &FailingCounter,
    );
    assert!(matches!(failed, Err(UsageError::Counter { .. })));
}

fn batch_zero_total() {
    assert_eq!(
        batch(
            r#"{"inputTokens": 2, "outputTokens": 3, "totalTokens": 0, "cacheReadInputTokens": 4, "cacheWriteInputTokens": 5}"#
        ),
        json(
            r#"{
                "completion_tokens": 3,
                "prompt_tokens": 11,
                "total_tokens": 5,
                "completion_tokens_details": {"accepted_prediction_tokens": null, "audio_tokens": null, "image_tokens": null, "reasoning_tokens": 0, "rejected_prediction_tokens": null, "text_tokens": 3, "video_tokens": null},
                "prompt_tokens_details": {"audio_tokens": null, "cache_creation_tokens": 5, "cache_write_tokens": 5, "cached_tokens": 4, "image_tokens": null, "text_tokens": 2, "video_tokens": null},
                "cache_creation_input_tokens": 5,
                "cache_read_input_tokens": 4
            }"#
        )
    );
}

fn batch_cache_only() {
    assert_eq!(
        batch(r#"{"cacheReadInputTokens": 3, "cacheWriteInputTokens": 4, "totalTokens": 9}"#),
        json(
            r#"{
                "completion_tokens": 0,
                "prompt_tokens": 7,
                "total_tokens": 9,
                "completion_tokens_details": {"accepted_prediction_tokens": null, "audio_tokens": null, "image_tokens": null, "reasoning_tokens": 0, "rejected_prediction_tokens": null, "text_tokens": 0, "video_tokens": null},
                "prompt_tokens_details": {"audio_tokens": null, "cache_creation_tokens": 4, "cache_write_tokens": 4, "cached_tokens": 3, "image_tokens": null, "text_tokens": 0, "video_tokens": null},
                "cache_creation_input_tokens": 4,
                "cache_read_input_tokens": 3
            }"#
        )
    );
}

fn batch_usage_shape() {
    batch_zero_total();
    batch_cache_only();
    let empty = batch("{}");
    assert_eq!(at(&empty, &["prompt_tokens"]), &json("0"));
    assert_eq!(at(&empty, &["cache_read_input_tokens"]), &json("0"));
    let kept = batch(
        r#"{"inputTokens": 1, "outputTokens": 1, "cacheWriteInputTokens": 3, "cacheDetails": [{"inputTokens": 3, "ttl": "5m"}]}"#,
    );
    assert!(!has_key(
        at(&kept, &["prompt_tokens_details"]),
        "cache_creation_token_details"
    ));
    batch_shape();
}

fn batch_shape() {
    assert!(is_converse_usage_shape(&json(r#"{"inputTokens": null}"#)));
    assert!(is_converse_usage_shape(&json(r#"{"outputTokens": null}"#)));
    assert!(is_converse_usage_shape(&json(
        r#"{"inputTokens": 1, "outputTokens": 2}"#
    )));
    assert!(!is_converse_usage_shape(&json("{}")));
    assert!(!is_converse_usage_shape(&json(
        r#"{"cacheReadInputTokens": 1}"#
    )));
    assert!(!is_converse_usage_shape(&json(r#"{"totalTokens": 1}"#)));
    assert!(!is_converse_usage_shape(&json("[]")));
}

fn negative_or_missing_counts() {
    let skipped = batch(
        r#"{"inputTokens": true, "outputTokens": "5", "cacheReadInputTokens": null, "cacheReadInputTokenCount": 7, "cacheWriteInputTokens": false, "cacheWriteInputTokenCount": 2}"#,
    );
    assert_eq!(at(&skipped, &["prompt_tokens"]), &json("9"));
    assert_eq!(at(&skipped, &["completion_tokens"]), &json("0"));
    assert_eq!(
        at(&skipped, &["prompt_tokens_details", "cached_tokens"]),
        &json("7")
    );
    let truncated = batch(
        r#"{"inputTokens": 3.9, "outputTokens": -3.9, "cacheReadInputTokens": 1.5, "cacheWriteInputTokens": -1.5}"#,
    );
    assert_eq!(at(&truncated, &["prompt_tokens"]), &json("3"));
    assert_eq!(at(&truncated, &["completion_tokens"]), &json("-3"));
    assert_eq!(
        at(&truncated, &["prompt_tokens_details", "cached_tokens"]),
        &json("1")
    );
    live_counts_are_required();
}

fn live_counts_are_required() {
    assert!(
        transform_error(r#"{"outputTokens": 2}"#)
            .to_string()
            .contains("inputTokens")
    );
    assert!(
        transform_error(r#"{"inputTokens": 2}"#)
            .to_string()
            .contains("outputTokens")
    );
    assert!(
        transform_error(r#"{"inputTokens": null, "outputTokens": 2}"#)
            .to_string()
            .contains("inputTokens")
    );
    let boolean = transform_error(r#"{"inputTokens": true, "outputTokens": 2}"#);
    assert!(matches!(boolean, UsageError::Usage(_)));
    let fractional = transform_error(r#"{"inputTokens": 1.5, "outputTokens": 2}"#);
    assert!(matches!(fractional, UsageError::Usage(_)));
    let _huge_float =
        usage_from_batch_output(&usage(r#"{"inputTokens": 1e400, "outputTokens": 1}"#))
            .unwrap_err();
    let _huge = usage_from_batch_output(&usage(
        r#"{"inputTokens": 9223372036854775808, "outputTokens": 1}"#,
    ))
    .unwrap_err();
}

fn total_token_normalization() {
    let absent = plain(r#"{"inputTokens": 2, "outputTokens": 3}"#);
    assert_eq!(at(&absent, &["total_tokens"]), &json("5"));
    let zero = plain(r#"{"inputTokens": 2, "outputTokens": 3, "totalTokens": 0}"#);
    assert_eq!(at(&zero, &["total_tokens"]), &json("0"));
    let null = plain(r#"{"inputTokens": 2, "outputTokens": 3, "totalTokens": null}"#);
    assert_eq!(at(&null, &["total_tokens"]), &json("0"));
    let negative = plain(r#"{"inputTokens": 2, "outputTokens": 3, "totalTokens": -3}"#);
    assert_eq!(at(&negative, &["total_tokens"]), &json("-3"));
    let inconsistent = plain(
        r#"{"inputTokens": 2, "outputTokens": 3, "cacheReadInputTokens": 1, "totalTokens": 4}"#,
    );
    assert_eq!(at(&inconsistent, &["prompt_tokens"]), &json("3"));
    assert_eq!(at(&inconsistent, &["total_tokens"]), &json("4"));
    batch_total_normalization();
}

fn batch_total_normalization() {
    let zero = batch(r#"{"inputTokens": 2, "outputTokens": 3, "totalTokens": 0}"#);
    assert_eq!(at(&zero, &["total_tokens"]), &json("5"));
    let null = batch(r#"{"inputTokens": 2, "outputTokens": 3, "totalTokens": null}"#);
    assert_eq!(at(&null, &["total_tokens"]), &json("5"));
    let negative = batch(r#"{"inputTokens": 2, "outputTokens": 3, "totalTokens": -1}"#);
    assert_eq!(at(&negative, &["total_tokens"]), &json("-1"));
    let supplied = batch(r#"{"inputTokens": 2, "outputTokens": 3, "totalTokens": 9}"#);
    assert_eq!(at(&supplied, &["total_tokens"]), &json("9"));
}

fn block_projections() {
    let blocks = json(
        r#"[
            {"reasoningText": {"text": "a", "signature": "s1"}},
            {"reasoningText": {"text": "b"}},
            {"redactedContent": "abc"},
            {"unknown": 1},
            {"redactedContent": null},
            {"reasoningText": {"text": "", "signature": ""}}
        ]"#,
    );
    assert_eq!(transform_reasoning_content(list(&blocks)).unwrap(), "ab");
    assert_eq!(
        rendered(&transform_thinking_blocks(list(&blocks)).unwrap()),
        json(
            r#"[
                {"type": "thinking", "thinking": "a", "signature": "s1"},
                {"type": "thinking", "thinking": "b"},
                {"type": "redacted_thinking", "data": "abc"},
                {"type": "redacted_thinking", "data": null},
                {"type": "thinking", "thinking": "", "signature": ""}
            ]"#
        )
    );
}

fn reasoning_and_thinking_blocks() {
    block_projections();
    block_edges();
}

fn block_edges() {
    let signature_only = json(r#"[{"reasoningText": {"signature": "s"}}]"#);
    assert_eq!(
        rendered(&transform_thinking_blocks(list(&signature_only)).unwrap()),
        json(r#"[{"type": "thinking", "signature": "s"}]"#)
    );
    let _missing_text = transform_reasoning_content(list(&signature_only)).unwrap_err();
    let null_text = json(r#"[{"reasoningText": {"text": null}}]"#);
    let _null_text = transform_reasoning_content(list(&null_text)).unwrap_err();
    assert_eq!(
        rendered(&transform_thinking_blocks(list(&null_text)).unwrap()),
        json(r#"[{"type": "thinking"}]"#)
    );
    let both = json(r#"[{"reasoningText": {"text": "t"}, "redactedContent": "r"}]"#);
    assert_eq!(transform_reasoning_content(list(&both)).unwrap(), "t");
    assert_eq!(
        rendered(&transform_thinking_blocks(list(&both)).unwrap()),
        json(r#"[{"type": "thinking", "thinking": "t"}]"#)
    );
    assert_eq!(transform_reasoning_content(&[]).unwrap(), "");
    assert_eq!(
        rendered(&transform_thinking_blocks(&[]).unwrap()),
        json("[]")
    );
}
