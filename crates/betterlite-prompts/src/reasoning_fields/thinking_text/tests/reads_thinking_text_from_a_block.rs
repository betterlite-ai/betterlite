use super::*;
use rstest::rstest;

#[rstest]
#[case::thinking_block_with_text(
    "thinking",
    Object::from_iter([("thinking".to_owned(), Json::Text("because".to_owned()))]),
    "because"
)]
#[case::redacted_block_has_no_readable_text(
    "redacted_thinking",
    Object::from_iter([("data".to_owned(), Json::Text("opaque".to_owned()))]),
    ""
)]
#[case::thinking_block_missing_the_field("thinking", Object::new(), "")]
#[case::thinking_field_of_the_wrong_type(
    "thinking",
    Object::from_iter([(
        "thinking".to_owned(),
        Json::Number(serde_json::Number::from(7_i32)),
    )]),
    ""
)]
fn reasoning_fields_behavior_tests_behavior(
    #[case] kind: &str,
    #[case] fields: Object,
    #[case] expected: &str,
) {
    let block = content_part(kind, fields);
    assert_eq!(readable_thinking_text(&block), expected);
}
