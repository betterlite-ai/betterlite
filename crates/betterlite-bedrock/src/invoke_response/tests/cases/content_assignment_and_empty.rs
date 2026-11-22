#![cfg(test)]

use crate::invoke_response::error::InvokeReplyError;
use crate::invoke_response::tests::support::{
    decode, fixture, headers, seeded, tool_call, with_tool_calls,
};

pub(in crate::invoke_response::tests) fn content_assignment_and_empty() {
    missing_and_empty_text_error();
    non_string_text_errors();
    tool_calls_keep_the_seed_content();
    empty_tool_calls_suppress_the_error();
    missing_choice_is_a_content_error();
}

fn missing_and_empty_text_error() {
    let case = fixture();
    let run = decode(&case, r#"{"results": [{}]}"#);
    assert_eq!(run.status(), Some(200));
    assert_eq!(run.message(), "Error parsing received text=None.\nError-");
    assert!(matches!(run.failure(), InvokeReplyError::Content(_)));

    let run = decode(&case, r#"{"results": [{"outputText": ""}]}"#);
    assert_eq!(run.status(), Some(200));
    assert_eq!(run.message(), "Error parsing received text=.\nError-");
}

fn non_string_text_errors() {
    let case = fixture();
    let run = decode(&case, r#"{"results": [{"outputText": 5}]}"#);
    assert_eq!(run.status(), Some(200));
    assert_eq!(
        run.message(),
        "Error parsing received text=5.\nError-the response text is not a string"
    );

    let run = decode(&case, r#"{"results": [{"outputText": true}]}"#);
    assert_eq!(
        run.message(),
        "Error parsing received text=True.\nError-the response text is not a string"
    );

    let run = decode(&case, r#"{"results": [{"outputText": ["a"]}]}"#);
    assert_eq!(
        run.message(),
        "Error parsing received text=[\"a\"].\nError-the response text is not a string"
    );

    let run = decode(&case, r#"{"results": [{"outputText": {"a": 1}}]}"#);
    assert_eq!(
        run.message(),
        "Error parsing received text={\"a\":1}.\nError-the response text is not a string"
    );
}

fn tool_calls_keep_the_seed_content() {
    let mut case = fixture();
    let mut response = seeded();
    response.choices[0].message.content = Some(String::from("kept"));
    case.response = with_tool_calls(response, vec![tool_call()]);
    case.headers = headers("2", "1");
    let run = decode(&case, r#"{"results": [{"outputText": ""}]}"#);
    let reply = run.chat();
    assert_eq!(reply.choices[0].message.content.as_deref(), Some("kept"));
    assert_eq!(
        reply.choices[0].message.tool_calls.as_ref().map(Vec::len),
        Some(1)
    );
    assert_eq!(reply.usage.total_tokens, 3);

    let mut missing = fixture();
    missing.response = with_tool_calls(seeded(), vec![tool_call()]);
    missing.headers = headers("2", "1");
    let run = decode(&missing, r#"{"results": [{}]}"#);
    assert_eq!(run.chat().choices[0].message.content, None);
}

fn empty_tool_calls_suppress_the_error() {
    let mut case = fixture();
    case.response = with_tool_calls(seeded(), Vec::new());
    case.headers = headers("4", "0");
    let run = decode(&case, r#"{"results": [{}]}"#);
    let reply = run.chat();
    assert_eq!(reply.choices[0].message.content, None);
    assert_eq!(
        reply.choices[0].message.tool_calls.as_ref().map(Vec::len),
        Some(0)
    );
    assert_eq!(reply.usage.completion_tokens, 0);
    assert_eq!(reply.usage.total_tokens, 4);
}

fn missing_choice_is_a_content_error() {
    let mut case = fixture();
    case.response.choices.clear();
    let run = decode(&case, r#"{"results": [{"outputText": "x"}]}"#);
    assert_eq!(run.status(), Some(200));
    assert_eq!(
        run.message(),
        "Error parsing received text=x.\nError-list index out of range"
    );
}
