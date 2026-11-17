use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use crate::api::responses_stream_primitives::python_text::value_text;

#[rstest]
#[case::a_string(Json::Text("slow".to_owned()), "slow")]
#[case::null(Json::Null, "None")]
#[case::bool_true(Json::Bool(true), "True")]
#[case::bool_false(Json::Bool(false), "False")]
#[case::an_integer(Json::Number(Number::from(7_i32)), "7")]
#[case::a_float(Json::Number(Number::from_f64(1.5).expect("1.5 is finite")), "1.5")]
#[case::an_array(
    Json::List(vec![Json::Text("a".to_owned()), Json::Null]),
    "['a', None]"
)]
#[case::an_object(
    Json::Object(IndexMap::from([(
        "a".to_owned(),
        Json::Number(Number::from(1_i32))
    )])),
    "{'a': 1}"
)]
fn value_text_matches_str(#[case] value: Json, #[case] expected: &str) {
    assert_eq!(value_text(&value), expected);
}
