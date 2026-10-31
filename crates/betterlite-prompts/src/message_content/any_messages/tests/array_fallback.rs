use super::super::*;
use serde_json::Number;

#[test]
fn message_content_behavior_tests_behavior() {
    // A non-mapping element fails the whole-array conversion, so every element falls back to JSON.
    let messages = Json::List(vec![
        Json::Text("a string element".to_owned()),
        Json::Number(Number::from(1_i64)),
        Json::Object(Object::from_iter([
            ("role".to_owned(), Json::Text("user".to_owned())),
            ("content".to_owned(), Json::Text("kept".to_owned())),
        ])),
    ]);
    let converted = to_str_messages(&messages);
    assert_eq!(
        converted,
        Json::List(vec![
            Json::Object(Object::from_iter([(
                "input".to_owned(),
                Json::Text("\"a string element\"".to_owned())
            )])),
            Json::Object(Object::from_iter([(
                "input".to_owned(),
                Json::Text("1".to_owned())
            )])),
            Json::Object(Object::from_iter([(
                "input".to_owned(),
                Json::Text("{\"role\":\"user\",\"content\":\"kept\"}".to_owned())
            )])),
        ])
    );
}
