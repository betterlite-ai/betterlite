use super::*;

#[test]
fn message_fields_behavior_tests_behavior() {
    let input = build_input();
    let filtered = filter_value_from_dict(&input, "secret", 0);

    assert_top_level_and_nested_object_are_stripped(&filtered);
    assert_list_items_are_filtered_in_place(&filtered);
    assert!(
        input.contains_key("secret"),
        "the source input must not be mutated"
    );
}

fn build_input() -> Object {
    Object::from_iter([
        ("secret".to_owned(), Json::Text("top".to_owned())),
        ("kept".to_owned(), Json::Bool(true)),
        (
            "outer".to_owned(),
            Json::Object(Object::from_iter([(
                "secret".to_owned(),
                Json::Text("nested".to_owned()),
            )])),
        ),
        (
            "items".to_owned(),
            Json::List(vec![
                Json::Object(Object::from_iter([(
                    "secret".to_owned(),
                    Json::Text("in-list".to_owned()),
                )])),
                Json::Text("not a dict".to_owned()),
                Json::Number(serde_json::Number::from(7_i32)),
            ]),
        ),
    ])
}

fn assert_top_level_and_nested_object_are_stripped(filtered: &Object) {
    assert!(!filtered.contains_key("secret"));
    assert!(filtered.contains_key("kept"));
    let Some(Json::Object(outer)) = filtered.get("outer") else {
        panic!("outer key must survive filtering");
    };
    assert!(!outer.contains_key("secret"));
}

fn assert_list_items_are_filtered_in_place(filtered: &Object) {
    let Some(Json::List(items)) = filtered.get("items") else {
        panic!("items key must survive filtering");
    };
    assert_eq!(
        items,
        &[
            Json::Object(Object::new()),
            Json::Text("not a dict".to_owned()),
            Json::Number(serde_json::Number::from(7_i32)),
        ]
    );
}
