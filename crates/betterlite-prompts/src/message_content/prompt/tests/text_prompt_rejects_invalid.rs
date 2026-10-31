use super::super::*;
use crate::arguments::json::Object;
use rstest::rstest;
use serde_json::Number;

#[rstest]
#[case::empty_string(Json::Text(String::new()), "str")]
#[case::int_tokens(
    Json::List(vec![Json::Number(Number::from(1_i64)), Json::Number(Number::from(2_i64))]),
    "list"
)]
#[case::mixed(Json::List(vec![Json::Text("ok".to_owned()), Json::Text(String::new())]), "list")]
#[case::int(Json::Number(Number::from(7_i64)), "int")]
#[case::float(Json::Number(Number::from_f64(7.5).expect("finite")), "float")]
#[case::bool_value(Json::Bool(true), "bool")]
#[case::null(Json::Null, "NoneType")]
#[case::dict(Json::Object(Object::from_iter([("a".to_owned(), Json::Number(Number::from(1_i64)))])), "dict")]
fn message_content_behavior_tests_behavior(#[case] prompt: Json, #[case] prompt_type: &str) {
    assert_eq!(
        to_messages(&prompt).unwrap_err(),
        ContentError::InvalidPrompt {
            prompt_type: prompt_type.to_owned()
        }
    );
}
