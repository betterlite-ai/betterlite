#![cfg(test)]

use crate::cost_usage::modality_projection::CacheCreationTokenDetails;
use crate::savings_evidence::usage::{RawUsageObject, usage_from_spend_log};
use crate::savings_pricing::tests::record;

use super::{
    RequestUsage, baseline_request_usage, billed_usage, cost_usage, prompt_extras, request_usage,
};

fn raw(json: &str) -> RawUsageObject {
    serde_json::from_str(json).expect("the fixture is a usage object")
}

fn usage(json: &str) -> RequestUsage {
    request_usage(Some(&raw(json))).expect("the fixture parses")
}

#[test]
fn request_usage_is_none_without_a_record() {
    assert!(request_usage(None).is_none());
    let empty = raw("{}");
    assert!(request_usage(Some(&empty)).is_none());
}

#[test]
fn request_usage_reads_the_logged_counts() {
    let parsed = usage(
        r#"{"prompt_tokens": 10, "completion_tokens": 4, "total_tokens": 14,
            "cache_read_input_tokens": 6, "cache_creation_input_tokens": 3}"#,
    );
    assert_eq!(parsed.usage.prompt_tokens, 10);
    assert_eq!(parsed.usage.completion_tokens, 4);
    assert_eq!(parsed.usage.total_tokens, 14);
    assert_eq!(parsed.usage.cache_read_input_tokens, 6);
    assert_eq!(parsed.usage.cache_creation_input_tokens, 3);
    let details = parsed
        .usage
        .prompt_tokens_details
        .expect("a top-level cache count creates the details");
    assert_eq!(details.cached_tokens, 6);
    assert_eq!(details.cache_write_tokens, 3);
    assert_eq!(details.cache_creation_tokens, 3);
}

#[test]
fn cost_usage_carries_the_prompt_and_completion_details() {
    let parsed = usage(
        r#"{"prompt_tokens": 12, "completion_tokens": 5, "total_tokens": 17,
            "prompt_tokens_details": {"cached_tokens": 4, "text_tokens": 8,
                "audio_tokens": 1, "image_tokens": 2, "video_tokens": 3},
            "completion_tokens_details": {"reasoning_tokens": 5}}"#,
    );
    let cost = cost_usage(&parsed);
    assert_eq!(cost.prompt_tokens, 12);
    assert_eq!(cost.completion_tokens, 5);
    assert_eq!(cost.total_tokens, 17);
    assert_eq!(cost.cache_read_tokens, Some(4));
    assert_eq!(
        cost.prompt_details.map(|details| details.text_tokens),
        Some(Some(8))
    );
    assert_eq!(
        cost.prompt_details.map(|details| details.audio_tokens),
        Some(Some(1))
    );
    assert_eq!(
        cost.completion_details
            .and_then(|details| details.reasoning_tokens),
        Some(5)
    );
}

#[test]
fn prompt_extras_carry_the_ttl_split() {
    let mut parsed = usage(
        r#"{"prompt_tokens": 10, "cache_read_input_tokens": 4,
            "prompt_tokens_details": {"cache_write_tokens": 6}}"#,
    );
    parsed.cache_creation_token_details = Some(CacheCreationTokenDetails {
        ephemeral_5m_input_tokens: Some(2),
        ephemeral_1h_input_tokens: Some(4),
    });
    let extras = prompt_extras(&parsed);
    assert_eq!(extras.cache_write_tokens, Some(6));
    assert_eq!(extras.cache_creation_tokens, Some(6));
    assert_eq!(
        extras
            .cache_creation_token_details
            .and_then(|details| details.ephemeral_5m_input_tokens),
        Some(2)
    );
}

#[test]
fn billed_usage_carries_counts_and_ttl() {
    let mut parsed = usage(
        r#"{"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15,
            "prompt_tokens_details": {"cached_tokens": 4, "cache_write_tokens": 6},
            "completion_tokens_details": {"reasoning_tokens": 3}}"#,
    );
    parsed.cache_creation_token_details = Some(CacheCreationTokenDetails {
        ephemeral_5m_input_tokens: Some(2),
        ephemeral_1h_input_tokens: Some(4),
    });
    let billed = billed_usage(&parsed);
    assert_eq!(billed.detail_reasoning, Some(3));
    assert_eq!(billed.detail_cache_read, Some(4));
    assert_eq!(billed.detail_cache_creation, Some(6));
    assert_eq!(billed.detail_cache_creation_5m, Some(2));
    assert_eq!(billed.detail_cache_creation_1h, Some(4));
}

