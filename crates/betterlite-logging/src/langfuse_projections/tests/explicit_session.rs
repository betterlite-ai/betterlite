#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::session_header::is_session_header_trace;
use super::support::value;

fn request(headers: &str) -> Json {
    value(&format!(r#"{{"headers": {headers}}}"#))
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

#[test]
fn explicit_session_header_matches_the_session_and_trace_ids() {
    let proxy =
        request(r#"{"langfuse_session_id": "sess-999", "X-LiteLLM-Session-Id": "trace-12345678"}"#);
    assert!(is_session_header_trace(
        Some(&text("trace-12345678")),
        Some(&text("sess-999")),
        Some(&proxy)
    ));
}

#[test]
fn explicit_session_header_mismatch_is_not_a_session_trace() {
    let proxy =
        request(r#"{"langfuse_session_id": "another-session", "x-session-id": "trace-12345678"}"#);
    assert!(!is_session_header_trace(
        Some(&text("trace-12345678")),
        Some(&text("sess-999")),
        Some(&proxy)
    ));
}

#[test]
fn x_litellm_session_id_matches_before_the_trace_id_shape_check() {
    let proxy = request(r#"{"x-litellm-session-id": "abc"}"#);
    assert!(is_session_header_trace(
        Some(&text("abc")),
        Some(&text("abc")),
        Some(&proxy)
    ));
}

#[test]
fn trace_ids_with_a_disallowed_character_are_not_session_headers() {
    let proxy = request(r#"{"x-session-id": "trace.12345678"}"#);
    assert!(!is_session_header_trace(
        Some(&text("trace.12345678")),
        Some(&text("trace.12345678")),
        Some(&proxy)
    ));
}

#[test]
fn x_session_header_matches_the_trace_id() {
    let proxy = request(r#"{"x-session-id": "trace-12345678"}"#);
    assert!(is_session_header_trace(
        Some(&text("trace-12345678")),
        Some(&text("trace-12345678")),
        Some(&proxy)
    ));
}

#[test]
fn pattern_session_header_matches_a_custom_name() {
    let proxy = request(r#"{"x-my-agent-session-id": "trace-12345678"}"#);
    assert!(is_session_header_trace(
        Some(&text("trace-12345678")),
        Some(&text("trace-12345678")),
        Some(&proxy)
    ));
}

#[test]
fn pattern_session_header_needs_a_middle_segment() {
    let proxy = request(r#"{"x--session-id": "trace-12345678"}"#);
    assert!(!is_session_header_trace(
        Some(&text("trace-12345678")),
        Some(&text("trace-12345678")),
        Some(&proxy)
    ));
}

#[rstest]
#[case::lowercase_agent("codex-cli/0.1.0", true)]
#[case::uppercase_agent("Codex/1.0", true)]
#[case::prefix_without_a_separator("codexfoo/1.0", false)]
#[case::another_agent("betterlite/0.1.0", false)]
fn codex_session_header_matches_with_the_codex_user_agent(
    #[case] user_agent: &str,
    #[case] expected: bool,
) {
    let proxy = request(&format!(
        r#"{{"session-id": "trace-12345678", "user-agent": "{user_agent}"}}"#
    ));
    assert_eq!(
        is_session_header_trace(
            Some(&text("trace-12345678")),
            Some(&text("trace-12345678")),
            Some(&proxy)
        ),
        expected
    );
}

#[rstest]
#[case::session_id("session-id")]
#[case::session_id_underscore("session_id")]
#[case::thread_id("thread-id")]
#[case::conversation_id("conversation_id")]
fn codex_session_keys_mark_the_trace_as_a_session_header(#[case] header: &str) {
    let proxy = request(&format!(
        r#"{{"{header}": "trace-12345678", "user-agent": "codex-cli/0.1.0"}}"#
    ));
    assert!(is_session_header_trace(
        Some(&text("trace-12345678")),
        Some(&text("trace-12345678")),
        Some(&proxy)
    ));
}

#[test]
fn x_litellm_trace_id_blocks_session_detection() {
    let proxy = request(r#"{"x-litellm-trace-id": "1", "x-session-id": "trace-12345678"}"#);
    assert!(!is_session_header_trace(
        Some(&text("trace-12345678")),
        Some(&text("trace-12345678")),
        Some(&proxy)
    ));
}

#[test]
fn langfuse_trace_id_blocks_session_detection() {
    let proxy = request(r#"{"langfuse_trace_id": "existing", "x-session-id": "trace-12345678"}"#);
    assert!(!is_session_header_trace(
        Some(&text("trace-12345678")),
        Some(&text("trace-12345678")),
        Some(&proxy)
    ));
}

#[test]
fn langfuse_trace_id_null_does_not_block_session_detection() {
    let proxy = request(r#"{"langfuse_trace_id": null, "x-session-id": "trace-12345678"}"#);
    assert!(is_session_header_trace(
        Some(&text("trace-12345678")),
        Some(&text("trace-12345678")),
        Some(&proxy)
    ));
}

#[test]
fn short_trace_ids_are_not_session_headers() {
    let proxy = request(r#"{"x-session-id": "short"}"#);
    assert!(!is_session_header_trace(
        Some(&text("short")),
        Some(&text("short")),
        Some(&proxy)
    ));
}

#[test]
fn non_string_ids_and_missing_requests_are_not_session_headers() {
    let proxy = request(r#"{"x-session-id": "trace-12345678"}"#);
    assert!(!is_session_header_trace(
        Some(&Json::Null),
        Some(&text("trace-12345678")),
        Some(&proxy)
    ));
    assert!(!is_session_header_trace(
        Some(&text("trace-12345678")),
        Some(&text("trace-12345678")),
        None
    ));
    assert!(!is_session_header_trace(
        Some(&text("trace-12345678")),
        Some(&text("trace-12345678")),
        Some(&Json::Text("not a request".to_owned()))
    ));
}
