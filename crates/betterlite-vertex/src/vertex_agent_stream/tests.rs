#![cfg(test)]

use betterlite_models::decisions::{Extra, Json};
use rstest::rstest;

use super::{ASSISTANT_ROLE, AgentEngineStreamError, AgentEngineUsage, parse_chunk};

fn fields(source: &str) -> Extra {
    let Json::Object(fields) = serde_json::from_str::<Json>(source).unwrap() else {
        panic!("a chunk is a JSON object");
    };
    fields
}

fn assert_text(source: &str, content: Option<&str>, role: Option<&str>) {
    let fields = fields(source);
    let chunk = parse_chunk(&fields).unwrap();
    assert_eq!(chunk.content, content);
    assert_eq!(chunk.role, role);
}

fn assert_finish(source: &str, expected: Option<&str>) {
    let fields = fields(source);
    assert_eq!(parse_chunk(&fields).unwrap().finish_reason, expected);
}

fn assert_usage(source: &str, expected: Option<AgentEngineUsage>) {
    let fields = fields(source);
    assert_eq!(parse_chunk(&fields).unwrap().usage, expected);
}

fn field_type(source: &str) -> &'static str {
    let fields = fields(source);
    let Err(AgentEngineStreamError::FieldType { field }) = parse_chunk(&fields) else {
        panic!("expected a field type error");
    };
    field
}

fn usage_count(source: &str) -> &'static str {
    let fields = fields(source);
    let Err(AgentEngineStreamError::UsageCount { field }) = parse_chunk(&fields) else {
        panic!("expected a usage count error");
    };
    field
}

#[test]
fn the_first_dictionary_part_with_a_text_key_wins() {
    assert_text(
        r#"{"content": {"parts": [{}, {"role": "model"}, {"text": "first"}, {"text": "second"}]}}"#,
        Some("first"),
        Some(ASSISTANT_ROLE),
    );
}

#[rstest]
#[case::empty(r#"{"content": {"parts": [{"text": ""}]}}"#)]
#[case::empty_after_a_part(r#"{"content": {"parts": [{"thought": true}, {"text": ""}]}}"#)]
fn an_empty_text_is_kept_and_names_no_role(#[case] source: &str) {
    assert_text(source, Some(""), None);
}

#[rstest]
#[case::no_keys("{}")]
#[case::null_content(r#"{"content": null}"#)]
#[case::empty_content(r#"{"content": {}}"#)]
#[case::no_parts(r#"{"content": {"role": "model"}}"#)]
#[case::null_parts(r#"{"content": {"parts": null}}"#)]
#[case::empty_parts(r#"{"content": {"parts": []}}"#)]
#[case::part_without_text(r#"{"content": {"parts": [{"role": "model"}]}}"#)]
#[case::scalar_part(r#"{"content": {"parts": [3]}}"#)]
#[case::null_part(r#"{"content": {"parts": [null]}}"#)]
#[case::null_text_before_text(r#"{"content": {"parts": [{"text": null}, {"text": "later"}]}}"#)]
fn a_chunk_without_text_has_no_content_and_no_role(#[case] source: &str) {
    assert_text(source, None, None);
}

#[rstest]
#[case::number(r#"{"content": {"parts": [{"text": 5}]}}"#)]
#[case::object(r#"{"content": {"parts": [{"text": {"value": "x"}}]}}"#)]
#[case::list(r#"{"content": {"parts": [{"text": ["x"]}]}}"#)]
#[case::truth(r#"{"content": {"parts": [{"text": true}]}}"#)]
fn a_non_text_part_names_the_text_field(#[case] source: &str) {
    assert_eq!(field_type(source), "text");
}

#[rstest]
#[case::number(r#"{"content": 5}"#)]
#[case::text(r#"{"content": "parts"}"#)]
#[case::list(r#"{"content": []}"#)]
#[case::truth(r#"{"content": true}"#)]
fn a_non_object_content_names_the_content_field(#[case] source: &str) {
    assert_eq!(field_type(source), "content");
}

#[rstest]
#[case::number(r#"{"content": {"parts": 5}}"#)]
#[case::text(r#"{"content": {"parts": "nope"}}"#)]
#[case::object(r#"{"content": {"parts": {}}}"#)]
#[case::truth(r#"{"content": {"parts": true}}"#)]
fn a_non_list_parts_names_the_parts_field(#[case] source: &str) {
    assert_eq!(field_type(source), "parts");
}

