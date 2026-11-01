use super::*;

#[test]
fn reasoning_fields_behavior_tests_behavior() {
    let tagged = message(
        Some(Content::Text("<think>because</think>visible".to_owned())),
        Object::new(),
    );
    assert_eq!(
        extract_reasoning_content(&tagged),
        (
            Some("because".to_owned()),
            Some(Content::Text("visible".to_owned()))
        )
    );

    let parts_content = message(Some(Content::Parts(Vec::new())), Object::new());
    assert_eq!(
        extract_reasoning_content(&parts_content),
        (None, Some(Content::Parts(Vec::new()))),
        "a non-string content falls through unchanged"
    );

    let no_content = message(None, Object::new());
    assert_eq!(extract_reasoning_content(&no_content), (None, None));
}
