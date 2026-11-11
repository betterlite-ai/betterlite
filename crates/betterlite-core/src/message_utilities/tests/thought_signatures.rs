use rstest::rstest;

use super::support;
use crate::message_utilities::thought_signatures::{
    ThoughtSignatureError, is_gemini_model, process_tool_message_id, strip_assistant_tool_calls,
    strip_signature_from_id, strip_signatures_from_messages,
};

const SEPARATOR: &str = "__thought__";

#[rstest]
#[case::no_model(None, None, false)]
#[case::gemini_name(Some("gemini-pro"), None, true)]
#[case::case_insensitive(Some("Gemini-2.0"), None, true)]
#[case::gemini_provider(Some("gpt-4o"), Some("gemini"), true)]
#[case::vertex_without_gemini(Some("gpt-4o"), Some("vertex_ai"), false)]
#[case::vertex_gemini(Some("gemini-pro"), Some("vertex_ai"), true)]
#[case::vertex_beta_uppercase(Some("GEMINI-pro"), Some("vertex_ai_beta"), true)]
#[case::vertex_other(Some("claude"), Some("vertex_ai"), false)]
#[case::vertex_no_model(None, Some("vertex_ai"), false)]
#[tokio::test]
async fn thought_signature_gemini_detection(
    #[case] model: Option<&str>,
    #[case] provider: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(is_gemini_model(model, provider), expected);
}

#[rstest]
#[case::present("abc__thought__sig", "abc")]
#[case::absent("abc", "abc")]
#[case::twice("a__thought__b__thought__c", "a")]
#[tokio::test]
async fn thought_signature_id_split_at_the_first_separator(
    #[case] tool_call_id: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        strip_signature_from_id(tool_call_id, SEPARATOR),
        Ok(expected.to_owned())
    );
}

#[tokio::test]
async fn thought_signature_empty_separator_is_an_error() {
    assert_eq!(
        strip_signature_from_id("abc", ""),
        Err(ThoughtSignatureError::EmptySeparator)
    );
}

#[rstest]
#[case::assistant(
    r#"{"role": "assistant", "tool_calls": [{"id": "c1__thought__s1", "type": "function"}, {"id": "c2"}, "x", {"id": 5}]}"#,
    r#"{"role": "assistant", "tool_calls": [{"id": "c1", "type": "function"}, {"id": "c2"}, "x", {"id": 5}]}"#
)]
#[case::other_role(
    r#"{"role": "user", "tool_calls": [{"id": "c1__thought__s1"}]}"#,
    r#"{"role": "user", "tool_calls": [{"id": "c1__thought__s1"}]}"#
)]
#[case::no_tool_calls(
    r#"{"role": "assistant", "tool_calls": "x"}"#,
    r#"{"role": "assistant", "tool_calls": "x"}"#
)]
#[case::not_an_object("\"x\"", "\"x\"")]
#[tokio::test]
async fn thought_signature_strips_assistant_tool_call_ids(
    #[case] input: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        strip_assistant_tool_calls(&support::value(input), SEPARATOR),
        Ok(support::value(expected))
    );
}

#[rstest]
#[case::tool(
    r#"{"role": "tool", "tool_call_id": "c1__thought__s1"}"#,
    r#"{"role": "tool", "tool_call_id": "c1"}"#
)]
#[case::other_role(
    r#"{"role": "user", "tool_call_id": "c1__thought__s1"}"#,
    r#"{"role": "user", "tool_call_id": "c1__thought__s1"}"#
)]
#[case::no_id(
    r#"{"role": "tool", "content": "x"}"#,
    r#"{"role": "tool", "content": "x"}"#
)]
#[case::numeric_id(
    r#"{"role": "tool", "tool_call_id": 5}"#,
    r#"{"role": "tool", "tool_call_id": 5}"#
)]
#[tokio::test]
async fn thought_signature_strips_tool_message_ids(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(
        process_tool_message_id(&support::value(input), SEPARATOR),
        Ok(support::value(expected))
    );
}

#[tokio::test]
async fn thought_signature_removes_from_every_message_and_keeps_the_input() {
    let messages = support::values(
        r#"[
    {"role": "assistant", "content": null, "tool_calls": [{"id": "c1__thought__s1", "type": "function", "function": {"name": "f", "arguments": "{}"}}]},
    {"role": "tool", "tool_call_id": "c1__thought__s1", "content": "x"},
    {"role": "user", "content": "hi"},
    "not-a-dict"
]"#,
    );
    let processed = strip_signatures_from_messages(&messages, SEPARATOR).unwrap();
    assert_eq!(
        processed,
        support::values(
            r#"[
    {"role": "assistant", "content": null, "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}}]},
    {"role": "tool", "tool_call_id": "c1", "content": "x"},
    {"role": "user", "content": "hi"},
    "not-a-dict"
]"#
        )
    );
    assert_eq!(
        messages,
        support::values(
            r#"[
    {"role": "assistant", "content": null, "tool_calls": [{"id": "c1__thought__s1", "type": "function", "function": {"name": "f", "arguments": "{}"}}]},
    {"role": "tool", "tool_call_id": "c1__thought__s1", "content": "x"},
    {"role": "user", "content": "hi"},
    "not-a-dict"
]"#
        )
    );
}
