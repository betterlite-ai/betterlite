use super::*;

#[test]
fn message_content_behavior_tests_behavior() {
    let parted = message(
        "user",
        Some(Content::Parts(vec![ContentPart {
            kind: "text".to_owned(),
            fields: Object::from_iter([("text".to_owned(), Json::Text("hi".to_owned()))]),
        }])),
    );
    let empty = message("user", None);
    let mut input = [parted, empty];
    let converted = convert_message_content_lists(&mut input);
    assert_eq!(converted[0].content, Some(Content::Text("hi".to_owned())));
    assert_eq!(converted[1].content, None);
}
