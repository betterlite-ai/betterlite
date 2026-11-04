#![cfg(test)]

use std::{collections::BTreeMap, fs, path::PathBuf};

use rstest::rstest;
use serde_json::json;

use super::{
    CohereIteratorError, CohereStreamVersion, decode_cohere_item, decode_cohere_line,
    validate_environment,
};

#[test]
fn v1_prefers_text_to_finish_and_forwards_citations() {
    let chunk = decode_cohere_line(
        CohereStreamVersion::V1,
        br#"{"text":"hi","is_finished":true,"index":"2","citations":[]}"#,
        false,
    )
    .unwrap();
    assert_eq!(chunk.text, "hi");
    assert!(!chunk.is_finished);
    assert_eq!(chunk.index, 2);
    assert_eq!(
        chunk.provider_specific_fields,
        Some(json!({"citations":[]}))
    );
}

#[test]
fn v2_type_takes_precedence_over_message_end() {
    let chunk = decode_cohere_line(CohereStreamVersion::V2, br#"{"type":"content-delta","event":"message-end","delta":{"message":{"content":{"text":"x"}}}}"#, false).unwrap();
    assert_eq!(chunk.text, "x");
    assert!(!chunk.is_finished);
}

#[test]
fn byte_sse_prefix_is_retained_before_json_decode() {
    assert!(decode_cohere_line(CohereStreamVersion::V1, b"data: {\"text\":\"x\"}", true).is_err());
}

#[test]
fn shared_headers_keep_auth_for_empty_key() {
    let mut headers = BTreeMap::from([(String::from("Authorization"), String::from("caller"))]);
    validate_environment(&mut headers, "");
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("caller")
    );
    assert_eq!(
        headers.get("Request-Source").map(String::as_str),
        Some("unspecified:litellm")
    );
}

#[rstest]
fn replays_the_cohere_line_corpus(#[files("fuzz/corpus/cohere_line/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let _result = decode_cohere_line(CohereStreamVersion::V2, &bytes, true);
}

#[test]
fn v2_tool_delta_uses_only_first_direct_tool_and_wins_over_plan_event() {
    let chunk = decode_cohere_line(CohereStreamVersion::V2, br#"{"type":"tool-call-delta","event":"tool-plan-delta","delta":{"tool_calls":[{"id":"a","name":"first","arguments":"{}"},{"id":"b","name":"second"}]},"data":{"delta":{"message":{"tool_plan":"plan"}}}}"#, false).unwrap();
    assert_eq!(
        chunk.tool_use,
        Some(json!({"id":"a","type":"function","function":{"name":"first","arguments":"{}"}}))
    );
    assert_eq!(chunk.provider_specific_fields, None);
}

#[test]
fn v2_event_plan_and_citation_defaults_are_provider_fields() {
    let plan = decode_cohere_line(
        CohereStreamVersion::V2,
        br#"{"event":"tool-plan-delta","data":{"delta":{"message":{"tool_plan":"use search"}}}}"#,
        false,
    )
    .unwrap();
    assert_eq!(
        plan.provider_specific_fields,
        Some(json!({"tool_plan":"use search"}))
    );
    let citation = decode_cohere_line(
        CohereStreamVersion::V2,
        br#"{"event":"citation-start","data":{"delta":{"message":{"citations":{"text":"ref"}}}}}"#,
        false,
    )
    .unwrap();
    assert_eq!(
        citation.provider_specific_fields,
        Some(
            json!({"citations":[{"start":0,"end":0,"text":"ref","sources":[],"type":"TEXT_CONTENT"}]})
        )
    );
}

#[test]
fn v2_message_end_uses_token_usage_and_top_level_empty_citations_override() {
    let chunk = decode_cohere_line(CohereStreamVersion::V2, br#"{"event":"message-end","citations":[],"data":{"delta":{"usage":{"tokens":{"input_tokens":3,"output_tokens":4}}}}}"#, false).unwrap();
    assert!(chunk.is_finished);
    assert_eq!(chunk.finish_reason, "stop");
    assert_eq!(
        chunk.usage.as_ref().map(|usage| usage.total_tokens),
        Some(7)
    );
    assert_eq!(
        chunk.provider_specific_fields,
        Some(json!({"citations":[]}))
    );
}

#[test]
fn v2_empty_usage_does_not_synthesize_usage_and_unknown_event_stays_unfinished() {
    let empty_usage = decode_cohere_line(
        CohereStreamVersion::V2,
        br#"{"event":"message-end","data":{"delta":{"usage":{}}}}"#,
        false,
    )
    .unwrap();
    assert_eq!(empty_usage.usage, None);
    let unknown =
        decode_cohere_line(CohereStreamVersion::V2, br#"{"event":"unknown"}"#, false).unwrap();
    assert!(!unknown.is_finished);
    assert_eq!(unknown.text, "");
}

#[test]
fn v2_string_content_and_invalid_utf8_are_handled() {
    let content = decode_cohere_line(
        CohereStreamVersion::V2,
        br#"{"type":"content-delta","delta":{"message":{"content":"plain"}}}"#,
        false,
    )
    .unwrap();
    assert_eq!(content.text, "plain");
    assert!(decode_cohere_line(CohereStreamVersion::V2, &[0xff], true).is_err());
}

#[test]
fn iterator_item_keeps_eof_and_distinguishes_receive_from_parse_errors() {
    assert!(
        decode_cohere_item(CohereStreamVersion::V1, Ok(None)).is_ok_and(|chunk| chunk.is_none())
    );
    let receive_error = decode_cohere_item(
        CohereStreamVersion::V1,
        Err(std::io::Error::other("transport")),
    )
    .unwrap_err();
    assert!(matches!(
        receive_error,
        CohereIteratorError::Receiving { .. }
    ));
    let parse_error =
        decode_cohere_item(CohereStreamVersion::V1, Ok(Some(b"not-json".to_vec()))).unwrap_err();
    assert!(matches!(parse_error, CohereIteratorError::Parsing { .. }));
    assert!(parse_error.to_string().starts_with("Error parsing chunk:"));
}