#[test]
fn baseline_request_usage_keeps_priced_cache_buckets() {
    let mut original = usage(
        r#"{"prompt_tokens": 10, "cache_read_input_tokens": 4,
            "prompt_tokens_details": {"cache_write_tokens": 6}}"#,
    );
    original.cache_creation_token_details = Some(CacheCreationTokenDetails {
        ephemeral_5m_input_tokens: Some(2),
        ephemeral_1h_input_tokens: Some(4),
    });
    let info =
        record(r#"{"cache_read_input_token_cost": 0.5, "cache_creation_input_token_cost": 1.0}"#);
    let normalized = baseline_request_usage(&original, Some(&info));
    let details = normalized
        .usage
        .prompt_tokens_details
        .expect("details stay");
    assert_eq!(details.cached_tokens, 4);
    assert_eq!(details.cache_write_tokens, 6);
    assert_eq!(
        normalized
            .cache_creation_token_details
            .and_then(|value| value.ephemeral_1h_input_tokens),
        Some(4)
    );
}

#[test]
fn baseline_request_usage_folds_unpriced_writes_while_reads_stay_priced() {
    let mut original = usage(
        r#"{"prompt_tokens": 10, "cache_read_input_tokens": 4,
            "prompt_tokens_details": {"cache_write_tokens": 6}}"#,
    );
    original.cache_creation_token_details = Some(CacheCreationTokenDetails {
        ephemeral_5m_input_tokens: Some(2),
        ephemeral_1h_input_tokens: Some(4),
    });
    let info = record(r#"{"cache_read_input_token_cost": 0.5}"#);
    let normalized = baseline_request_usage(&original, Some(&info));
    let details = normalized
        .usage
        .prompt_tokens_details
        .expect("details stay");
    assert_eq!(details.cached_tokens, 4);
    assert_eq!(details.cache_write_tokens, 0);
    assert_eq!(details.cache_creation_tokens, 0);
    assert_eq!(details.text_tokens, 6);
    assert_eq!(normalized.usage.cache_creation_input_tokens, 0);
    assert!(normalized.cache_creation_token_details.is_none());
}

#[test]
fn baseline_request_usage_keeps_ttl_when_writes_stay_priced() {
    let mut original =
        usage(r#"{"prompt_tokens": 10, "prompt_tokens_details": {"cached_tokens": 4}}"#);
    original.cache_creation_token_details = Some(CacheCreationTokenDetails {
        ephemeral_5m_input_tokens: Some(2),
        ephemeral_1h_input_tokens: None,
    });
    let info = record(r#"{"cache_read_input_token_cost": 0.5}"#);
    let normalized = baseline_request_usage(&original, Some(&info));
    assert_eq!(
        normalized.cache_creation_token_details,
        original.cache_creation_token_details
    );
}

#[test]
fn baseline_request_usage_folds_unpriced_reads_into_text() {
    let original = usage(r#"{"prompt_tokens": 10, "prompt_tokens_details": {"cached_tokens": 4}}"#);
    let info = record("{}");
    let normalized = baseline_request_usage(&original, Some(&info));
    let details = normalized
        .usage
        .prompt_tokens_details
        .expect("details stay");
    assert_eq!(details.cached_tokens, 0);
    assert_eq!(details.text_tokens, 10);
    assert_eq!(normalized.usage.cache_read_input_tokens, 0);
}

#[test]
fn baseline_request_usage_folds_unpriced_writes_into_text() {
    let mut original =
        usage(r#"{"prompt_tokens": 10, "prompt_tokens_details": {"cache_write_tokens": 6}}"#);
    original.cache_creation_token_details = Some(CacheCreationTokenDetails {
        ephemeral_5m_input_tokens: Some(1),
        ephemeral_1h_input_tokens: None,
    });
    let info = record("{}");
    let normalized = baseline_request_usage(&original, Some(&info));
    let details = normalized
        .usage
        .prompt_tokens_details
        .expect("details stay");
    assert_eq!(details.cache_write_tokens, 0);
    assert_eq!(details.text_tokens, 10);
    assert_eq!(normalized.usage.cache_creation_input_tokens, 0);
    assert!(normalized.cache_creation_token_details.is_none());
}

#[test]
fn baseline_request_usage_returns_the_same_record_without_cache_tokens() {
    let original = usage(r#"{"prompt_tokens": 10, "completion_tokens": 2, "total_tokens": 12}"#);
    let info = record(r#"{"cache_read_input_token_cost": 0.5}"#);
    let normalized = baseline_request_usage(&original, Some(&info));
    assert_eq!(normalized.usage.prompt_tokens, 10);
    assert_eq!(normalized.usage.total_tokens, 12);
    assert!(normalized.usage.prompt_tokens_details.is_none());
}

#[test]
fn parsed_usage_matches_the_evidence_parser() {
    let raw_usage = raw(r#"{"prompt_tokens": 3, "completion_tokens": 1, "total_tokens": 4}"#);
    let expected = usage_from_spend_log(Some(&raw_usage)).expect("the fixture parses");
    let parsed = request_usage(Some(&raw_usage)).expect("the fixture parses");
    assert_eq!(parsed.usage.prompt_tokens, expected.prompt_tokens);
    assert_eq!(parsed.usage.completion_tokens, expected.completion_tokens);
}
