#![cfg(test)]

use crate::http_analytics_loggers::tests::support::json;

use super::{response_dump, tag_text, usage_token};

#[rstest::rstest]
#[case::text(r#""hello""#, "hello")]
#[case::number("2", "2")]
#[case::boolean("true", "true")]
#[case::null("null", "null")]
#[case::list(r#"["a", 1]"#, r#"["a", 1]"#)]
fn renders_tag_values_as_python_strings(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(tag_text(&json(source)), expected);
}

#[rstest::rstest]
#[case::usage(r#"{"usage":{"prompt_tokens":3}}"#, "prompt_tokens", "3")]
#[case::other_token(r#"{"usage":{"completion_tokens":2}}"#, "completion_tokens", "2")]
#[case::missing_usage(r#"{"id":"x"}"#, "prompt_tokens", "null")]
#[case::null_usage(r#"{"usage":null}"#, "prompt_tokens", "null")]
#[case::missing_token(r#"{"usage":{}}"#, "total_tokens", "null")]
fn reads_a_usage_token_or_null(
    #[case] response: &str,
    #[case] token: &'static str,
    #[case] expected: &str,
) {
    assert_eq!(usage_token(&json(response), token), json(expected));
}

#[test]
fn dumps_a_missing_response_as_an_empty_object() {
    assert_eq!(response_dump(None), json("{}"));
    assert_eq!(response_dump(Some(&json("null"))), json("{}"));
    assert_eq!(
        response_dump(Some(&json(r#"{"id":"x"}"#))),
        json(r#"{"id":"x"}"#)
    );
}
