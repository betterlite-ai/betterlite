#![cfg(test)]

use std::future::Future;

use betterlite_models::decisions::Json;
use betterlite_openai::chat::response::{ChatResponse, ChatUsage};
use betterlite_tokenizer::count::request::Message;
use rstest::rstest;
use tokio::runtime::Builder;

use crate::agentcore_response::error::AgentCoreResponseError;
use crate::agentcore_response::json::parse_json_reply;
use crate::agentcore_response::reply::decode_agentcore_reply;
use crate::agentcore_response::sse::parse_sse_stream;
use crate::agentcore_response::tests::support::{
    FixtureCounter, capture, choice_message, json_response, messages, response, sse_response,
};
use crate::agentcore_response::transform::{RawResponse, transform_agentcore_response};
use crate::bedrock_settings::ErrorHeaders;

#[rstest]
#[case::json_content_message(json_content_message)]
#[case::sse_content_delta(sse_content_delta)]
#[case::sse_usage_event(sse_usage_event)]
#[case::plain_text_response(plain_text_response)]
#[case::missing_usage(missing_usage)]
#[case::malformed_event_fallback(malformed_event_fallback)]
#[case::a2a_json_reply(a2a_json_reply)]
#[case::json_raw_fallback(json_raw_fallback)]
#[case::sse_final_message(sse_final_message)]
#[case::agentcore_response_error(agentcore_response_error)]
fn agentcore_response_cases(#[case] case: fn()) {
    case();
}

fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn usage(prompt: u64, completion: u64, total: u64) -> ChatUsage {
    ChatUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
        prompt_tokens_details: None,
        completion_tokens_details: None,
    }
}

fn content_of(text: &str) -> String {
    parse_json_reply(&json(text)).unwrap().content
}

fn run(
    raw: &RawResponse,
    request_messages: &[Message],
    content: &str,
    counts: (Option<usize>, Option<usize>),
) -> ChatResponse {
    let mut response = response();
    let counter = FixtureCounter::new("gpt-4", request_messages, content, counts.0, counts.1);
    block_on(transform_agentcore_response(
        "gpt-4",
        raw,
        request_messages,
        &mut response,
        &counter,
    ))
    .unwrap();
    response
}

