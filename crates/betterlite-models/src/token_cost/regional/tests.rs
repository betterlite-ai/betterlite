#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;

use super::{provider_geo_multiplier, regional_uplift_multiplier, vertex_endpoint_uplift};
use crate::price_lookup::PriceRecord;
use crate::tiered_pricing::Scalar;
use crate::token_rates::ModelInfo;

fn model_with(key: &str, value: Scalar) -> ModelInfo {
    let mut model = ModelInfo::new();
    model.insert(key.to_owned(), value);
    model
}

fn number(text: &str) -> Scalar {
    Scalar::Number(
        Number::from_f64(text.parse().expect("test literal is numeric")).expect("finite"),
    )
}

#[rstest]
#[case::absent(None, Decimal::ONE)]
#[case::unrecognized_region(Some("ca"), Decimal::ONE)]
#[case::uppercase_not_trimmed(Some("US"), Decimal::ONE)]
#[case::padded_not_trimmed(Some(" us"), Decimal::ONE)]
fn regional_uplift_defaults_to_one_without_a_matching_model_entry(
    #[case] residency: Option<&str>,
    #[case] expected: Decimal,
) {
    let model = ModelInfo::new();
    assert_eq!(regional_uplift_multiplier(&model, residency), expected);
}

#[test]
fn regional_uplift_applies_the_lowercased_residency_key() {
    let model = model_with("regional_processing_uplift_multiplier_us", number("1.25"));
    assert_eq!(
        regional_uplift_multiplier(&model, Some("us")),
        Decimal::new(125, 2)
    );
}

#[test]
fn regional_uplift_accepts_a_boolean_multiplier() {
    let model = model_with(
        "regional_processing_uplift_multiplier_eu",
        Scalar::Bool(true),
    );
    assert_eq!(regional_uplift_multiplier(&model, Some("eu")), Decimal::ONE);
}

#[test]
fn regional_uplift_warns_and_defaults_on_a_malformed_multiplier() {
    let model = model_with(
        "regional_processing_uplift_multiplier_us",
        Scalar::Text("not-a-number".to_owned()),
    );
    assert_eq!(regional_uplift_multiplier(&model, Some("us")), Decimal::ONE);
}

#[rstest]
#[case::absent(None, Decimal::ONE)]
#[case::lowercase_global(Some("global"), Decimal::ONE)]
#[case::mixed_case_global(Some("GLOBAL"), Decimal::ONE)]
fn vertex_uplift_skips_the_global_endpoint(
    #[case] location: Option<&str>,
    #[case] expected: Decimal,
) {
    let model = model_with("regional_endpoint_uplift_multiplier", number("2"));
    assert_eq!(vertex_endpoint_uplift(&model, location), expected);
}

#[test]
fn vertex_uplift_applies_the_multiplier_for_an_empty_location() {
    let model = model_with("regional_endpoint_uplift_multiplier", number("1.5"));
    assert_eq!(
        vertex_endpoint_uplift(&model, Some("")),
        Decimal::new(15, 1)
    );
}

#[test]
fn vertex_uplift_warns_and_defaults_on_a_malformed_multiplier() {
    let model = model_with(
        "regional_endpoint_uplift_multiplier",
        Scalar::Array(Vec::new()),
    );
    assert_eq!(
        vertex_endpoint_uplift(&model, Some("us-east1")),
        Decimal::ONE
    );
}

#[rstest]
#[case::absent_geo(None, "not_available")]
#[case::global(Some("global"), "not_available")]
#[case::not_available(Some("not_available"), "not_available")]
fn provider_geo_multiplier_skips_global_and_not_available(
    #[case] geo: Option<&str>,
    #[case] entry_key: &str,
) {
    let mut entry = PriceRecord::new();
    entry.insert(entry_key.to_owned(), Scalar::Null);
    let result = provider_geo_multiplier(Some(&entry), geo).expect("skipped before lookup");
    assert_eq!(result, Decimal::ONE);
}

#[test]
fn provider_geo_multiplier_defaults_a_missing_entry_to_one() {
    let entry: PriceRecord = BTreeMap::new();
    let result = provider_geo_multiplier(Some(&entry), Some("eu-west")).expect("no entry present");
    assert_eq!(result, Decimal::ONE);
}

#[test]
fn provider_geo_multiplier_reads_the_lowercased_geo_key() {
    let mut entry = PriceRecord::new();
    entry.insert("eu-west".to_owned(), number("1.1"));
    let result = provider_geo_multiplier(Some(&entry), Some("EU-WEST")).expect("valid price");
    assert_eq!(result, Decimal::new(11, 1));
}

#[test]
fn provider_geo_multiplier_propagates_an_error_for_an_invalid_present_price() {
    let mut entry = PriceRecord::new();
    entry.insert("eu-west".to_owned(), Scalar::Null);
    let result = provider_geo_multiplier(Some(&entry), Some("eu-west"));
    assert!(
        result.is_err(),
        "a present but invalid price must not default"
    );
}

#[test]
fn provider_geo_multiplier_without_a_provider_entry_defaults_to_one() {
    let result = provider_geo_multiplier(None, Some("eu-west")).expect("no entry to read");
    assert_eq!(result, Decimal::ONE);
}
