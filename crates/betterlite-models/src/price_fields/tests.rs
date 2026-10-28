#![cfg(test)]

use super::PriceFields;
use rstest::rstest;
use serde_json::value::RawValue;
use std::fs;
use std::path::Path;

#[rstest]
#[case::fraction(r#"{"input_cost_per_token":0.000000123456789}"#, "0.000000123456789")]
#[case::exponent(r#"{"output_cost_per_token":1.2e-7}"#, "0.00000012")]
#[case::overprecision(
    r#"{"input_cost_per_token":0.12345678901234567890123456789}"#,
    "input_cost_per_token"
)]
#[case::overflow(
    r#"{"input_cost_per_token":999999999999999999999999999999999999999999999999}"#,
    "input_cost_per_token"
)]
#[case::overprecision_exponent(
    r#"{"input_cost_per_token":0.12345678901234567890123456789e-1}"#,
    "input_cost_per_token"
)]
#[case::overflow_exponent(
    r#"{"input_cost_per_token":999999999999999999999999999999e10}"#,
    "input_cost_per_token"
)]
#[case::nested(r#"{"provider_specific_entry":{"us":0.000000000000000001},"guardrail_cost_per_unit":{"contentPolicyUnits":0.0000001},"metadata":{"original_pricing_per_minute":0.020833333333333333},"sample_spec":{"computer_use_input_cost_per_1k_tokens":0.00001,"file_search_cost_per_1k_calls":0.003},"off_peak_pricing":{"input_cost_per_token":0.0000002,"windows":[{"hours_utc":"00:00-01:00","weekdays":["monday",2]}]},"search_context_cost_per_query":{"search_context_size_high":0.01},"tiered_pricing":[{"input_cost_per_token":0.00000005,"range":[0,256000]}]}"#, "nested")]
#[case::unknown_tier(r#"{"tiered_pricing":[{"input_cost_per_token":5e-8,"max_results_range":[0,25],"unknown_price":0.5}]}"#, "unknown field `unknown_price`")]
#[case::null(r#"{"input_cost_per_token":null}"#, "must not be null")]
#[case::string(r#"{"input_cost_per_token":"0.1"}"#, "must be a JSON number")]
#[case::boolean(r#"{"input_cost_per_token":true}"#, "must be a JSON number")]
#[case::array(r#"{"input_cost_per_token":[]}"#, "must be a JSON number")]
#[case::object(r#"{"input_cost_per_token":{}}"#, "must be a JSON number")]
fn model_price_types_price_field_tests_behavior(#[case] source: &str, #[case] expected: &str) {
    let raw: Result<Box<RawValue>, _> = serde_json::from_str(source);
    let parsed = match raw {
        Ok(value) => PriceFields::parse(value.as_ref()),
        Err(error) => panic!("test JSON must parse: {error}"),
    };
    if expected == "nested" {
        let parsed_value = parsed.unwrap();
        assert_eq!(
            parsed_value.provider_specific.us.unwrap().to_string(),
            "0.000000000000000001"
        );
        assert_eq!(parsed_value.guardrail_units.len(), 1);
        assert_eq!(
            parsed_value.guardrail_units["contentPolicyUnits"].to_string(),
            "0.0000001"
        );
        assert_eq!(
            parsed_value.metadata.per_minute.unwrap().to_string(),
            "0.020833333333333333"
        );
        assert_eq!(
            parsed_value
                .sample_spec
                .computer_use_input
                .unwrap()
                .to_string(),
            "0.00001"
        );
        assert_eq!(
            parsed_value
                .sample_spec
                .file_search_calls
                .unwrap()
                .to_string(),
            "0.003"
        );
        assert_eq!(
            parsed_value
                .off_peak
                .unwrap()
                .input_cost_per_token
                .unwrap()
                .to_string(),
            "0.0000002"
        );
        assert_eq!(
            parsed_value
                .search_context
                .unwrap()
                .search_context_size_high
                .unwrap()
                .to_string(),
            "0.01"
        );
        assert_eq!(parsed_value.tiers.len(), 1);
        assert_eq!(parsed_value.tiers[0].range.as_ref().unwrap().len(), 2);
        assert_eq!(
            parsed_value.tiers[0].range.as_ref().unwrap()[1].to_string(),
            "256000"
        );
    } else if expected.contains('_') || expected.contains("must") {
        assert!(parsed.unwrap_err().to_string().contains(expected));
    } else {
        let parsed_value = parsed.unwrap();
        assert_eq!(
            parsed_value
                .values
                .input_token
                .or(parsed_value.values.output_token)
                .unwrap()
                .to_string(),
            expected
        );
    }
}

#[rstest]
fn model_price_types_inventory_covers_pinned_fixtures() {
    for fixture in [
        "model_prices_and_context_window.json",
        "model_prices_and_context_window.schema.json",
    ] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(fixture);
        let source = fs::read_to_string(path).unwrap();
        let entries: std::collections::BTreeMap<String, Box<RawValue>> =
            serde_json::from_str(&source).unwrap();
        for (name, value) in entries {
            if name == "sample_spec" || name == "fallback_generalizations" {
                continue;
            }
            PriceFields::parse(value.as_ref()).unwrap();
        }
    }
}

#[rstest]
fn replays_the_price_fields_corpus() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/price_fields");
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        let source = fs::read_to_string(path).unwrap();
        let raw: Box<RawValue> = serde_json::from_str(&source).unwrap();
        drop(PriceFields::parse(raw.as_ref()));
    }
}