fn json_content_message() {
    let a2a = r#"{"jsonrpc":"2.0","result":{"parts":[{"kind":"text","text":"direct"}]}}"#;
    assert_eq!(content_of(a2a), "direct");
    let null_jsonrpc = r#"{"jsonrpc":null,"result":{"parts":[{"kind":"text","text":"still"}]}}"#;
    assert_eq!(content_of(null_jsonrpc), "still");
    let without_jsonrpc = r#"{"result":{"parts":[{"kind":"text","text":"x"}]}}"#;
    assert_eq!(content_of(without_jsonrpc), "");
    let falling_through = r#"{"jsonrpc":"2.0","result":{"content":[{"text":"fall"}]}}"#;
    assert_eq!(content_of(falling_through), "fall");
    let empty_result = r#"{"result":{},"response":"ignored"}"#;
    let reply = parse_json_reply(&json(empty_result)).unwrap();
    assert_eq!(reply.content, "");
    assert!(reply.final_message.is_some());
    assert_eq!(
        content_of(r#"{"result":{"content":[{"text":"a"},{"text":"b"}]}}"#),
        "ab"
    );
    assert_eq!(
        content_of(r#"{"response":[{"text":"x"},{"text":"y"}]}"#),
        "xy"
    );
    assert_eq!(content_of(r#"{"result":"","response":"later"}"#), "");
    assert_eq!(
        content_of(r#"{"result":"s","response":[{"text":"list"}]}"#),
        "list"
    );
    assert_eq!(content_of(r#"{"response":"only"}"#), "only");
    let choices_only = r#"{"choices":[{"index":0,"message":{"role":"assistant"}}]}"#;
    assert_eq!(
        content_of(choices_only),
        r#"{"choices": [{"index": 0, "message": {"role": "assistant"}}]}"#
    );
}

fn sse_content_delta() {
    let body = concat!(
        "  data: {\"event\":{\"contentBlockDelta\":{\"delta\":{\"text\":\"a\"}}}}  \n",
        "\n",
        "  data: {\"event\":{\"contentBlockDelta\":{\"delta\":{\"text\":\"c\"}}}}\n",
        "data: {\"event\":{\"contentBlockDelta\":{\"delta\":{\"text\":\"b\"}}}}\n",
        "data: 42\n",
        "data: {}\n",
        "data: {\"event\":{}}\n",
        "data: {\"event\":5}\n",
        "data: {\"message\":5}\n",
        "data: {\"event\":{\"contentBlockDelta\":{}}}\n",
        "data: {\"event\":{\"contentBlockDelta\":{\"delta\":{}}}}\n",
        "data: {\"event\":{\"contentBlockDelta\":{\"delta\":{\"text\":\"\"}}}}\n",
        "data: {\"event\":{\"contentBlockDelta\":{\"delta\":{\"text\":\"last\"}}}}",
    );
    assert_eq!(parse_sse_stream(body).unwrap().content, "acblast");
    let broken_delta =
        parse_sse_stream(r#"data: {"event":{"contentBlockDelta":true}}"#).unwrap_err();
    assert!(matches!(
        broken_delta,
        AgentCoreResponseError::DeltaEventNotObject
    ));
    let broken_text =
        parse_sse_stream(r#"data: {"event":{"contentBlockDelta":{"delta":{"text":7}}}}"#)
            .unwrap_err();
    assert!(matches!(
        broken_text,
        AgentCoreResponseError::DeltaTextNotString
    ));
}

fn sse_usage_event() {
    let body = "data: {\"event\":{\"metadata\":{\"usage\":{\"inputTokens\":1,\"outputTokens\":2,\"totalTokens\":3}}}}\n\
                data: {\"event\":{\"metadata\":{\"usage\":null}}}\n\
                data: {\"event\":{\"metadata\":{\"usage\":{}}}}\n\
                data: {\"event\":{\"metadata\":{}}}\n\
                data: {\"event\":{\"metadata\":{\"usage\":{\"outputTokens\":9}}}}\n\
                data: {\"event\":{\"metadata\":{\"usage\":{\"inputTokens\":0}}}}";
    let reply = parse_sse_stream(body).unwrap();
    let found = reply.usage.unwrap();
    assert_eq!(found.input_tokens, 0);
    assert_eq!(found.output_tokens, 0);
    assert_eq!(found.total_tokens, 0);
    let negative = parse_sse_stream(r#"data: {"event":{"metadata":{"usage":{"inputTokens":-1}}}}"#)
        .unwrap_err();
    assert!(matches!(
        negative,
        AgentCoreResponseError::UsageFieldNegative {
            field: "inputTokens"
        }
    ));
    let last_wins = run(&sse_response(body), &[], "hello", (Some(1), Some(2)));
    assert_eq!(last_wins.usage, usage(0, 0, 0));
    let total_only = "data: {\"event\":{\"metadata\":{\"usage\":{\"totalTokens\":5}}}}";
    let independent = run(&sse_response(total_only), &[], "hello", (Some(1), Some(2)));
    assert_eq!(independent.usage, usage(0, 0, 5));
}

fn plain_text_response() {
    let body = br#"{"result":{"content":[{"text":"j"}]}}"#;
    let via_json = decode_agentcore_reply(body, Some("APPLICATION/JSON; charset=utf-8")).unwrap();
    assert_eq!(via_json.content, "j");
    let via_plain = decode_agentcore_reply(body, Some("text/plain")).unwrap();
    assert_eq!(via_plain.content, "");
    let without_header = decode_agentcore_reply(body, None).unwrap();
    assert_eq!(without_header.content, "");
    let lines = b"data: {\"event\":{\"contentBlockDelta\":{\"delta\":{\"text\":\"s\"}}}}";
    let via_sse = decode_agentcore_reply(lines, Some("text/event-stream")).unwrap();
    assert_eq!(via_sse.content, "s");
    let raw = RawResponse {
        status_code: 200,
        headers: vec![("Content-Type".to_owned(), "Application/JSON".to_owned())],
        body: br#"{"result":{"content":[{"text":"named"}]}}"#.to_vec(),
    };
    assert_eq!(transformed_content(&raw, "named"), Some("named".to_owned()));
    let repeated = RawResponse {
        status_code: 200,
        headers: vec![
            ("content-type".to_owned(), "text/plain".to_owned()),
            ("content-type".to_owned(), "application/json".to_owned()),
        ],
        body: br#"{"result":{"content":[{"text":"joined"}]}}"#.to_vec(),
    };
    assert_eq!(
        transformed_content(&repeated, "joined"),
        Some("joined".to_owned())
    );
}

fn transformed_content(raw: &RawResponse, content: &str) -> Option<String> {
    let completed = run(raw, &[], content, (Some(1), Some(1)));
    choice_message(&completed).3
}

fn missing_usage() {
    let (logs, _guard) = capture();
    let raw = json_response(r#"{"result":{"content":[{"text":"hello"}]}}"#);
    let request_messages = messages();
    let calculated = run(&raw, &request_messages, "hello", (Some(3), Some(5)));
    assert_eq!(calculated.usage, usage(3, 5, 8));
    assert!(
        logs.text()
            .contains("Calculated usage - prompt: 3, completion: 5, total: 8")
    );
    let prompt_failed = run(&raw, &request_messages, "hello", (None, Some(5)));
    assert_eq!(prompt_failed.usage, usage(1, 2, 3));
    assert_eq!(
        logs.text()
            .matches("Failed to calculate token usage: fixture counter unavailable")
            .count(),
        1
    );
    let completion_failed = run(&raw, &request_messages, "hello", (Some(3), None));
    assert_eq!(completion_failed.usage, usage(1, 2, 3));
    assert_eq!(
        logs.text()
            .matches("Failed to calculate token usage: fixture counter unavailable")
            .count(),
        2
    );
}

fn malformed_event_fallback() {
    let (logs, _guard) = capture();
    let body = "data: [DONE]\n\
                data: {not json}\n\
                data: 7\n\
                data:\n\
                \n\
                data: {\"event\":{\"contentBlockDelta\":{\"delta\":{\"text\":\"ok\"}}}}";
    assert_eq!(parse_sse_stream(body).unwrap().content, "ok");
    assert!(logs.text().contains("Skipping non-JSON line"));
    let mut completed = response();
    let failure = block_on(transform_agentcore_response(
        "gpt-4",
        &json_response("{\"result\":"),
        &[],
        &mut completed,
        &FixtureCounter::new("gpt-4", &[], "", None, None),
    ))
    .unwrap_err();
    let AgentCoreResponseError::Bedrock {
        status,
        error: cause,
    } = failure
    else {
        panic!("expected a bedrock error");
    };
    assert_eq!(status, 200);
    assert!(cause.cause.error.message.contains("not valid JSON"));
}

fn a2a_content(result: &str) -> String {
    let body = format!("{{\"jsonrpc\":\"2.0\",\"result\":{result}}}");
    parse_json_reply(&json(&body)).unwrap().content
}

fn a2a_json_reply() {
    assert_eq!(
        a2a_content(r#"{"parts":[{"kind":"text","text":"direct"}]}"#),
        "direct"
    );
    assert_eq!(
        a2a_content(r#"{"message":{"parts":[{"kind":"text","text":"nested"}]}}"#),
        "nested"
    );
    assert_eq!(
        a2a_content(r#"{"artifact":{"parts":[{"kind":"text","text":"single"}]}}"#),
        "single"
    );
    assert_eq!(
        a2a_content(r#"{"status":{"message":{"parts":[{"kind":"text","text":"status"}]}}}"#),
        "status"
    );
    assert_eq!(
        a2a_content(
            r#"{"artifacts":[{"parts":[{"kind":"text","text":"first"}]},{"parts":[{"kind":"text","text":"second"}]}]}"#
        ),
        "first"
    );
    let mut deep = String::from(r#"{"kind":"text","text":"deep"}"#);
    for _ in 0_usize..11 {
        deep = format!(r#"{{"parts":[{deep}]}}"#);
    }
    assert_eq!(
        a2a_content(&format!(
            r#"{{"parts":[{deep}],"content":[{{"text":"fallback"}}]}}"#
        )),
        "fallback"
    );
}

fn json_raw_fallback() {
    let (logs, _guard) = capture();
    let list = parse_json_reply(&json("[1, 2, {\"b\": 1, \"a\": 2}]")).unwrap();
    assert_eq!(list.content, "[1, 2, {\"b\": 1, \"a\": 2}]");
    assert!(logs.text().contains("JSON response is not a dict"));
    let unknown = parse_json_reply(&json(r#"{"z": 1, "a": 2}"#)).unwrap();
    assert_eq!(unknown.content, r#"{"z": 1, "a": 2}"#);
    assert!(
        logs.text()
            .contains("Could not extract content from JSON response keys")
    );
    let supplied = parse_json_reply(&json(
        r#"{"result":{"content":[{"text":"x"}]},"usage":{"inputTokens":9}}"#,
    ))
    .unwrap();
    assert!(supplied.usage.is_none());
    assert_eq!(
        content_of(r#"{"result":{"content":[{"text":"a"},5,{"other":1},{"text":"b"}]}}"#),
        "ab"
    );
    assert_eq!(content_of(r#"{"result":{"content":"text"}}"#), "");
    let block_text = parse_json_reply(&json(r#"{"result":{"content":[{"text":1}]}}"#)).unwrap_err();
    assert!(matches!(
        block_text,
        AgentCoreResponseError::ContentTextNotString { index: 0 }
    ));
    let raw = json_response(r#"{"result":{"content":[{"text":"x"}]},"usage":{"inputTokens":9}}"#);
    let calculated = run(&raw, &[], "x", (Some(4), Some(6)));
    assert_eq!(calculated.usage, usage(4, 6, 10));
}

fn sse_final_message() {
    let replaced = "data: {\"event\":{\"contentBlockDelta\":{\"delta\":{\"text\":\"delta\"}}}}\n\
                    data: {\"message\":{\"role\":\"assistant\"}}";
    assert_eq!(parse_sse_stream(replaced).unwrap().content, "");
    let falsey = "data: {\"event\":{\"contentBlockDelta\":{\"delta\":{\"text\":\"delta\"}}}}\n\
                  data: {\"message\":{}}";
    let kept = parse_sse_stream(falsey).unwrap();
    assert_eq!(kept.content, "delta");
    assert!(kept.final_message.is_some());
    let last_wins = "data: {\"message\":{\"content\":[{\"text\":\"first\"}]}}\n\
                     data: {\"message\":{\"role\":\"assistant\"}}\n\
                     data: {\"message\":{\"content\":[{\"text\":\"last\"}]}}";
    assert_eq!(parse_sse_stream(last_wins).unwrap().content, "last");
}

fn error_for_malformed_body() {
    let headers = vec![
        ("content-type".to_owned(), "application/json".to_owned()),
        ("x-request-id".to_owned(), "raw".to_owned()),
    ];
    let raw = RawResponse {
        status_code: 200,
        headers: headers.clone(),
        body: b"{".to_vec(),
    };
    let mut completed = response();
    let failure = block_on(transform_agentcore_response(
        "gpt-4",
        &raw,
        &[],
        &mut completed,
        &FixtureCounter::new("gpt-4", &[], "", Some(1), Some(1)),
    ))
    .unwrap_err();
    let AgentCoreResponseError::Bedrock {
        status,
        error: cause,
    } = failure
    else {
        panic!("expected a bedrock error");
    };
    assert_eq!(status, 200);
    assert_eq!(cause.cause.error.status_code, 200);
    assert!(!cause.cause.error.status_code_is_synthesized);
    assert_eq!(cause.cause.error.response.status_code, 200);
    assert_eq!(cause.cause.error.headers, Some(headers.clone()));
    assert_eq!(cause.cause.headers, Some(ErrorHeaders::Http(headers)));
    assert!(
        cause
            .cause
            .error
            .message
            .starts_with("Error processing response: ")
    );
    assert!(completed.choices.is_empty());
}

fn success_keeps_unrelated_fields() {
    let raw = RawResponse {
        status_code: 500,
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body: br#"{"result":{"content":[{"text":"kept"}]}}"#.to_vec(),
    };
    let completed = run(&raw, &[], "kept", (Some(2), Some(3)));
    assert_eq!(completed.id, "chatcmpl-fixture");
    assert_eq!(completed.object, "chat.completion");
    assert_eq!(completed.created, 42);
    assert_eq!(completed.system_fingerprint.as_deref(), Some("fp-fixture"));
    assert_eq!(completed.service_tier.as_deref(), Some("standard"));
    assert_eq!(completed.model, "gpt-4");
    assert_eq!(
        choice_message(&completed),
        (
            0,
            Some("stop".to_owned()),
            "assistant".to_owned(),
            Some("kept".to_owned())
        )
    );
}

fn agentcore_response_error() {
    error_for_malformed_body();
    success_keeps_unrelated_fields();
}
