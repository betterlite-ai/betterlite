#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{CounterFailed, FixtureCounter, json, rendered, usage};
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

const USAGE: &str = r#"{"inputTokens": 1, "outputTokens": 10}"#;
const FULL: &str = r#"{"inputTokens": 10, "outputTokens": 5, "cacheReadInputTokens": 2, "cacheReadInputTokenCount": 99, "cacheWriteInputTokens": 3, "cacheWriteInputTokenCount": 88}"#;
const FALLBACK: &str = r#"{"inputTokens": 1, "outputTokens": 1, "cacheReadInputTokens": true, "cacheReadInputTokenCount": 7, "cacheWriteInputTokens": "5", "cacheWriteInputTokenCount": 4}"#;
const SPLIT: &str = r#"{"inputTokens": 10, "outputTokens": 5, "cacheWriteInputTokenCount": 3, "cacheDetails": [{"inputTokens": 1, "ttl": "5m"}, {"inputTokens": 2, "ttl": "1h"}]}"#;
const BATCH: &str = r#"{"inputTokens": 2, "outputTokens": 3, "totalTokens": 0, "cacheReadInputTokens": 4, "cacheWriteInputTokens": 5}"#;
const SKIPPED: &str = r#"{"inputTokens": true, "outputTokens": "5", "cacheReadInputTokens": null, "cacheReadInputTokenCount": 7, "cacheWriteInputTokens": false, "cacheWriteInputTokenCount": 2}"#;
const TRUNCATED: &str = r#"{"inputTokens": 3.9, "outputTokens": -3.9, "cacheReadInputTokens": 1.5, "cacheWriteInputTokens": -1.5}"#;
const INCONSISTENT: &str =
    r#"{"inputTokens": 2, "outputTokens": 3, "cacheReadInputTokens": 1, "totalTokens": 4}"#;
const CACHED_TOTAL: &str = r#"{"inputTokens": 2, "outputTokens": 3, "cacheReadInputTokens": 4, "cacheWriteInputTokens": 5}"#;

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

fn prompt(value: &Json) -> &Json {
    at(value, &["prompt_tokens"])
}

fn total(value: &Json) -> &Json {
    at(value, &["total_tokens"])
}

fn detail<'value>(value: &'value Json, name: &str) -> &'value Json {
    at(value, &["prompt_tokens_details", name])
}

fn reasoning_tokens(value: &Json) -> &Json {
    at(value, &["completion_tokens_details", "reasoning_tokens"])
}

fn text_tokens(value: &Json) -> &Json {
    at(value, &["completion_tokens_details", "text_tokens"])
}

fn plain(source: &str) -> Json {
    let counter = FixtureCounter(Some(0));
    rendered(&transform_usage(&usage(source), None, false, None, &counter).unwrap())
}

fn estimated(source: &str, content: &str, estimate: i64) -> Json {
    let counter = FixtureCounter(Some(estimate));
    rendered(&transform_usage(&usage(source), Some(content), false, None, &counter).unwrap())
}

fn provided(source: &str, thinking_ran: bool, provider: Option<i64>) -> Json {
    let counter = FixtureCounter(Some(0));
    rendered(&transform_usage(&usage(source), None, thinking_ran, provider, &counter).unwrap())
}

fn batch(source: &str) -> Json {
    rendered(&usage_from_batch_output(&usage(source)).unwrap())
}

fn cache_read_write_tokens() {
    let usage = plain(FULL);
    assert_eq!(prompt(&usage), &json("15"));
    assert_eq!(detail(&usage, "cached_tokens"), &json("2"));
    assert_eq!(detail(&usage, "cache_creation_tokens"), &json("3"));
    assert_eq!(at(&usage, &["cache_read_input_tokens"]), &json("2"));
    assert_eq!(at(&usage, &["cache_creation_input_tokens"]), &json("3"));
    assert_eq!(detail(&usage, "text_tokens"), &json("10"));
    let fallback = plain(FALLBACK);
    assert_eq!(prompt(&fallback), &json("12"));
    assert_eq!(detail(&fallback, "cached_tokens"), &json("7"));
    assert_eq!(detail(&fallback, "cache_creation_tokens"), &json("4"));
    let negative = plain(
        r#"{"inputTokens": 1, "outputTokens": 1, "cacheReadInputTokens": -2, "cacheWriteInputTokens": 3.9}"#,
    );
    assert_eq!(at(&negative, &["cache_read_input_tokens"]), &json("-2"));
    assert_eq!(at(&negative, &["cache_creation_input_tokens"]), &json("3"));
}

