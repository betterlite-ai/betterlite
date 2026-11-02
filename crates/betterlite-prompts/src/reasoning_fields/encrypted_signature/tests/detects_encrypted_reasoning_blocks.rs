use super::*;
use rstest::rstest;

#[rstest]
#[case::thinking_with_encrypted_signature(
    "thinking",
    "signature",
    "litellm_encrypted_reasoning:abc",
    true
)]
#[case::redacted_with_encrypted_data(
    "redacted_thinking",
    "data",
    "litellm_encrypted_reasoning:abc",
    true
)]
#[case::thinking_with_anthropic_signature("thinking", "signature", "anthropic-signed", false)]
#[case::tool_use_block_is_never_encrypted(
    "tool_use",
    "signature",
    "litellm_encrypted_reasoning:abc",
    false
)]
fn reasoning_fields_behavior_tests_behavior(
    #[case] kind: &str,
    #[case] field: &str,
    #[case] value: &str,
    #[case] expected: bool,
) {
    let block = content_part(
        kind,
        Object::from_iter([(field.to_owned(), Json::Text(value.to_owned()))]),
    );
    assert_eq!(is_encrypted_reasoning_block(&block), expected);
    if expected {
        assert_eq!(encrypted_content_of_block(&block), Some("abc".to_owned()));
    }
}
