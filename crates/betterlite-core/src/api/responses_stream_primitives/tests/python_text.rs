use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use crate::api::responses_stream_primitives::python_text::{float_text, value_text};

#[rstest]
#[case::a_whole_number(2.0, "2.0")]
#[case::a_fraction(2.5, "2.5")]
#[case::zero(0.0, "0.0")]
#[case::negative_zero(-0.0, "-0.0")]
#[case::a_negative_number(-1.0, "-1.0")]
#[case::the_smallest_fixed_fraction(0.0001, "0.0001")]
#[case::just_below_the_fixed_range(0.00001, "1e-05")]
#[case::fifteen_digits(1e15, "1000000000000000.0")]
#[case::sixteen_digits(1e16, "1e+16")]
#[case::a_large_number(1e20, "1e+20")]
#[case::nan(f64::NAN, "nan")]
#[case::infinity(f64::INFINITY, "inf")]
#[case::negative_infinity(f64::NEG_INFINITY, "-inf")]
fn float_text_matches_str(#[case] value: f64, #[case] expected: &str) {
    assert_eq!(float_text(value), expected);
}

#[rstest]
#[case::a_string(Json::Text("slow".to_owned()), "slow")]
#[case::null(Json::Null, "None")]
#[case::bool_true(Json::Bool(true), "True")]
#[case::bool_false(Json::Bool(false), "False")]
#[case::an_integer(Json::Number(Number::from(7)), "7")]
#[case::a_float(Json::Number(Number::from_f64(1.5).expect("1.5 is finite")), "1.5")]
#[case::an_array(
    Json::List(vec![Json::Text("a".to_owned()), Json::Null]),
    "['a', None]"
)]
#[case::an_object(
    Json::Object(IndexMap::from([(
        "a".to_owned(),
        Json::Number(Number::from(1))
    )])),
    "{'a': 1}"
)]
fn value_text_matches_str(#[case] value: Json, #[case] expected: &str) {
    assert_eq!(value_text(&value), expected);
}
