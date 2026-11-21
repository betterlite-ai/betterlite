use betterlite_foundry::anthropic::request::Style;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;

use super::support::{
    ANTHROPIC_REPLY, ClaudeInputs, Env, HOST, KEY, Ran, field, first_post, run_claude,
};
use crate::foundry_dispatch::claude;
use crate::foundry_dispatch::fixtures::claude::Wire;
use crate::foundry_dispatch::route;

pub(super) fn anthropic_case() {
    buffered_sync_call();
    async_buffered_call();
    async_stream_call();
    provider_failure();
}

fn buffered_sync_call() {
    let inputs = ClaudeInputs::new();
    let wire = Wire::new(200, ANTHROPIC_REPLY);
    let (ran, logger) = run_claude(inputs.call(Style::Sync, false), &wire);
    let expected = Ran::Reply {
        model: Some(String::from("claude-sonnet-4-5")),
        content: Some(String::from("hello")),
    };
    assert_eq!(ran, expected);
    assert!(logger.calls.is_empty());
    let posted = first_post(&wire);
    assert_eq!(
        posted.url,
        "https://res.services.ai.azure.com/anthropic/v1/messages"
    );
    assert_eq!(posted.api_key.as_deref(), Some(KEY));
}

// acompletion alone logs the buffered reply, and the async mapper carries the optional params
fn async_buffered_call() {
    let inputs = ClaudeInputs::new();
    let wire = Wire::new(200, ANTHROPIC_REPLY);
    let (ran, logger) = run_claude(inputs.call(Style::Async, false), &wire);
    assert!(matches!(ran, Ran::Reply { .. }));
    assert_eq!(logger.calls.len(), 1);
    let expected_report: Json =
        serde_json::from_str(r#"{"model":"claude-sonnet-4-5","content":"hello"}"#)
            .expect("the reply report");
    assert_eq!(logger.calls[0].original_response, expected_report);
    assert_eq!(
        field(&first_post(&wire).body, "temperature"),
        &JsonValue::Text(String::from("warm"))
    );
}

// an async call with stream opens the stream and logs it, without additional_args headers
fn async_stream_call() {
    let inputs = ClaudeInputs::new();
    let mut wire = Wire::new(200, "");
    wire.lines = vec![String::from(
        "event: message_stop\ndata: {\"type\":\"message_stop\"}",
    )];
    let (ran, logger) = run_claude(inputs.call(Style::Async, true), &wire);
    assert_eq!(ran, Ran::Stream);
    assert_eq!(logger.calls.len(), 1);
    assert!(logger.calls[0].headers.is_none());
}

// a provider failure stays unlogged, and the valid body behind the status proves the transport reads the status
fn provider_failure() {
    let inputs = ClaudeInputs::new();
    let wire = Wire::new(500, ANTHROPIC_REPLY);
    let (ran, logger) = run_claude(inputs.call(Style::Sync, false), &wire);
    assert_eq!(ran, Ran::Failed);
    assert!(logger.calls.is_empty());
}

pub(super) fn claude_missing_base() {
    missing_base_fails();
    global_base_is_a_fallback();
}

fn missing_base_fails() {
    let inputs = ClaudeInputs::new();
    let env = Env::default();
    let mut call = inputs.call(Style::Sync, false);
    call.api_base = None;
    let error = claude::prepare(&call, &env.globals())
        .err()
        .expect("no base is a failure");
    assert_eq!(error.to_string(), route::CLAUDE_BASE_MESSAGE);
}

// a global base is a fallback, and a global header map never reaches this branch
fn global_base_is_a_fallback() {
    let inputs = ClaudeInputs::new();
    let env = Env::with_header("x-global", "g").global_base(HOST);
    let mut call = inputs.call(Style::Sync, false);
    call.api_base = None;
    let globals = env.globals();
    let prepared = claude::prepare(&call, &globals).expect("the global base resolves");
    let sent = claude::request(&prepared, &call, &globals);
    assert_eq!(
        sent.api_base,
        "https://res.services.ai.azure.com/anthropic/v1/messages"
    );
    assert!(sent.caller_headers.is_empty());
}

pub(super) fn claude_url_rewrite() {
    let rows = [
        ("https://x.test", "https://x.test/anthropic/v1/messages"),
        ("https://x.test/", "https://x.test/anthropic/v1/messages"),
        (
            "https://x.test/anthropic",
            "https://x.test/anthropic/v1/messages",
        ),
        (
            "https://x.test/anthropic/v1/messages",
            "https://x.test/anthropic/v1/messages",
        ),
        (
            "https://x.test/anthropic/v1/messages/",
            "https://x.test/anthropic/v1/messages",
        ),
        (
            "https://x.test/anthropic/other/route",
            "https://x.test/anthropic/v1/messages",
        ),
        ("", ""),
    ];
    for (base, expected) in rows {
        assert_eq!(claude::endpoint(base), expected, "base {base}");
    }
}
