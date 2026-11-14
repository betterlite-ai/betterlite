#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::guardrail_error_response::parse_error_response;

const DEFAULT_MESSAGE: &str = "Bedrock guardrail request failed";

#[rstest]
#[case::top_level_message(400, r#"{"message":"top level"}"#, "top level")]
#[case::nested_error_message(400, r#"{"error":{"message":"nested"}}"#, "nested")]
#[case::top_level_error_string(403, r#"{"error":"forbidden"}"#, "forbidden")]
#[case::plain_text(500, "gateway is down", "gateway is down")]
#[case::text_is_trimmed(500, "  gateway is down  ", "gateway is down")]
#[case::empty_body(500, "", DEFAULT_MESSAGE)]
#[case::non_object_json(500, "[1,2]", DEFAULT_MESSAGE)]
#[case::non_string_message(400, r#"{"message":42}"#, DEFAULT_MESSAGE)]
#[case::nested_non_string_message(400, r#"{"error":{"message":42}}"#, DEFAULT_MESSAGE)]
fn error_parsing(#[case] status: u16, #[case] body: &str, #[case] expected: &str) {
    let (parsed_status, message) = parse_error_response(status, body.as_bytes());
    assert_eq!(parsed_status, status);
    assert_eq!(message, expected);
}

#[rstest]
fn replays_the_guardrail_error_response_corpus(
    #[files("fuzz/corpus/guardrail_error_response/*")] path: PathBuf,
) {
    let body = fs::read(path).expect("the seed is readable");
    let (status, message) = parse_error_response(502, &body);
    assert_eq!(status, 502);
    assert!(!message.is_empty());
}

#[test]
fn error_parsing_keeps_the_status_code() {
    let (status, _message) = parse_error_response(429, br#"{"message":"throttled"}"#);
    assert_eq!(status, 429);
}

#[test]
fn error_parsing_reads_a_top_level_error_string() {
    let (status, message) = parse_error_response(403, br#"{"error":"forbidden"}"#);
    assert_eq!(status, 403);
    assert_eq!(message, "forbidden");
}
