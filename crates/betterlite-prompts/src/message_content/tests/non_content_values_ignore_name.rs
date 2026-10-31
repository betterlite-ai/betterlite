use super::*;

#[test]
fn message_content_behavior_tests_behavior() {
    // LiteLLM's ignore_keys excludes "name", so setting only a name must not count as set.
    let mut named_only = message("user", Some(Content::Text("hi".to_owned())));
    named_only.name = Some("caller".to_owned());
    assert!(!is_non_content_values_set(&named_only));

    let mut with_search_results = message("user", Some(Content::Text("hi".to_owned())));
    with_search_results.provider_specific_fields.insert(
        "search_results".to_owned(),
        Json::List(vec![Json::Object(Object::from_iter([(
            "source".to_owned(),
            Json::Text("s".to_owned()),
        )]))]),
    );
    assert!(is_non_content_values_set(&with_search_results));

    let mut with_provider_field = message("user", Some(Content::Text("hi".to_owned())));
    with_provider_field
        .provider_specific_fields
        .insert("vendor".to_owned(), Json::Bool(true));
    assert!(is_non_content_values_set(&with_provider_field));

    let mut with_null_field = message("user", Some(Content::Text("hi".to_owned())));
    with_null_field
        .provider_specific_fields
        .insert("vendor".to_owned(), Json::Null);
    assert!(!is_non_content_values_set(&with_null_field));
}
