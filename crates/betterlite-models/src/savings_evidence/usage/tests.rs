#![cfg(test)]

use rstest::rstest;

use super::{RawUsageObject, usage_from_spend_log};

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

#[test]
fn usage_from_spend_log_carries_completion_counters_through() {
    let usage_object = raw_usage(r#"{"completion_tokens": 7, "total_tokens": 7}"#);
    let usage = usage_from_spend_log(Some(&usage_object))
        .expect("a nonempty mapping with no prompt fields still rebuilds");
    assert_eq!(usage.completion_tokens, 7);
    assert_eq!(usage.total_tokens, 7);
}

#[test]
fn usage_from_spend_log_carries_completion_tokens_details_through() {
    let usage_object = raw_usage(
        r#"{
            "completion_tokens": 10,
            "completion_tokens_details": {"reasoning_tokens": 4, "audio_tokens": 2}
        }"#,
    );
    let usage = usage_from_spend_log(Some(&usage_object)).expect("a well-shaped record rebuilds");
    let details = usage
        .completion_tokens_details
        .expect("completion_tokens_details carried over");
    assert_eq!(details.reasoning_tokens, Some(4));
    assert_eq!(details.audio_tokens, Some(2));
    assert_eq!(details.text_tokens, None);
}

#[test]
fn usage_from_spend_log_rejects_a_nonnumeric_completion_detail_count() {
    let usage_object = raw_usage(
        r#"{
            "completion_tokens": 10,
            "completion_tokens_details": {"reasoning_tokens": "not-a-number"}
        }"#,
    );
    assert!(usage_from_spend_log(Some(&usage_object)).is_none());
}

#[test]
fn usage_from_spend_log_applies_top_level_cache_overrides_without_nested_details() {
    let usage_object = raw_usage(
        r#"{
            "prompt_tokens": 100,
            "cache_read_input_tokens": 5,
            "cache_creation_input_tokens": 3
        }"#,
    );
    let usage = usage_from_spend_log(Some(&usage_object))
        .expect("top-level cache counts alone still rebuild");
    let details = usage
        .prompt_tokens_details
        .expect("the override creates prompt_tokens_details");
    assert_eq!(details.cached_tokens, 5);
    assert_eq!(details.cache_write_tokens, 3);
}

#[test]
fn usage_from_spend_log_top_level_cache_read_overrides_an_existing_detail() {
    let usage_object = raw_usage(
        r#"{
            "prompt_tokens": 100,
            "cache_read_input_tokens": 9,
            "prompt_tokens_details": {"cached_tokens": 2}
        }"#,
    );
    let usage = usage_from_spend_log(Some(&usage_object)).expect("a well-shaped record rebuilds");
    assert_eq!(
        usage
            .prompt_tokens_details
            .expect("details carried over")
            .cached_tokens,
        9
    );
}
