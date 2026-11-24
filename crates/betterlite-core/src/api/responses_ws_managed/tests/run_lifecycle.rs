use std::sync::Arc;

use super::support;
use crate::api::responses_ws_managed::run::run;

const FIRST_FRAME: &str = r#"{"type": "response.create", "input": "hello"}"#;
const SECOND_FRAME: &str = r#"{"type": "response.create", "input": "again"}"#;
const FIRST: &str = r#"{"type": "response.completed", "response": {"id": "r-first", "output": [{"type": "message", "content": [{"type": "output_text", "text": "A"}]}]}}"#;
const SECOND: &str = r#"{"type": "response.completed", "response": {"id": "r2", "output": [{"type": "message", "content": [{"type": "output_text", "text": "B"}]}]}}"#;
const STORED_FIRST: &str = r#"{"r-first": [
    {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hello"}]},
    {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "A"}]}
]}"#;
const STORED_BOTH: &str = r#"{"r-first": [
    {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hello"}]},
    {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "A"}]}
], "r2": [
    {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "again"}]},
    {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "B"}]}
]}"#;

#[tokio::test]
async fn the_first_message_is_processed_before_any_frame_is_received() {
    let shared = support::shared();
    let mut connection = support::connection(support::MODEL, support::sink(&shared));
    connection.first_message = Some(FIRST_FRAME.to_owned());
    let mut handler = support::handler(connection);
    let provider =
        support::ScriptedProvider::new(&shared, vec![support::script(vec![support::event(FIRST)])]);
    let frames = support::source(&shared, &[], "client gone");

    let ((), logs) = support::capture(run(&mut handler, &provider, frames)).await;
    assert_eq!(
        support::trace(&shared),
        ["provider", "provider-end", "receive"]
    );
    assert_eq!(
        logs,
        vec![
            "ManagedResponsesWS: stored 2 messages for response_id=r-first",
            "ManagedResponsesWS: client disconnected: client gone",
        ]
    );
    assert_eq!(support::history(&handler), support::json(STORED_FIRST));
}

#[tokio::test]
async fn each_turn_finishes_before_the_next_frame_is_read() {
    let shared = support::shared();
    let mut handler = support::handler(support::connection(support::MODEL, support::sink(&shared)));
    let scripts = vec![
        support::script(vec![support::event(FIRST)]),
        support::script(vec![support::event(SECOND)]),
    ];
    let provider = support::ScriptedProvider::new(&shared, scripts);
    let frames = support::source(&shared, &[FIRST_FRAME, SECOND_FRAME], "client gone");

    let ((), logs) = support::capture(run(&mut handler, &provider, frames)).await;
    let expected_trace = [
        "receive",
        "provider",
        "provider-end",
        "receive",
        "provider",
        "provider-end",
        "receive",
    ];
    assert_eq!(support::trace(&shared), expected_trace);
    assert_eq!(
        logs,
        vec![
            "ManagedResponsesWS: stored 2 messages for response_id=r-first",
            "ManagedResponsesWS: stored 2 messages for response_id=r2",
            "ManagedResponsesWS: client disconnected: client gone",
        ]
    );
    assert_eq!(support::history(&handler), support::json(STORED_BOTH));
}

#[tokio::test]
async fn an_error_that_escapes_the_loop_is_reported_and_ends_the_connection() {
    let shared = support::shared();
    let mut connection = support::connection(support::MODEL, support::sink(&shared));
    let failure = support::QuotaFailure::InternalServer("unexpected boom");
    let quota = support::ScriptedQuota::new(&shared, Some(failure));
    connection.quota_callbacks = Some(vec![Arc::new(quota)]);
    connection.first_message = Some(FIRST_FRAME.to_owned());
    let mut handler = support::handler(connection);
    let provider = support::ScriptedProvider::idle(&shared);
    let frames = support::source(&shared, &[SECOND_FRAME], "client gone");

    let ((), logs) = support::capture(run(&mut handler, &provider, frames)).await;
    let error_frame = r#"{"type": "error", "error": {"type": "server_error", "message": "Internal server error: unexpected boom"}}"#;
    assert_eq!(support::frames(&shared), vec![support::json(error_frame)]);
    assert_eq!(
        logs,
        vec!["ManagedResponsesWS: unexpected error: unexpected boom"]
    );
    assert_eq!(support::trace(&shared), ["quota"]);
    assert!(support::calls(&shared).is_empty());
    assert_eq!(support::history(&handler), support::json("{}"));
}
