use super::*;

#[test]
fn reasoning_fields_behavior_tests_behavior() {
    let blocks = [
        content_part(
            "thinking",
            Object::from_iter([("thinking".to_owned(), Json::Text("first".to_owned()))]),
        ),
        content_part(
            "redacted_thinking",
            Object::from_iter([("data".to_owned(), Json::Text("opaque".to_owned()))]),
        ),
        content_part(
            "thinking",
            Object::from_iter([("thinking".to_owned(), Json::Text("second".to_owned()))]),
        ),
    ];

    assert_eq!(reasoning_content_from_blocks(&blocks), "first\nsecond");
    assert_eq!(reasoning_content_from_blocks(&[]), "");
}
