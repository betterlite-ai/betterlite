use super::*;

#[test]
fn reasoning_fields_behavior_tests_behavior() {
    let with_reasoning_content = message(
        Some(Content::Text("<think>ignored</think>visible".to_owned())),
        Object::from_iter([(
            "reasoning_content".to_owned(),
            Json::Text("because".to_owned()),
        )]),
    );
    assert_eq!(
        extract_reasoning_content(&with_reasoning_content),
        (
            Some("because".to_owned()),
            with_reasoning_content.content.clone()
        )
    );

    let with_null_reasoning_content = message(
        Some(Content::Text("<think>ignored</think>visible".to_owned())),
        Object::from_iter([("reasoning_content".to_owned(), Json::Null)]),
    );
    assert_eq!(
        extract_reasoning_content(&with_null_reasoning_content),
        (None, with_null_reasoning_content.content.clone()),
        "a present reasoning_content key wins even when its value is null"
    );

    let with_reasoning = message(
        None,
        Object::from_iter([("reasoning".to_owned(), Json::Text("why".to_owned()))]),
    );
    assert_eq!(
        extract_reasoning_content(&with_reasoning),
        (Some("why".to_owned()), None)
    );
}
