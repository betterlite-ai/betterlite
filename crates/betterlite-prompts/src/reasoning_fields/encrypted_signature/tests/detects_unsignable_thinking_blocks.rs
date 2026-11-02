use super::*;
use rstest::rstest;

#[rstest]
#[case::encrypted_block_is_unsignable(
    "thinking",
    Object::from_iter([(
        "signature".to_owned(),
        Json::Text("litellm_encrypted_reasoning:abc".to_owned()),
    )]),
    true
)]
#[case::missing_signature_is_unsignable(
    "thinking",
    Object::from_iter([("thinking".to_owned(), Json::Text("because".to_owned()))]),
    true
)]
#[case::empty_signature_is_unsignable(
    "thinking",
    Object::from_iter([
        ("thinking".to_owned(), Json::Text("because".to_owned())),
        ("signature".to_owned(), Json::Text(String::new())),
    ]),
    true
)]
#[case::whitespace_only_thinking_is_unsignable(
    "thinking",
    Object::from_iter([
        ("thinking".to_owned(), Json::Text("   ".to_owned())),
        ("signature".to_owned(), Json::Text("anthropic-signed".to_owned())),
    ]),
    true
)]
#[case::signed_thinking_block_is_signable(
    "thinking",
    Object::from_iter([
        ("thinking".to_owned(), Json::Text("because".to_owned())),
        ("signature".to_owned(), Json::Text("anthropic-signed".to_owned())),
    ]),
    false
)]
#[case::redacted_block_is_always_signable("redacted_thinking", Object::new(), false)]
fn reasoning_fields_behavior_tests_behavior(
    #[case] kind: &str,
    #[case] fields: Object,
    #[case] expected: bool,
) {
    let block = content_part(kind, fields);
    assert_eq!(is_unsignable_thinking_block(&block), expected);
}
