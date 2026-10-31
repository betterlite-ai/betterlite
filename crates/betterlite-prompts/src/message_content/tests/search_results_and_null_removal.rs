use super::*;

#[test]
fn message_content_behavior_tests_behavior() {
    let mut input = message("assistant", None);
    input.name = Some("assistant-name".to_owned());
    input.provider_specific_fields.insert(
        "search_results".to_owned(),
        Json::List(vec![Json::Object(Object::from_iter([
            ("source".to_owned(), Json::Text("s".to_owned())),
            ("title".to_owned(), Json::Text("t".to_owned())),
            (
                "content".to_owned(),
                Json::List(vec![Json::Object(Object::from_iter([(
                    "text".to_owned(),
                    Json::Text("c".to_owned()),
                )]))]),
            ),
            (
                "citations".to_owned(),
                Json::List(vec![Json::Text("x".to_owned())]),
            ),
        ]))]),
    );
    assert_eq!(
        extract_search_results_text(input.provider_specific_fields.get("search_results")),
        "stc[\"x\"]"
    );
    assert_eq!(convert_content_list_to_str(&input), "stc[\"x\"]");
    input
        .provider_specific_fields
        .insert("vendor".to_owned(), Json::Null);
    let stripped = strip_none_values_from_message(&input);
    assert_eq!(stripped.content, None);
    assert_eq!(stripped.name, Some("assistant-name".to_owned()));
    assert!(!stripped.provider_specific_fields.contains_key("vendor"));
    assert!(
        stripped
            .provider_specific_fields
            .contains_key("search_results")
    );
}
