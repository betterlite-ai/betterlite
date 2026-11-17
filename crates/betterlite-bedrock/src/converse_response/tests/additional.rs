#![cfg(test)]

use betterlite_models::decisions::Json;

use crate::converse_response::message::MessageError;
use crate::converse_response::{ReplyFieldError, transform::TransformError};
use crate::converse_usage::reasoning::ThinkingBlock;

use super::support::{convert, fixture, json, rendered, supplied, with_tier};

pub(super) fn metrics_headers() {
    metrics_and_headers_stay_out_of_the_response();
    top_level_extras_stay_out_of_the_response();
    a_body_without_a_message_keeps_headers_off_the_failure();
}

fn metrics_and_headers_stay_out_of_the_response() {
    let mut fixture = fixture();
    fixture.headers = vec![
        (String::from("x-amzn-requestid"), String::from("req-7")),
        (
            String::from("x-amzn-bedrock-invocation-latency"),
            String::from("250"),
        ),
    ];
    fixture.supplied = with_tier(
        supplied("chatcmpl-supplied", Some("fp-1")),
        r#""priority""#,
        r#""kept""#,
    );
    let body = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "done"}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 10, "outputTokens": 5, "totalTokens": 15}, "metrics": {"latencyMs": 250}}"#;
    let run = convert(&fixture, body);
    let Json::Object(fields) = rendered(run.response()) else {
        panic!("the envelope is an object");
    };
    assert!(!fields.contains_key("metrics"));
    assert!(!fields.contains_key("headers"));
    assert_eq!(run.response().service_tier, Some(json(r#""priority""#)));
    assert_eq!(run.response().trace, Some(json(r#""kept""#)));
    assert_eq!(run.response().system_fingerprint.as_deref(), Some("fp-1"));
    assert_eq!(run.message().content.as_deref(), Some("done"));
    assert_eq!(run.message().provider_specific_fields, None);
    assert_eq!(
        run.detail("original_response"),
        &Json::Text(body.to_owned())
    );
}

fn top_level_extras_stay_out_of_the_response() {
    let body = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "done"}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 2, "totalTokens": 3}, "customField": {"any": ["value", 7, null]}, "requestMetadata": {"team": "a"}}"#;
    let run = convert(&fixture(), body);
    let Json::Object(fields) = rendered(run.response()) else {
        panic!("the envelope is an object");
    };
    assert!(!fields.contains_key("customField"));
    assert!(!fields.contains_key("requestMetadata"));
    assert_eq!(run.message().content.as_deref(), Some("done"));
    assert_eq!(run.message().provider_specific_fields, None);
    assert_eq!(
        run.response()
            .usage
            .as_ref()
            .expect("the usage")
            .total_tokens,
        3
    );
}

fn a_body_without_a_message_keeps_headers_off_the_failure() {
    let mut fixture = fixture();
    fixture.headers = vec![
        (String::from("x-amzn-requestid"), String::from("req-9")),
        (String::from("Retry-After"), String::from("3")),
    ];
    let body = r#"{"metrics": {"latencyMs": 12}}"#;
    let run = convert(&fixture, body);
    assert!(matches!(
        run.failure(),
        TransformError::Message(MessageError::Reply(ReplyFieldError::Missing {
            path: "output"
        }))
    ));
    assert_eq!(run.status(), None);
    assert_eq!(
        run.detail("original_response"),
        &Json::Text(body.to_owned())
    );
}

pub(super) fn additional_model_fields() {
    thinking_tokens_come_from_the_additional_fields();
    tier_and_trace_are_read_when_the_members_exist();
    a_null_or_empty_service_tier_replaces_what_it_names();
    a_non_object_service_tier_leaves_the_existing_value();
    an_empty_service_tier_type_is_kept();
    reasoning_blocks_project_content_and_thinking();
}

const REASONING_TOKEN_CASES: [(&str, i64, i64); 4] = [
    (
        r#"{"usage": {"output_tokens_details": {"thinking_tokens": 3}}}"#,
        3,
        2,
    ),
    (
        r#"{"usage": {"output_tokens_details": {"thinking_tokens": 9}}, "passthrough": {"kept": false}}"#,
        5,
        0,
    ),
    (r#"{"usage": {"output_tokens_details": {}}}"#, 0, 5),
    (r#""not a mapping""#, 0, 5),
];

fn thinking_tokens_come_from_the_additional_fields() {
    for (fields, reasoning, text) in REASONING_TOKEN_CASES {
        let body = format!(
            r#"{{"output": {{"message": {{"content": [{{"text": "done"}}]}}}}, "stopReason": "end_turn", "usage": {{"inputTokens": 10, "outputTokens": 5, "totalTokens": 15}}, "additionalModelResponseFields": {fields}}}"#
        );
        let run = convert(&fixture(), &body);
        let usage = run.response().usage.as_ref().expect("the usage");
        assert_eq!(
            usage.completion_tokens_details.reasoning_tokens,
            Some(reasoning),
            "fields {fields}"
        );
        assert_eq!(
            usage.completion_tokens_details.text_tokens,
            Some(text),
            "fields {fields}"
        );
        assert_eq!(usage.total_tokens, 15);
        assert_eq!(run.message().provider_specific_fields, None);
    }
}

fn tier_and_trace_are_read_when_the_members_exist() {
    let body = r#"{"output": {"message": {"content": [{"text": "done"}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 10, "outputTokens": 5, "totalTokens": 15}, "trace": {"guardrail": {"action": "NONE"}}, "serviceTier": {"type": "priority"}}"#;
    let run = convert(&fixture(), body);
    assert_eq!(
        run.response().trace,
        Some(json(r#"{"guardrail": {"action": "NONE"}}"#))
    );
    assert_eq!(run.response().service_tier, Some(json(r#""priority""#)));
}

fn a_null_or_empty_service_tier_replaces_what_it_names() {
    let kept = with_tier(
        supplied("chatcmpl-supplied", None),
        r#""priority""#,
        r#"{"guardrail": "kept"}"#,
    );
    let body = r#"{"output": {"message": {"content": [{"text": "done"}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 10, "outputTokens": 5, "totalTokens": 15}, "trace": null, "serviceTier": {"type": null}}"#;
    let mut null_tier = fixture();
    null_tier.supplied = kept;
    let run = convert(&null_tier, body);
    assert_eq!(run.response().trace, Some(Json::Null));
    assert_eq!(run.response().service_tier, Some(Json::Null));

    let empty_body = r#"{"output": {"message": {"content": [{"text": "done"}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 10, "outputTokens": 5, "totalTokens": 15}, "serviceTier": {}}"#;
    let mut empty_tier = fixture();
    empty_tier.supplied = with_tier(
        supplied("chatcmpl-supplied", None),
        r#""priority""#,
        r#"{"guardrail": "kept"}"#,
    );
    let empty_run = convert(&empty_tier, empty_body);
    assert_eq!(
        empty_run.response().service_tier,
        Some(json(r#""priority""#))
    );
}

fn a_non_object_service_tier_leaves_the_existing_value() {
    let body = r#"{"output": {"message": {"content": [{"text": "done"}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 10, "outputTokens": 5, "totalTokens": 15}, "serviceTier": "priority"}"#;
    let mut fixture = fixture();
    fixture.supplied = with_tier(
        supplied("chatcmpl-supplied", None),
        r#""priority""#,
        r#"{"guardrail": "kept"}"#,
    );
    let run = convert(&fixture, body);
    assert_eq!(run.response().service_tier, Some(json(r#""priority""#)));
    assert_eq!(run.response().trace, Some(json(r#"{"guardrail": "kept"}"#)));
}

fn an_empty_service_tier_type_is_kept() {
    let body = r#"{"output": {"message": {"content": [{"text": "done"}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 10, "outputTokens": 5, "totalTokens": 15}, "serviceTier": {"type": ""}}"#;
    let run = convert(&fixture(), body);
    assert_eq!(run.response().service_tier, Some(json(r#""""#)));
}

fn reasoning_blocks_project_content_and_thinking() {
    let body = r#"{"output": {"message": {"role": "assistant", "content": [{"reasoningContent": {"reasoningText": {"text": "hello world", "signature": "sig-1"}}}, {"text": "The answer."}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 10, "outputTokens": 5, "totalTokens": 15}}"#;
    let run = convert(&fixture(), body);
    assert_eq!(run.message().content.as_deref(), Some("The answer."));
    assert_eq!(
        run.message().reasoning_content.as_deref(),
        Some("hello world")
    );
    assert_eq!(
        run.message().thinking_blocks,
        Some(vec![ThinkingBlock::Thinking {
            thinking: Some(String::from("hello world")),
            signature: Some(String::from("sig-1")),
        }])
    );
    let blocks = run
        .message()
        .provider_specific_fields
        .as_ref()
        .expect("the raw blocks");
    assert_eq!(
        blocks.get("reasoningContentBlocks"),
        Some(&json(
            r#"[{"reasoningText": {"text": "hello world", "signature": "sig-1"}}]"#
        ))
    );
    let usage = run.response().usage.as_ref().expect("the usage");
    assert_eq!(usage.completion_tokens_details.reasoning_tokens, Some(2));
    assert_eq!(usage.completion_tokens_details.text_tokens, Some(3));
}