#[rstest]
#[case::stop("STOP", Some("stop"))]
#[case::stop_lower("stop", Some("stop"))]
#[case::max_tokens("MAX_TOKENS", Some("length"))]
#[case::max_tokens_lower("max_tokens", Some("length"))]
#[case::length("length", Some("length"))]
#[case::tool_use("tool_use", Some("tool_calls"))]
#[case::tool_calls("tool_calls", Some("tool_calls"))]
#[case::end_turn("end_turn", Some("stop"))]
#[case::refusal("refusal", Some("content_filter"))]
#[case::content_filtered("content_filtered", Some("content_filter"))]
#[case::error("error", Some("stop"))]
fn a_finish_reason_is_lowercased_and_mapped(#[case] raw: &str, #[case] mapped: Option<&str>) {
    let source = format!(r#"{{"finish_reason": "{raw}"}}"#);
    assert_finish(&source, mapped);
}

// the reason is lowercased before the map runs, so the map's uppercase SAFETY entry never matches
#[rstest]
#[case::upper_safety("SAFETY")]
#[case::unmapped("weird")]
fn a_reason_outside_the_openai_map_becomes_stop(#[case] raw: &str) {
    let source = format!(r#"{{"finish_reason": "{raw}"}}"#);
    assert_finish(&source, Some("stop"));
}

#[rstest]
#[case::missing("{}")]
#[case::null(r#"{"finish_reason": null}"#)]
#[case::empty(r#"{"finish_reason": ""}"#)]
fn a_falsy_finish_reason_is_absent(#[case] source: &str) {
    assert_finish(source, None);
}

#[rstest]
#[case::number(r#"{"finish_reason": 5}"#)]
#[case::truth(r#"{"finish_reason": true}"#)]
#[case::list(r#"{"finish_reason": []}"#)]
#[case::object(r#"{"finish_reason": {}}"#)]
fn a_non_text_finish_reason_names_the_field(#[case] source: &str) {
    assert_eq!(field_type(source), "finish_reason");
}

#[test]
fn a_full_usage_metadata_carries_all_three_counts() {
    assert_usage(
        r#"{"usage_metadata": {"prompt_token_count": 4, "candidates_token_count": 5, "total_token_count": 9}}"#,
        Some(AgentEngineUsage {
            prompt_tokens: 4,
            completion_tokens: 5,
            total_tokens: 9,
        }),
    );
}

#[test]
fn a_partial_usage_metadata_defaults_the_missing_counts_to_zero() {
    assert_usage(
        r#"{"usage_metadata": {"prompt_token_count": 7}}"#,
        Some(AgentEngineUsage {
            prompt_tokens: 7,
            completion_tokens: 0,
            total_tokens: 0,
        }),
    );
}

#[test]
fn the_counts_litellm_does_not_read_are_ignored() {
    assert_usage(
        r#"{"usage_metadata": {"prompt_token_count": 1, "candidates_token_count": 2, "total_token_count": 3, "traffic_type": "ON_DEMAND", "thoughts_token_count": 40}}"#,
        Some(AgentEngineUsage {
            prompt_tokens: 1,
            completion_tokens: 2,
            total_tokens: 3,
        }),
    );
}

#[rstest]
#[case::missing("{}")]
#[case::null(r#"{"usage_metadata": null}"#)]
#[case::empty(r#"{"usage_metadata": {}}"#)]
fn a_missing_or_empty_usage_metadata_is_absent(#[case] source: &str) {
    assert_usage(source, None);
}

#[test]
fn a_null_count_reads_as_zero() {
    assert_usage(
        r#"{"usage_metadata": {"prompt_token_count": null, "candidates_token_count": null, "total_token_count": 2}}"#,
        Some(AgentEngineUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 2,
        }),
    );
}

#[rstest]
#[case::prompt(
    r#"{"usage_metadata": {"prompt_token_count": "4"}}"#,
    "prompt_token_count"
)]
#[case::completion(
    r#"{"usage_metadata": {"candidates_token_count": [4]}}"#,
    "candidates_token_count"
)]
#[case::total(
    r#"{"usage_metadata": {"total_token_count": false}}"#,
    "total_token_count"
)]
fn a_non_number_count_names_its_field(#[case] source: &str, #[case] expected: &'static str) {
    assert_eq!(field_type(source), expected);
}

#[rstest]
#[case::negative(
    r#"{"usage_metadata": {"prompt_token_count": -4}}"#,
    "prompt_token_count"
)]
#[case::fractional(
    r#"{"usage_metadata": {"candidates_token_count": 4.5}}"#,
    "candidates_token_count"
)]
fn a_count_that_is_not_a_token_count_names_its_field(
    #[case] source: &str,
    #[case] expected: &'static str,
) {
    assert_eq!(usage_count(source), expected);
}

#[rstest]
#[case::number(r#"{"usage_metadata": 5}"#)]
#[case::text(r#"{"usage_metadata": "4"}"#)]
#[case::list(r#"{"usage_metadata": []}"#)]
#[case::truth(r#"{"usage_metadata": true}"#)]
fn a_non_object_usage_metadata_names_the_field(#[case] source: &str) {
    assert_eq!(field_type(source), "usage_metadata");
}

// buffered Agent Engine answers fall back to actions.state_delta, but the stream never does
#[test]
fn a_state_delta_is_not_read_as_text() {
    assert_text(
        r#"{"actions": {"state_delta": {"answer": "buffered only"}}}"#,
        None,
        None,
    );
}

#[test]
fn a_chunk_carries_its_text_finish_and_usage_together() {
    let fields = fields(
        r#"{"content": {"parts": [{"text": "done"}]}, "finish_reason": "STOP", "usage_metadata": {"prompt_token_count": 2, "candidates_token_count": 1, "total_token_count": 3}}"#,
    );
    let chunk = parse_chunk(&fields).unwrap();
    assert_eq!(chunk.content, Some("done"));
    assert_eq!(chunk.role, Some(ASSISTANT_ROLE));
    assert_eq!(chunk.finish_reason, Some("stop"));
    assert_eq!(
        chunk.usage,
        Some(AgentEngineUsage {
            prompt_tokens: 2,
            completion_tokens: 1,
            total_tokens: 3,
        })
    );
}
