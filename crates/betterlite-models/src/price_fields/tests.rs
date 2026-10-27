#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::value::RawValue;
use std::fs;
use std::path::Path;

use super::PriceFields;

#[rstest]
#[case::fraction(r#"{"input_cost_per_token":0.000000123456789}"#, "0.000000123456789")]
#[case::exponent(r#"{"output_cost_per_token":1.2e-7}"#, "0.00000012")]
fn model_price_types_price_field_tests_behavior(#[case] source: &str, #[case] expected: &str) {
    let raw: Box<RawValue> = serde_json::from_str(source).unwrap();
    let parsed = PriceFields::parse(raw.as_ref()).unwrap();
    let values = parsed.values.values().next().unwrap();
    assert_eq!(values.values().next().unwrap().to_string(), expected);
}

#[test]
fn model_price_types_price_field_tests_behavior_preserves_nested_prices() {
    let raw: Box<RawValue> = serde_json::from_str(
        r#"{
            "provider_specific_entry":{"us":0.000000000000000001,"fast":2e-8},
            "guardrail_cost_per_unit":{"image":0.0000000000000000001},
            "metadata":{"original_pricing_per_hour":1.25,"original_pricing_per_minute":0.020833333333333333},
            "sample_spec":{"computer_use_input_cost_per_1k_tokens":0.00001,"file_search_cost_per_1k_calls":0.003},
            "off_peak_pricing":{"input_cost_per_token":0.0000002}
        }"#,
    )
    .unwrap();
    let parsed = PriceFields::parse(raw.as_ref()).unwrap();
    assert_eq!(
        parsed.provider_specific.us,
        Some(Decimal::from_str_exact("0.000000000000000001").unwrap())
    );
    assert_eq!(
        parsed.guardrail_units["image"],
        Decimal::from_str_exact("0.0000000000000000001").unwrap()
    );
    assert_eq!(
        parsed.metadata.per_hour,
        Some(Decimal::from_str_exact("1.25").unwrap())
    );
    assert_eq!(
        parsed.metadata.per_minute,
        Some(Decimal::from_str_exact("0.020833333333333333").unwrap())
    );
    assert_eq!(
        parsed.sample_spec.file_search_calls,
        Some(Decimal::new(3, 3))
    );
    assert_eq!(
        parsed.off_peak.unwrap().values["input_cost_per_token"],
        Decimal::new(2, 7)
    );
}

#[test]
fn model_price_types_price_field_tests_behavior_preserves_tiers_and_search_context() {
    let raw: Box<RawValue> = serde_json::from_str(
        r#"{
            "search_context_cost_per_query":{"search_context_size_high":0.01,"search_context_size_low":0.005},
            "tiered_pricing":[{"input_cost_per_token":5e-8,"output_cost_per_token":4e-7,"unknown_price":0.5,"range":[0,256000.0]}]
        }"#,
    )
    .unwrap();
    let parsed = PriceFields::parse(raw.as_ref()).unwrap();
    assert_eq!(
        parsed.search_context["search_context_size_high"],
        Decimal::new(1, 2)
    );
    assert_eq!(
        parsed.tiers[0].values["input_cost_per_token"],
        Decimal::new(5, 8)
    );
    assert_eq!(parsed.tiers[0].range[1], Decimal::from(256_000_i32));
    assert!(!parsed.tiers[0].values.contains_key("unknown_price"));
}

#[rstest]
#[case::null(
    r#"{"input_cost_per_token":null}"#,
    "price field input_cost_per_token must not be null"
)]
#[case::string(
    r#"{"input_cost_per_token":"0.1"}"#,
    "price field input_cost_per_token must be a JSON number"
)]
#[case::true_value(
    r#"{"input_cost_per_token":true}"#,
    "price field input_cost_per_token must be a JSON number"
)]
#[case::false_value(
    r#"{"input_cost_per_token":false}"#,
    "price field input_cost_per_token must be a JSON number"
)]
#[case::object(
    r#"{"input_cost_per_token":{}}"#,
    "price field input_cost_per_token must be a JSON number"
)]
#[case::malformed(r#"{"input_cost_per_token":1e}"#, "invalid number at line 1 column 27")]
fn model_price_types_price_field_tests_behavior_rejects_bad_values(
    #[case] source: &str,
    #[case] expected: &str,
) {
    let raw: Result<Box<RawValue>, _> = serde_json::from_str(source);
    let result = raw
        .map_err(|error| error.to_string())
        .and_then(|value| PriceFields::parse(value.as_ref()).map_err(|error| error.to_string()));
    assert_eq!(result.unwrap_err(), expected);
}

#[rstest]
#[case::empty("empty.json")]
#[case::malformed("malformed.json")]
fn replays_the_price_fields_corpus(#[case] file: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz/corpus/price_fields")
        .join(file);
    let source = fs::read_to_string(path).unwrap();
    let raw: Box<RawValue> = serde_json::from_str(&source).unwrap();
    drop(PriceFields::parse(raw.as_ref()));
}
