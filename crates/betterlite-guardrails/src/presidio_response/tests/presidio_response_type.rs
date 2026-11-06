#![cfg(test)]

use rstest::rstest;
use serde_json::value::RawValue;

use crate::presidio_response::anthropic::is_anthropic_message_response;

#[rstest]
#[case::message_with_text_blocks(
    r#"{"type":"message","content":[{"type":"text","text":"hi"}]}"#,
    true
)]
#[case::message_with_an_empty_content_list(r#"{"type":"message","content":[]}"#, true)]
#[case::content_is_not_a_list(r#"{"type":"message","content":"hi"}"#, false)]
#[case::content_is_missing(r#"{"type":"message"}"#, false)]
#[case::content_is_null(r#"{"type":"message","content":null}"#, false)]
#[case::type_is_not_message(r#"{"type":"error","content":[]}"#, false)]
#[case::type_is_missing(r#"{"content":[]}"#, false)]
#[case::type_is_null(r#"{"type":null,"content":[]}"#, false)]
#[case::the_body_is_a_list(r#"[{"type":"message","content":[]}]"#, false)]
#[case::the_body_is_a_string(r#""message""#, false)]
#[case::the_body_is_null("null", false)]
fn is_anthropic_message_response_checks_type_and_content(
    #[case] json: &str,
    #[case] expected: bool,
) {
    let raw: Box<RawValue> = serde_json::from_str(json).expect("JSON value");
    assert_eq!(is_anthropic_message_response(&raw), expected);
}
