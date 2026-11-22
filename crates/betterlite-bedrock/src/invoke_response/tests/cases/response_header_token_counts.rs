#![cfg(test)]

use crate::invoke_response::error::InvokeReplyError;
use crate::invoke_response::tests::counter::{CounterCall, FixtureCounter};
use crate::invoke_response::tests::support::{
    MODEL, decode, decode_with, fixture, headers, seeded, with_tool_calls,
};

pub(in crate::invoke_response::tests) fn response_header_token_counts() {
    missing_headers_fall_back_to_the_counter();
    empty_headers_fall_back_to_the_counter();
    mixed_case_headers_are_read();
    python_int_forms_are_parsed();
    invalid_headers_surface();
    negative_headers_surface();
    counter_failures_surface();
    created_and_model_are_set();
}

fn missing_headers_fall_back_to_the_counter() {
    let case = fixture();
    let run = decode(&case, r#"{"results": [{"outputText": "counted text"}]}"#);
    assert_eq!(
        run.calls,
        vec![
            CounterCall::Messages,
            CounterCall::Text(String::from("counted text"))
        ]
    );
    let usage = &run.chat().usage;
    assert_eq!(usage.prompt_tokens, 7);
    assert_eq!(usage.completion_tokens, 3);
    assert_eq!(usage.total_tokens, 10);
}

fn empty_headers_fall_back_to_the_counter() {
    let mut case = fixture();
    case.headers = headers("", "");
    let run = decode(&case, r#"{"results": [{"outputText": "fallback"}]}"#);
    assert_eq!(
        run.calls,
        vec![
            CounterCall::Messages,
            CounterCall::Text(String::from("fallback"))
        ]
    );
    assert_eq!(run.chat().usage.total_tokens, 10);
}

fn mixed_case_headers_are_read() {
    let mut case = fixture();
    case.headers = vec![
        (
            String::from("X-Amzn-Bedrock-Input-Token-Count"),
            String::from("12"),
        ),
        (
            String::from("X-Amzn-Bedrock-Output-Token-Count"),
            String::from("5"),
        ),
    ];
    let run = decode(
        &case,
        r#"{"results": [{"outputText": "x"}], "usage": {"prompt_tokens": 99, "completion_tokens": 99, "total_tokens": 198}}"#,
    );
    assert!(run.calls.is_empty());
    let usage = &run.chat().usage;
    assert_eq!(usage.prompt_tokens, 12);
    assert_eq!(usage.completion_tokens, 5);
    assert_eq!(usage.total_tokens, 17);
}

fn python_int_forms_are_parsed() {
    let mut case = fixture();
    case.headers = headers(" 1_1 ", "+4");
    let run = decode(&case, r#"{"results": [{"outputText": "x"}]}"#);
    let usage = &run.chat().usage;
    assert_eq!(usage.prompt_tokens, 11);
    assert_eq!(usage.completion_tokens, 4);
    assert_eq!(usage.total_tokens, 15);
}

fn invalid_headers_surface() {
    let mut case = fixture();
    case.headers = headers("abc", "4");
    let run = decode(&case, r#"{"results": [{"outputText": "x"}]}"#);
    assert!(matches!(
        run.failure(),
        InvokeReplyError::TokenCountHeader { name, .. }
            if *name == "x-amzn-bedrock-input-token-count"
    ));
    assert_eq!(run.status(), None);

    case.headers = headers("4", "nope");
    let run = decode(&case, r#"{"results": [{"outputText": "x"}]}"#);
    assert!(matches!(
        run.failure(),
        InvokeReplyError::TokenCountHeader { name, .. }
            if *name == "x-amzn-bedrock-output-token-count"
    ));
}

fn negative_headers_surface() {
    let mut case = fixture();
    case.headers = headers("-3", "4");
    let run = decode(&case, r#"{"results": [{"outputText": "x"}]}"#);
    assert!(matches!(
        run.failure(),
        InvokeReplyError::UsageRange { name } if *name == "prompt_tokens"
    ));
    assert_eq!(run.status(), None);

    case.headers = headers("4", "-2");
    let run = decode(&case, r#"{"results": [{"outputText": "x"}]}"#);
    assert!(matches!(
        run.failure(),
        InvokeReplyError::UsageRange { name } if *name == "completion_tokens"
    ));
}

fn counter_failures_surface() {
    let case = fixture();
    let counter = FixtureCounter::failing();
    let run = decode_with(&case, br#"{"results": [{"outputText": "x"}]}"#, &counter);
    assert!(matches!(run.failure(), InvokeReplyError::TokenCount { .. }));
    assert_eq!(run.status(), None);

    let mut header_case = fixture();
    header_case.headers = headers("5", "");
    let counter = FixtureCounter::failing();
    let run = decode_with(
        &header_case,
        br#"{"results": [{"outputText": "x"}]}"#,
        &counter,
    );
    assert!(matches!(run.failure(), InvokeReplyError::TokenCount { .. }));

    let mut tool_case = fixture();
    tool_case.response = with_tool_calls(seeded(), Vec::new());
    let run = decode(&tool_case, r#"{"results": [{}]}"#);
    assert!(matches!(run.failure(), InvokeReplyError::TokenCount { .. }));
}

fn created_and_model_are_set() {
    let mut case = fixture();
    case.headers = headers("1", "2");
    case.now = 1_700_000_123.9;
    let mut response = seeded();
    response.system_fingerprint = Some(String::from("fp-1"));
    case.response = response;
    let run = decode(&case, r#"{"results": [{"outputText": "x"}]}"#);
    let reply = run.chat();
    assert_eq!(reply.created, 1_700_000_123);
    assert_eq!(reply.model, MODEL);
    assert_eq!(reply.id, "chatcmpl-supplied");
    assert_eq!(reply.object, "chat.completion");
    assert_eq!(reply.system_fingerprint.as_deref(), Some("fp-1"));
    assert_eq!(reply.usage.prompt_tokens, 1);
    assert_eq!(reply.usage.completion_tokens, 2);
    assert_eq!(reply.usage.total_tokens, 3);
}
