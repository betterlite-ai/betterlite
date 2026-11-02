use super::*;

#[test]
fn reasoning_fields_behavior_tests_behavior() {
    let redacted = content_part(
        "redacted_thinking",
        Object::from_iter([("data".to_owned(), Json::Text("opaque".to_owned()))]),
    );

    let items = reasoning_items_from_blocks(&[redacted]);

    assert_eq!(
        items,
        Vec::new(),
        "a group with no readable text and no encrypted content yields no item"
    );
    assert_eq!(reasoning_items_from_blocks(&[]), Vec::new());
}
