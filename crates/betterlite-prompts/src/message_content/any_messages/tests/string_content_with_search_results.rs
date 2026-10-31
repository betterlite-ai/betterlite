use super::super::*;

#[test]
fn message_content_behavior_tests_behavior() {
    // A string content combined with search_results must append to the string, not replace it.
    let with_search_results = Json::List(vec![Json::Object(Object::from_iter([
        ("role".to_owned(), Json::Text("user".to_owned())),
        ("content".to_owned(), Json::Text("base".to_owned())),
        (
            "search_results".to_owned(),
            Json::List(vec![Json::Object(Object::from_iter([(
                "source".to_owned(),
                Json::Text("src".to_owned()),
            )]))]),
        ),
    ]))]);
    assert_eq!(
        to_str_messages(&with_search_results),
        Json::List(vec![Json::Object(Object::from_iter([
            ("role".to_owned(), Json::Text("user".to_owned())),
            ("content".to_owned(), Json::Text("basesrc".to_owned())),
            (
                "search_results".to_owned(),
                Json::List(vec![Json::Object(Object::from_iter([(
                    "source".to_owned(),
                    Json::Text("src".to_owned()),
                )]))]),
            ),
        ]))])
    );
}
