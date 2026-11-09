use betterlite_models::decisions::Json;
use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{row, row_json};

fn decimal_of(value: &Json) -> Decimal {
    match value {
        Json::Number(number) => number.to_string().parse().unwrap(),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            panic!("{value:?} is not a JSON number")
        }
    }
}

#[test]
fn response_cost_and_saved_cache_cost_stay_decimals() {
    let row = row(r#"{"response_cost": 0.0015, "saved_cache_cost": 0.0003}"#);
    assert_eq!(row.response_cost, Some("0.0015".parse().unwrap()));
    assert_eq!(row.saved_cache_cost, Some("0.0003".parse().unwrap()));
}

#[test]
fn an_explicit_zero_cost_is_kept_as_zero() {
    let row = row(r#"{"response_cost": 0.0, "saved_cache_cost": 0}"#);
    assert_eq!(row.response_cost, Some(Decimal::ZERO));
    assert_eq!(row.saved_cache_cost, Some(Decimal::ZERO));
}

#[test]
fn a_cost_is_written_as_a_json_number_with_every_decimal_digit() {
    let text = "1.100000000000000088817841970";
    let row = row(&format!(r#"{{"response_cost": {text}}}"#));
    let cost: Decimal = text.parse().unwrap();

    assert_eq!(row.response_cost, Some(cost));

    let json = row_json(&row);
    assert!(matches!(json["response_cost"], Json::Number(_)));
    assert_eq!(decimal_of(&json["response_cost"]), cost);
}

#[rstest]
#[case::boolean("true")]
#[case::text(r#""0.5""#)]
#[case::object(r#"{"amount": 1}"#)]
#[case::null("null")]
fn a_non_numeric_cost_is_null(#[case] value: &str) {
    let row = row(&format!(
        r#"{{"response_cost": {value}, "saved_cache_cost": {value}}}"#
    ));
    assert_eq!(row.response_cost, None);
    assert_eq!(row.saved_cache_cost, None);
}

#[test]
fn a_missing_cost_is_null() {
    let row = row("{}");
    assert_eq!(row.response_cost, None);
    assert_eq!(row.saved_cache_cost, None);
}