fn cache_detail_ttl() {
    let split = plain(SPLIT);
    assert_eq!(prompt(&split), &json("13"));
    assert_eq!(
        detail(&split, "cache_creation_token_details"),
        &json(r#"{"ephemeral_5m_input_tokens": 1, "ephemeral_1h_input_tokens": 2}"#)
    );
    let partial = plain(
        r#"{"inputTokens": 1, "outputTokens": 1, "cacheWriteInputTokens": 9, "cacheDetails": [{"inputTokens": 5, "ttl": "5m"}]}"#,
    );
    assert!(!has_key(
        at(&partial, &["prompt_tokens_details"]),
        "cache_creation_token_details"
    ));
    let unknown = plain(
        r#"{"inputTokens": 1, "outputTokens": 1, "cacheWriteInputTokens": 0, "cacheDetails": [{"inputTokens": 5, "ttl": "2h"}, {"inputTokens": 7}]}"#,
    );
    assert_eq!(
        detail(&unknown, "cache_creation_token_details"),
        &json(r#"{"ephemeral_5m_input_tokens": 0, "ephemeral_1h_input_tokens": 0}"#)
    );
    let unknown_without_tokens = plain(
        r#"{"inputTokens": 1, "outputTokens": 1, "cacheWriteInputTokens": 0, "cacheDetails": [{"ttl": "2h"}, {}]}"#,
    );
    assert_eq!(
        detail(&unknown_without_tokens, "cache_creation_token_details"),
        &json(r#"{"ephemeral_5m_input_tokens": 0, "ephemeral_1h_input_tokens": 0}"#)
    );
    let empty = usage(r#"{"inputTokens": 1, "outputTokens": 1, "cacheDetails": []}"#);
    assert_eq!(parse_cache_details(&empty).unwrap(), None);
    let _missing = parse_cache_details(&usage(
        r#"{"inputTokens": 1, "outputTokens": 1, "cacheWriteInputTokens": 0, "cacheDetails": [{"ttl": "1h"}]}"#,
    ))
    .unwrap_err();
    cache_detail_mismatches();
}

fn cache_detail_mismatches() {
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
        r#"{"inputTokens": 1, "outputTokens": 1, "cacheWriteInputTokens": 3, "cacheDetails": [{"inputTokens": 1.5, "ttl": "5m"}]}"#,
    );
    let _fractional = parse_cache_details(&fractional).unwrap_err();
    cache_detail_scalars();
}

fn cache_detail_scalars() {
    for details in ["0", "-0.0", "false", "\"\"", "{}", "null"] {
        let falsey = usage(&format!(
            r#"{{"inputTokens": 1, "outputTokens": 1, "cacheDetails": {details}}}"#
        ));
        assert_eq!(parse_cache_details(&falsey).unwrap(), None, "{details}");
    }
    for details in ["1", "true", "\"x\"", "{\"a\": 1}"] {
        let truthy = usage(&format!(
            r#"{{"inputTokens": 1, "outputTokens": 1, "cacheDetails": {details}}}"#
        ));
        let _not_a_list = parse_cache_details(&truthy).unwrap_err();
    }
}

fn additional_thinking_tokens() {
    for (count, expected) in [
        ("4", Some(4)),
        ("true", Some(1)),
        ("-3", Some(-3)),
        ("\"4\"", None),
        ("2.5", None),
        ("null", None),
    ] {
        let field = json(&format!(
            r#"{{"usage": {{"output_tokens_details": {{"thinking_tokens": {count}}}}}}}"#
        ));
        assert_eq!(thinking_tokens_from_fields(&field), expected);
    }
    for field in [
        r#"{"usage": {"output_tokens_details": {}}}"#,
        r#"{"usage": {"output_tokens_details": 5}}"#,
        r#"{"usage": {"output_tokens": 5}}"#,
        r#"{"usage": 5}"#,
    ] {
        assert_eq!(thinking_tokens_from_fields(&json(field)), None);
    }
    let estimate = estimated(USAGE, "hello world", 2);
    assert_eq!(reasoning_tokens(&estimate), &json("2"));
    assert_eq!(text_tokens(&estimate), &json("8"));
    assert_eq!(prompt(&estimate), &json("1"));
    let over = estimated(USAGE, "hello world", 20);
    assert_eq!(reasoning_tokens(&over), &json("20"));
    assert_eq!(text_tokens(&over), &json("-10"));
    let capped = provided(USAGE, false, Some(40));
    assert_eq!(reasoning_tokens(&capped), &json("10"));
    let negative = provided(USAGE, false, Some(-4));
    assert_eq!(reasoning_tokens(&negative), &json("0"));
    assert_eq!(text_tokens(&negative), &json("10"));
    reasoning_provider_wins();
}

fn reasoning_provider_wins() {
    let counter = FixtureCounter(Some(2));
    let provider = rendered(
        &transform_usage(&usage(USAGE), Some("hello world"), false, Some(4), &counter).unwrap(),
    );
    assert_eq!(reasoning_tokens(&provider), &json("4"));
    assert_eq!(text_tokens(&provider), &json("6"));
    let empty = estimated(USAGE, "", 9);
    assert_eq!(reasoning_tokens(&empty), &json("0"));
    for count in [Some(0), None] {
        let thinking = provided(USAGE, true, count);
        assert_eq!(reasoning_tokens(&thinking), &Json::Null);
        assert_eq!(text_tokens(&thinking), &Json::Null);
    }
    let failed = transform_usage(
        &usage(USAGE),
        Some("hello world"),
        false,
        None,
        &FixtureCounter(None),
    );
    assert!(matches!(failed, Err(UsageError::Counter { .. })));
    let failed_with_provider = transform_usage(
        &usage(USAGE),
        Some("hello world"),
        false,
        Some(4),
        &FixtureCounter(None),
    );
    assert!(matches!(
        failed_with_provider,
        Err(UsageError::Counter { .. })
    ));
}

fn batch_usage_shape() {
    let usage = batch(BATCH);
    assert_eq!(prompt(&usage), &json("11"));
    assert_eq!(total(&usage), &json("5"));
    assert_eq!(detail(&usage, "cached_tokens"), &json("4"));
    assert_eq!(detail(&usage, "cache_creation_tokens"), &json("5"));
    assert_eq!(detail(&usage, "text_tokens"), &json("2"));
    let cache_only =
        batch(r#"{"cacheReadInputTokens": 3, "cacheWriteInputTokens": 4, "totalTokens": 9}"#);
    assert_eq!(prompt(&cache_only), &json("7"));
    assert_eq!(total(&cache_only), &json("9"));
    let kept = batch(
        r#"{"inputTokens": 1, "outputTokens": 1, "cacheWriteInputTokens": 3, "cacheDetails": [{"inputTokens": 3, "ttl": "5m"}]}"#,
    );
    assert!(!has_key(
        at(&kept, &["prompt_tokens_details"]),
        "cache_creation_token_details"
    ));
    assert!(is_converse_usage_shape(&json(r#"{"inputTokens": null}"#)));
    assert!(is_converse_usage_shape(&json(r#"{"outputTokens": null}"#)));
    assert!(!is_converse_usage_shape(&json(r#"{"totalTokens": 1}"#)));
    assert!(!is_converse_usage_shape(&json(
        r#"{"cacheReadInputTokens": 1, "cacheWriteInputTokens": 1, "cacheReadInputTokenCount": 1}"#
    )));
    let empty = batch("{}");
    assert_eq!(prompt(&empty), &json("0"));
}

fn negative_or_missing_counts() {
    let skipped = batch(SKIPPED);
    assert_eq!(prompt(&skipped), &json("9"));
    assert_eq!(detail(&skipped, "cached_tokens"), &json("7"));
    let truncated = batch(TRUNCATED);
    assert_eq!(prompt(&truncated), &json("3"));
    assert_eq!(at(&truncated, &["completion_tokens"]), &json("-3"));
    assert_eq!(detail(&truncated, "cached_tokens"), &json("1"));
    for source in [
        r#"{"outputTokens": 2}"#,
        r#"{"inputTokens": null, "outputTokens": 2}"#,
    ] {
        assert!(transform_error(source).to_string().contains("inputTokens"));
    }
    assert!(
        transform_error(r#"{"inputTokens": 2}"#)
            .to_string()
            .contains("outputTokens")
    );
    live_count_types();
}

fn live_count_types() {
    for source in [
        r#"{"inputTokens": true, "outputTokens": 2}"#,
        r#"{"inputTokens": 1.5, "outputTokens": 2}"#,
    ] {
        assert!(matches!(transform_error(source), UsageError::Usage(_)));
    }
    for source in [
        r#"{"inputTokens": 1e400, "outputTokens": 1}"#,
        r#"{"inputTokens": 9223372036854775808, "outputTokens": 1}"#,
    ] {
        let _overflow = usage_from_batch_output(&usage(source)).unwrap_err();
    }
}

fn transform_error(source: &str) -> UsageError<CounterFailed> {
    transform_usage(&usage(source), None, false, None, &FixtureCounter(Some(0))).unwrap_err()
}

fn total_token_normalization() {
    for (source, expected) in [
        (r#"{"inputTokens": 2, "outputTokens": 3}"#, "5"),
        (CACHED_TOTAL, "14"),
        (
            r#"{"inputTokens": 2, "outputTokens": 3, "totalTokens": 0}"#,
            "0",
        ),
        (
            r#"{"inputTokens": 2, "outputTokens": 3, "totalTokens": null}"#,
            "0",
        ),
        (
            r#"{"inputTokens": 2, "outputTokens": 3, "totalTokens": -3}"#,
            "-3",
        ),
        (INCONSISTENT, "4"),
    ] {
        assert_eq!(total(&plain(source)), &json(expected));
    }
    batch_total_normalization();
}

fn batch_total_normalization() {
    let zero = batch(r#"{"inputTokens": 2, "outputTokens": 3, "totalTokens": 0}"#);
    assert_eq!(total(&zero), &json("5"));
    let null = batch(r#"{"inputTokens": 2, "outputTokens": 3, "totalTokens": null}"#);
    assert_eq!(total(&null), &json("5"));
    let negative = batch(r#"{"inputTokens": 2, "outputTokens": 3, "totalTokens": -1}"#);
    assert_eq!(total(&negative), &json("-1"));
    let supplied = batch(r#"{"inputTokens": 2, "outputTokens": 3, "totalTokens": 9}"#);
    assert_eq!(total(&supplied), &json("9"));
}

fn reasoning_and_thinking_blocks() {
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
    let expected = json(
        r#"[
            {"type": "thinking", "thinking": "a", "signature": "s1"},
            {"type": "thinking", "thinking": "b"},
            {"type": "redacted_thinking", "data": "abc"},
            {"type": "redacted_thinking", "data": null},
            {"type": "thinking", "thinking": "", "signature": ""}
        ]"#,
    );
    assert_eq!(
        rendered(&transform_thinking_blocks(list(&blocks)).unwrap()),
        expected
    );
    block_edges();
}

fn block_edges() {
    let signature_only = json(r#"[{"reasoningText": {"signature": "s"}}]"#);
    assert_eq!(
        rendered(&transform_thinking_blocks(list(&signature_only)).unwrap()),
        json(r#"[{"type": "thinking", "signature": "s"}]"#)
    );
    let _missing_text = transform_reasoning_content(list(&signature_only)).unwrap_err();
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
