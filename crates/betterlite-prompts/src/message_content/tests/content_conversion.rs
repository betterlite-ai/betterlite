use super::*;
use rstest::rstest;

#[rstest]
#[case::text(Content::Text("hello".to_owned()), "hello")]
#[case::parts(
    Content::Parts(vec![ContentPart {
        kind: "text".to_owned(),
        fields: Object::from_iter([("text".to_owned(), Json::Text("hello".to_owned()))]),
    }]),
    "hello"
)]
fn message_content_behavior_tests_behavior(#[case] content: Content, #[case] expected: &str) {
    assert_eq!(
        convert_content_list_to_str(&message("user", Some(content))),
        expected
    );
}
