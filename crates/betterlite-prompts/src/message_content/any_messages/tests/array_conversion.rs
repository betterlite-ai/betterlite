use super::super::*;

#[test]
fn message_content_behavior_tests_behavior() {
    let messages = Json::List(vec![
        Json::Object(Object::from_iter([
            ("role".to_owned(), Json::Text("user".to_owned())),
            (
                "content".to_owned(),
                Json::List(vec![Json::Object(Object::from_iter([
                    ("type".to_owned(), Json::Text("text".to_owned())),
                    ("text".to_owned(), Json::Text("hi".to_owned())),
                ]))]),
            ),
        ])),
        Json::Object(Object::from_iter([
            ("role".to_owned(), Json::Text("assistant".to_owned())),
            ("content".to_owned(), Json::Text("ok".to_owned())),
        ])),
    ]);
    let converted = to_str_messages(&messages);
    assert_eq!(
        converted,
        Json::List(vec![
            Json::Object(Object::from_iter([
                ("role".to_owned(), Json::Text("user".to_owned())),
                ("content".to_owned(), Json::Text("hi".to_owned())),
            ])),
            Json::Object(Object::from_iter([
                ("role".to_owned(), Json::Text("assistant".to_owned())),
                ("content".to_owned(), Json::Text("ok".to_owned())),
            ])),
        ])
    );
}
