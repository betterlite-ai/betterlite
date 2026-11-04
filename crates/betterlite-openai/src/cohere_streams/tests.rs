#![cfg(test)]

use std::{collections::BTreeMap, error::Error, fs, io::Error as IoError, path::PathBuf};

use rstest::rstest;
use secrecy::SecretString;
use serde_json::Value;

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
        chunk
            .provider_specific_fields
            .map(|fields| serde_json::to_value(fields).unwrap()),
        Some(serde_json::from_str::<Value>(r#"{"citations":[]}"#).unwrap())
    );
}

#[test]
fn v1_finish_record_without_text_uses_finish_reason() {
    let chunk = decode_cohere_line(
        CohereStreamVersion::V1,
        br#"{"is_finished":true,"finish_reason":"COMPLETE"}"#,
        false,
    )
    .unwrap();
    assert!(chunk.is_finished);
    assert_eq!(chunk.finish_reason, "COMPLETE");
}

#[test]
fn v2_type_takes_precedence_over_message_end() {
    let chunk = decode_cohere_line(CohereStreamVersion::V2, br#"{"type":"content-delta","event":"message-end","delta":{"message":{"content":{"text":"x"}}}}"#, false).unwrap();
    assert_eq!(chunk.text, "x");
    assert!(!chunk.is_finished);
}

#[test]
fn byte_sse_prefix_is_retained_before_json_decode() {
    let _error =
        decode_cohere_line(CohereStreamVersion::V1, b"data: {\"text\":\"x\"}", true).unwrap_err();
}

#[test]
fn shared_headers_keep_auth_for_empty_key() {
    let mut headers = BTreeMap::from([(String::from("Authorization"), String::from("caller"))]);
    validate_environment(&mut headers, &SecretString::from(String::new()));
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
#[case::v1(CohereStreamVersion::V1)]
#[case::v2(CohereStreamVersion::V2)]
fn replays_the_cohere_line_corpus(
    #[case] version: CohereStreamVersion,
    #[files("fuzz/corpus/cohere_line/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let _result = decode_cohere_line(version, &bytes, true);
}

#[test]
fn v2_tool_delta_uses_only_first_direct_tool_and_wins_over_plan_event() {
    let chunk = decode_cohere_line(CohereStreamVersion::V2, br#"{"type":"tool-call-delta","event":"tool-plan-delta","delta":{"tool_calls":[{"id":"a","name":"first","arguments":"{}"},{"id":"b","name":"second"}]},"data":{"delta":{"message":{"tool_plan":"plan"}}}}"#, false).unwrap();
    assert_eq!(
        chunk
            .tool_use
            .map(|tool| serde_json::to_value(tool).unwrap()),
        Some(
            serde_json::from_str::<Value>(
                r#"{"id":"a","type":"function","function":{"name":"first","arguments":"{}"}}"#
            )
            .unwrap()
        )
    );
    assert_eq!(
        chunk
            .provider_specific_fields
            .map(|fields| serde_json::to_value(fields).unwrap()),
        None
    );
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
        plan.provider_specific_fields
            .map(|fields| serde_json::to_value(fields).unwrap()),
        Some(serde_json::from_str::<Value>(r#"{"tool_plan":"use search"}"#).unwrap())
    );
    let citation = decode_cohere_line(
        CohereStreamVersion::V2,
        br#"{"event":"citation-start","data":{"delta":{"message":{"citations":{"text":"ref"}}}}}"#,
        false,
    )
    .unwrap();
    assert_eq!(
        citation.provider_specific_fields.map(|fields| serde_json::to_value(fields).unwrap()),
        Some(
            serde_json::from_str::<Value>(r#"{"citations":[{"start":0,"end":0,"text":"ref","sources":[],"type":"TEXT_CONTENT"}]}"#).unwrap()
        )
    );
}

#[test]
fn v2_empty_tool_plan_and_citation_object_are_not_emitted() {
    let plan = decode_cohere_line(
        CohereStreamVersion::V2,
        br#"{"event":"tool-plan-delta","data":{"delta":{"message":{"tool_plan":""}}}}"#,
        false,
    )
    .unwrap();
    assert_eq!(
        plan.provider_specific_fields
            .map(|fields| serde_json::to_value(fields).unwrap()),
        None
    );
    let citation = decode_cohere_line(
        CohereStreamVersion::V2,
        br#"{"event":"citation-start","data":{"delta":{"message":{"citations":{}}}}}"#,
        false,
    )
    .unwrap();
    assert_eq!(
        citation
            .provider_specific_fields
            .map(|fields| serde_json::to_value(fields).unwrap()),
        None
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
        chunk
            .provider_specific_fields
            .map(|fields| serde_json::to_value(fields).unwrap()),
        Some(serde_json::from_str::<Value>(r#"{"citations":[]}"#).unwrap())
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
fn v2_null_usage_defaults_and_non_object_usage_fails() {
    let null_usage = decode_cohere_line(
        CohereStreamVersion::V2,
        br#"{"event":"message-end","data":{"delta":{"usage":null}}}"#,
        false,
    )
    .unwrap();
    assert_eq!(null_usage.usage, None);
    let zero_usage = decode_cohere_line(
        CohereStreamVersion::V2,
        br#"{"event":"message-end","data":{"delta":{"usage":0}}}"#,
        false,
    );
    assert!(matches!(
        zero_usage,
        Err(super::CohereStreamError::V2Transform { .. })
    ));
    let invalid_usage = decode_cohere_line(
        CohereStreamVersion::V2,
        br#"{"event":"message-end","data":{"delta":{"usage":"truthy"}}}"#,
        false,
    );
    assert!(matches!(
        invalid_usage,
        Err(super::CohereStreamError::V2Transform { .. })
    ));
    let numeric_usage = decode_cohere_line(
        CohereStreamVersion::V2,
        br#"{"event":"message-end","data":{"delta":{"usage":1}}}"#,
        false,
    );
    assert!(matches!(
        numeric_usage,
        Err(super::CohereStreamError::V2Transform { .. })
    ));
}

#[rstest]
#[case::billed(
    r#"{"event":"message-end","data":{"delta":{"usage":{"billed_units":{"input_tokens":99}}}}}"#
)]
#[case::extension(r#"{"event":"message-end","data":{"delta":{"usage":{"extension":false}}}}"#)]
fn v2_nonempty_usage_without_tokens_emits_zero_counts(#[case] record: &str) {
    let chunk = decode_cohere_line(CohereStreamVersion::V2, record.as_bytes(), false).unwrap();
    assert_eq!(
        chunk.usage,
        Some(super::CohereUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
        })
    );
}

#[rstest]
#[case::bytes(CohereStreamVersion::V1, true)]
#[case::text(CohereStreamVersion::V1, false)]
#[case::v2_bytes(CohereStreamVersion::V2, true)]
#[case::v2_text(CohereStreamVersion::V2, false)]
fn decoder_validates_input_kind(#[case] version: CohereStreamVersion, #[case] bytes: bool) {
    let result = decode_cohere_line(version, br#"{"text":"data: inside text"}"#, bytes);
    assert_eq!(result.is_err(), bytes);
}

#[test]
fn shared_headers_overwrite_defaults_and_nonempty_auth() {
    let mut headers = BTreeMap::from([
        (String::from("Request-Source"), String::from("caller")),
        (String::from("accept"), String::from("caller")),
        (String::from("content-type"), String::from("caller")),
        (String::from("Authorization"), String::from("caller")),
        (String::from("X-Custom"), String::from("kept")),
    ]);
    validate_environment(&mut headers, &SecretString::from(String::from("key")));
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer key")
    );
    assert_eq!(
        headers.get("Request-Source").map(String::as_str),
        Some("unspecified:litellm")
    );
    assert_eq!(
        headers.get("accept").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(headers.get("X-Custom").map(String::as_str), Some("kept"));
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
    let _error = decode_cohere_line(CohereStreamVersion::V2, &[0xff], true).unwrap_err();
}

#[test]
fn iterator_item_keeps_eof_and_distinguishes_receive_from_parse_errors() {
    assert!(
        decode_cohere_item(CohereStreamVersion::V1, Ok(None)).is_ok_and(|chunk| chunk.is_none())
    );
    let receive_error =
        decode_cohere_item(CohereStreamVersion::V1, Err(IoError::other("transport"))).unwrap_err();
    assert!(matches!(
        receive_error,
        CohereIteratorError::Receiving { .. }
    ));
    let parse_error =
        decode_cohere_item(CohereStreamVersion::V1, Ok(Some(b"not-json".to_vec()))).unwrap_err();
    assert!(matches!(parse_error, CohereIteratorError::Parsing { .. }));
    assert!(parse_error.to_string().starts_with("Error parsing chunk:"));
}

#[test]
fn cohere_error_keeps_status_message_and_base_request() {
    let error = super::cohere_error(429, String::from("limited"));
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "limited");
    assert_eq!(error.request.method, "POST");
    assert_eq!(error.request.url, "https://docs.litellm.ai/docs");
    assert_eq!(error.response.status_code, 429);
    assert_eq!(error.response.request, error.request);
    assert!(!error.status_code_is_synthesized);
    assert!(error.headers.is_none());
    assert!(error.body.is_none());
}

#[rstest]
#[case::integer(r#"{"index":-3}"#, -3)]
#[case::text(r#"{"index":" +42 "}"#, 42)]
#[case::missing("{}", 0)]
#[case::fraction(r#"{"index":-3.9}"#, -3)]
#[case::boolean(r#"{"index":true}"#, 1)]
#[case::separators(r#"{"index":"1_002"}"#, 1002)]
fn stream_index_conversion(#[case] record: &str, #[case] expected: i64) {
    assert_eq!(
        decode_cohere_line(CohereStreamVersion::V1, record.as_bytes(), false)
            .unwrap()
            .index,
        expected
    );
    assert_eq!(
        decode_cohere_line(CohereStreamVersion::V2, record.as_bytes(), false)
            .unwrap()
            .index,
        expected
    );
}

#[rstest]
#[case::invalid_index(r#"{"index":"oops"}"#)]
#[case::wrong_delta(r#"{"type":"content-delta","delta":2}"#)]
#[case::bad_tokens(r#"{"event":"message-end","data":{"delta":{"usage":{"tokens":2}}}}"#)]
#[case::overflow(r#"{"event":"message-end","data":{"delta":{"usage":{"tokens":{"input_tokens":9223372036854775807,"output_tokens":1}}}}}"#)]
fn v2_wraps_transform_errors_and_preserves_cause(#[case] record: &str) {
    let error = decode_cohere_line(CohereStreamVersion::V2, record.as_bytes(), false).unwrap_err();
    assert!(matches!(
        error,
        super::CohereStreamError::V2Transform { .. }
    ));
    assert!(Error::source(&error).is_some());
}

#[test]
fn v1_preserves_non_value_transformation_errors() {
    let missing = decode_cohere_item(
        CohereStreamVersion::V1,
        Ok(Some(br#"{"is_finished":true}"#.to_vec())),
    )
    .unwrap_err();
    assert!(matches!(
        missing,
        CohereIteratorError::Transformation { .. }
    ));
    let wrong_type =
        decode_cohere_item(CohereStreamVersion::V1, Ok(Some(br#"{"text":5}"#.to_vec())))
            .unwrap_err();
    assert!(matches!(
        wrong_type,
        CohereIteratorError::Transformation { .. }
    ));
    let invalid_index = decode_cohere_item(
        CohereStreamVersion::V1,
        Ok(Some(br#"{"index":"bad"}"#.to_vec())),
    )
    .unwrap_err();
    assert!(matches!(invalid_index, CohereIteratorError::Parsing { .. }));
    let valid = decode_cohere_item(
        CohereStreamVersion::V1,
        Ok(Some(br#"{"text":"" ,"is_finished":true}"#.to_vec())),
    )
    .unwrap()
    .unwrap();
    assert!(!valid.is_finished);
    assert!(valid.tool_use.is_none());
    assert!(valid.usage.is_none());
    assert_eq!(valid.index, 0);
    assert_eq!(valid.finish_reason, "");
}

#[rstest]
#[case::start(
    r#"{"start":5}"#,
    r#"{"start":5,"end":0,"text":"","sources":[],"type":"TEXT_CONTENT"}"#
)]
#[case::end(
    r#"{"end":8}"#,
    r#"{"start":0,"end":8,"text":"","sources":[],"type":"TEXT_CONTENT"}"#
)]
#[case::sources(
    r#"{"sources":[{"id":"one"}]}"#,
    r#"{"start":0,"end":0,"text":"","sources":[{"id":"one"}],"type":"TEXT_CONTENT"}"#
)]
#[case::kind(
    r#"{"type":"CUSTOM"}"#,
    r#"{"start":0,"end":0,"text":"","sources":[],"type":"CUSTOM"}"#
)]
fn citation_preserves_each_explicit_field(#[case] citation: &str, #[case] expected: &str) {
    let record = format!(
        r#"{{"event":"citation-start","data":{{"delta":{{"message":{{"citations":{citation}}}}}}}}}"#
    );
    let chunk = decode_cohere_line(CohereStreamVersion::V2, record.as_bytes(), false).unwrap();
    let fields = serde_json::to_value(chunk.provider_specific_fields.unwrap()).unwrap();
    assert_eq!(
        fields,
        serde_json::from_str::<Value>(&format!(r#"{{"citations":[{expected}]}}"#)).unwrap()
    );
}

#[test]
fn top_level_citations_override_event_and_preserve_plan() {
    let record = br#"{"event":"citation-start","citations":[],"data":{"delta":{"message":{"citations":{"text":"old"}}}}}"#;
    let chunk = decode_cohere_line(CohereStreamVersion::V2, record, false).unwrap();
    assert_eq!(
        chunk
            .provider_specific_fields
            .unwrap()
            .citations
            .unwrap()
            .get(),
        "[]"
    );
    let plan = decode_cohere_line(CohereStreamVersion::V2, br#"{"event":"tool-plan-delta","citations":[],"data":{"delta":{"message":{"tool_plan":"plan"}}}}"#, false).unwrap();
    let fields = plan.provider_specific_fields.unwrap();
    assert_eq!(fields.tool_plan.as_deref(), Some("plan"));
    assert_eq!(fields.citations.unwrap().get(), "[]");
}

#[rstest]
#[case::missing_delta(r#"{"type":"content-delta"}"#)]
#[case::missing_message(r#"{"type":"content-delta","delta":{}}"#)]
#[case::empty_content(r#"{"type":"content-delta","delta":{"message":{"content":{}}}}"#)]
fn empty_content_keeps_empty_chunk(#[case] record: &str) {
    let chunk = decode_cohere_line(CohereStreamVersion::V2, record.as_bytes(), false).unwrap();
    assert_eq!(chunk.text, "");
    assert!(!chunk.is_finished);
}

#[test]
fn empty_tools_and_default_fields() {
    let empty = decode_cohere_line(
        CohereStreamVersion::V2,
        br#"{"type":"tool-call-delta","delta":{"tool_calls":[]}}"#,
        false,
    )
    .unwrap();
    assert!(empty.tool_use.is_none());
    let defaults = decode_cohere_line(
        CohereStreamVersion::V2,
        br#"{"type":"tool-call-delta","delta":{"tool_calls":[{}]}}"#,
        false,
    )
    .unwrap();
    let tool = defaults.tool_use.unwrap();
    assert_eq!(tool.id, "");
    assert_eq!(tool.kind, "function");
    assert_eq!(tool.function.name, "");
    assert_eq!(tool.function.arguments, "");
}

#[test]
fn decoder_enforces_line_bound_before_parsing() {
    let too_large = vec![b' '; 0x0010_0001];
    assert!(matches!(
        decode_cohere_line(CohereStreamVersion::V1, &too_large, false),
        Err(super::CohereStreamError::TooLarge)
    ));
    let mut maximum = b"{}".to_vec();
    maximum.resize(0x0010_0000, b' ');
    let _chunk = decode_cohere_line(CohereStreamVersion::V1, &maximum, false).unwrap();
}

#[test]
fn v2_explicit_finish_and_token_counts_ignore_billed_units() {
    let chunk = decode_cohere_line(CohereStreamVersion::V2, br#"{"event":"message-end","data":{"delta":{"finish_reason":"MAX_TOKENS","usage":{"tokens":{"input_tokens":2,"output_tokens":5},"billed_units":{"input_tokens":100}}}}}"#, false).unwrap();
    assert_eq!(chunk.finish_reason, "MAX_TOKENS");
    assert_eq!(
        chunk.usage,
        Some(super::CohereUsage {
            prompt_tokens: 2,
            completion_tokens: 5,
            total_tokens: 7
        })
    );
    let zero = decode_cohere_line(
        CohereStreamVersion::V2,
        br#"{"event":"message-end","data":{"delta":{"usage":{"tokens":{}}}}}"#,
        false,
    )
    .unwrap();
    assert_eq!(
        zero.usage,
        Some(super::CohereUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0
        })
    );
}

#[rstest]
#[case::empty(r#"{"index":""}"#)]
#[case::start(r#"{"index":"_1"}"#)]
#[case::end(r#"{"index":"1_"}"#)]
#[case::adjacent(r#"{"index":"1__2"}"#)]
#[case::sign(r#"{"index":"+_2"}"#)]
fn invalid_index_separators_are_errors(#[case] record: &str) {
    let error = decode_cohere_line(CohereStreamVersion::V1, record.as_bytes(), false).unwrap_err();
    assert!(matches!(error, super::CohereStreamError::IndexSeparator));
}

#[rstest]
#[case::v1(CohereStreamVersion::V1)]
#[case::v2(CohereStreamVersion::V2)]
fn syntax_errors_stay_outside_transform_errors(#[case] version: CohereStreamVersion) {
    let json_error = decode_cohere_line(version, b"{", false).unwrap_err();
    assert!(matches!(json_error, super::CohereStreamError::Json(_)));
    let utf8_error = decode_cohere_line(version, &[0xff], true).unwrap_err();
    assert!(matches!(utf8_error, super::CohereStreamError::Utf8(_)));
    let item_error = decode_cohere_item(version, Ok(Some(vec![0xff]))).unwrap_err();
    assert!(matches!(item_error, CohereIteratorError::Parsing { .. }));
    assert!(Error::source(&item_error).is_some());
}

#[test]
fn v1_false_finish_and_unknown_record_remain_empty() {
    let chunk = decode_cohere_line(
        CohereStreamVersion::V1,
        br#"{"is_finished":false,"finish_reason":"ignored","usage":{"tokens":{}}}"#,
        false,
    )
    .unwrap();
    assert!(!chunk.is_finished);
    assert_eq!(chunk.finish_reason, "");
    assert!(chunk.usage.is_none());
    assert!(chunk.tool_use.is_none());
}
