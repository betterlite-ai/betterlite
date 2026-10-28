#![cfg(test)]

use super::Op;
use crate::helpers::Helper as _;
use rstest::rstest;
use serde_json::value::RawValue;

#[rstest]
#[case::direct(r#"{"input_cost_per_token":0.1000}"#, "/input_cost_per_token", "0.1")]
#[case::source(
    r#"{"source":"{\"input_cost_per_token\":0.0000001234567890123456789}"}"#,
    "/input_cost_per_token",
    "0.0000001234567890123456789"
)]
#[case::tier(
    r#"{"tiered_pricing":[{"range":[0.0,2.0]}]}"#,
    "/tiered_pricing/0/range/1",
    "2"
)]
#[case::hours(
    r#"{"off_peak_pricing":{"hours_utc":"00"}}"#,
    "/off_peak_pricing/hours_utc",
    "00"
)]
fn canonical_projection(#[case] source: &str, #[case] path: &str, #[case] expected: &str) {
    let input: Box<RawValue> = serde_json::from_str(source).unwrap();
    let output = Op::run(&input).unwrap();
    assert_eq!(output.pointer(path).unwrap().as_str(), Some(expected));
}

#[rstest]
#[case::invalid_source(r#"{"source":"invalid"}"#)]
#[case::null_price(r#"{"input_cost_per_token":null}"#)]
fn rejects_invalid_prices(#[case] source: &str) {
    let input: Box<RawValue> = serde_json::from_str(source).unwrap();
    let error = Op::run(&input).unwrap_err();
    assert_eq!(error.class, "PriceFieldError");
}
