use super::super::*;
use rstest::rstest;
use serde_json::Number;

#[rstest]
#[case::dict(
    Json::Object(Object::from_iter([("role".to_owned(), Json::Text("user".to_owned()))])),
    "{\"role\":\"user\"}"
)]
#[case::string(Json::Text("raw".to_owned()), "raw")]
#[case::null(Json::Null, "null")]
#[case::bool_true(Json::Bool(true), "true")]
#[case::bool_false(Json::Bool(false), "false")]
#[case::number(Json::Number(Number::from(3_i64)), "3")]
fn message_content_behavior_tests_behavior(#[case] input: Json, #[case] expected: &str) {
    assert_eq!(
        to_str_messages(&input),
        Json::List(vec![Json::Object(Object::from_iter([(
            "input".to_owned(),
            Json::Text(expected.to_owned())
        )]))])
    );
}
