#![cfg(test)]

use betterlite_models::tiered_pricing::Scalar;
use rstest::rstest;
use serde::de::IgnoredAny;

use super::valid_price;
use crate::cost::prompt_cache_scenarios::test_support::number;
use crate::cost::prompt_cache_scenarios::test_support::text;

#[rstest]
#[case::zero("0")]
#[case::one("1")]
#[case::fraction("0.000000001")]
#[case::whole("123456789")]
fn a_non_negative_number_is_a_price(#[case] value: &str) {
    assert!(valid_price(Some(&number(value))));
}

#[rstest]
#[case::fraction("-0.5")]
#[case::whole("-123456789")]
fn a_negative_number_is_not_a_price(#[case] value: &str) {
    assert!(!valid_price(Some(&number(value))));
}

#[rstest]
#[case::boolean(Scalar::Bool(true))]
#[case::null(Scalar::Null)]
#[case::text(text("0.001"))]
#[case::array(Scalar::Array(Vec::new()))]
#[case::object(Scalar::Object(IgnoredAny))]
fn a_value_that_is_not_a_number_is_not_a_price(#[case] value: Scalar) {
    assert!(!valid_price(Some(&value)));
}

#[test]
fn an_absent_price_is_not_a_price() {
    assert!(!valid_price(None));
}
