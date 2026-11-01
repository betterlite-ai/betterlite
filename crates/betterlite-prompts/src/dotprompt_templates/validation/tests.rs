#![cfg(test)]

use super::validate_input;
use crate::arguments::json::{Json, Object};
use rstest::rstest;

fn object(json: &str) -> Object {
    serde_json::from_str(json).unwrap()
}

fn int(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

fn float(value: f64) -> Json {
    Json::Number(serde_json::Number::from_f64(value).unwrap())
}

fn huge_int() -> Json {
    Json::Number(serde_json::from_str("18446744073709551616").unwrap())
}

fn overflowing_exponent() -> Json {
    Json::Number(serde_json::from_str("1e400").unwrap())
}

#[rstest]
#[case::string("string", Json::Text("a".to_owned()))]
#[case::str_alias("str", Json::Text("a".to_owned()))]
#[case::integer("integer", int(5))]
#[case::int_alias("int", int(5))]
#[case::integer_larger_than_u64("integer", huge_int())]
#[case::number_accepts_int("number", int(5))]
#[case::number_accepts_float("number", float(5.5))]
#[case::float("float", float(5.5))]
#[case::overflowing_exponent_satisfies_float("float", overflowing_exponent())]
#[case::boolean("boolean", Json::Bool(true))]
#[case::bool_alias("bool", Json::Bool(true))]
#[case::bool_satisfies_int("integer", Json::Bool(true))]
#[case::array("array", Json::List(vec![int(1), int(2)]))]
#[case::list_alias("list", Json::List(vec![int(1), int(2)]))]
#[case::object("object", Json::Object(object(r#"{"a": 1}"#)))]
#[case::dict_alias("dict", Json::Object(object(r#"{"a": 1}"#)))]
#[case::case_insensitive_type("STRING", Json::Text("a".to_owned()))]
#[case::unknown_type_defaults_to_str("mystery", Json::Text("a".to_owned()))]
fn accepts_matching_types(#[case] schema_type: &str, #[case] value: Json) {
    let schema = object(&format!(r#"{{"field": "{schema_type}"}}"#));
    let mut variables = Object::new();
    variables.insert("field".to_owned(), value);
    validate_input(&variables, &schema).unwrap();
}

#[rstest]
fn rejects_a_mismatched_type() {
    let schema = object(r#"{"age": "integer"}"#);
    let variables = object(r#"{"age": "not a number"}"#);
    validate_input(&variables, &schema).unwrap_err();
}

#[rstest]
fn a_float_does_not_satisfy_integer() {
    let schema = object(r#"{"age": "integer"}"#);
    let variables = object(r#"{"age": 5.5}"#);
    let error = validate_input(&variables, &schema).unwrap_err();
    assert!(error.to_string().contains("expected int, got float"));
}

#[rstest]
fn an_overflowing_exponent_does_not_satisfy_integer() {
    let schema = object(r#"{"n": "integer"}"#);
    let mut variables = Object::new();
    variables.insert("n".to_owned(), overflowing_exponent());
    let error = validate_input(&variables, &schema).unwrap_err();
    assert!(error.to_string().contains("expected int, got float"));
}

#[rstest]
fn an_integer_larger_than_u64_is_reported_as_int_in_a_mismatch_message() {
    let schema = object(r#"{"age": "string"}"#);
    let mut variables = Object::new();
    variables.insert("age".to_owned(), huge_int());
    let error = validate_input(&variables, &schema).unwrap_err();
    assert!(error.to_string().contains("expected str, got int"));
}

#[rstest]
fn an_int_does_not_satisfy_string() {
    let schema = object(r#"{"age": "string"}"#);
    let variables = object(r#"{"age": 5}"#);
    let error = validate_input(&variables, &schema).unwrap_err();
    assert!(error.to_string().contains("expected str, got int"));
}

#[rstest]
fn fields_absent_from_variables_are_not_checked() {
    let schema = object(r#"{"age": "integer"}"#);
    let variables = object("{}");
    validate_input(&variables, &schema).unwrap();
}

#[rstest]
fn a_non_string_schema_type_is_an_error() {
    let schema = object(r#"{"age": 1}"#);
    let variables = object(r#"{"age": 5}"#);
    validate_input(&variables, &schema).unwrap_err();
}
