use super::*;

fn wrap(inner: Json) -> Json {
    Json::Object(Object::from_iter([
        ("type".to_owned(), Json::Text("wrapper".to_owned())),
        ("content".to_owned(), Json::List(vec![inner])),
    ]))
}

fn value_to_part(value: Json) -> ContentPart {
    let Json::Object(fields) = value else {
        panic!("test fixture is always an object")
    };
    let kind = fields
        .get("type")
        .and_then(Json::as_str)
        .expect("type")
        .to_owned();
    part(&kind, fields)
}

#[test]
fn message_content_behavior_tests_behavior() {
    let image = Json::Object(Object::from_iter([
        ("type".to_owned(), Json::Text("image_url".to_owned())),
        (
            "image_url".to_owned(),
            Json::Text("https://deep".to_owned()),
        ),
    ]));
    let within_bound = value_to_part(wrap(wrap(wrap(image.clone()))));
    let beyond_bound = value_to_part(wrap(wrap(wrap(wrap(image)))));
    assert!(request_contains_image_content(&[message(vec![
        within_bound
    ])]));
    assert!(!request_contains_image_content(&[message(vec![
        beyond_bound
    ])]));
}
