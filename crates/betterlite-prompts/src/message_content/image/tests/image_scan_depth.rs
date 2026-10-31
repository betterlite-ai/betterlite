use super::*;
use rstest::rstest;

#[rstest]
#[case::nested_tool_result(
    part(
        "tool_result",
        Object::from_iter([(
            "content".to_owned(),
            Json::List(vec![Json::Object(Object::from_iter([
                ("type".to_owned(), Json::Text("image_url".to_owned())),
                ("image_url".to_owned(), Json::Text("https://nested".to_owned())),
            ]))]),
        )]),
    ),
    true
)]
#[case::text_only(
    part(
        "text",
        Object::from_iter([("text".to_owned(), Json::Text("hi".to_owned()))]),
    ),
    false
)]
fn message_content_behavior_tests_behavior(#[case] input: ContentPart, #[case] expected: bool) {
    assert_eq!(
        request_contains_image_content(&[message(vec![input])]),
        expected
    );
}
