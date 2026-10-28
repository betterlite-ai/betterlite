#![cfg(test)]

use super::PriceFieldError;
use super::PriceFields;
use super::nested::Weekday;
use rstest::rstest;
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[rstest]
#[case::absent("{}", "absent")]
#[case::exact_digits(
    r#"{"input_cost_per_token":0.0000001234567890123456789}"#,
    "0.0000001234567890123456789"
)]
#[case::positive_exponent(r#"{"input_cost_per_token":1.25e2}"#, "125")]
#[case::maximum_scale(r#"{"input_cost_per_token":1e-28}"#, "0.0000000000000000000000000001")]
#[case::underflow(r#"{"input_cost_per_token":1e-29}"#, "input_cost_per_token")]
#[case::secondary_null(r#"{"annotation_cost_per_page":null}"#, "annotation_cost_per_page")]
#[case::nested_precision(
    r#"{"guardrail_cost_per_unit":{"custom":1e-29}}"#,
    "guardrail_cost_per_unit.custom"
)]
#[case::metadata_null(
    r#"{"metadata":{"original_pricing_per_hour":null}}"#,
    "metadata.original_pricing_per_hour"
)]
#[case::search_null(
    r#"{"search_context_cost_per_query":{"search_context_size_low":null}}"#,
    "search_context_cost_per_query.search_context_size_low"
)]
#[case::off_peak_null(
    r#"{"off_peak_pricing":{"input_cost_per_token":null}}"#,
    "off_peak_pricing.input_cost_per_token"
)]
#[case::tier_null(
    r#"{"tiered_pricing":[{"input_cost_per_query":null}]}"#,
    "tiered_pricing[0].input_cost_per_query"
)]
#[case::duplicate_unit(
    r#"{"guardrail_cost_per_unit":{"custom":1,"custom":2}}"#,
    "duplicate key"
)]
#[case::expanded_exponent(r#"{"input_cost_per_token":1.25e3}"#, "1250")]
#[case::expanded_overflow(r#"{"input_cost_per_token":1e29}"#, "input_cost_per_token")]
#[case::unbounded_exponent(
    r#"{"input_cost_per_token":1e999999999999999999999}"#,
    "input_cost_per_token"
)]
#[case::false_boolean(r#"{"input_cost_per_token":false}"#, "input_cost_per_token")]
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
#[case::extreme_exponent(r#"{"input_cost_per_token":1e-2147483648}"#, "input_cost_per_token")]
#[case::nested_null(
    r#"{"provider_specific_entry":{"us":null}}"#,
    "provider_specific_entry.us"
)]
#[case::null(r#"{"input_cost_per_token":null}"#, "input_cost_per_token")]
#[case::string(r#"{"input_cost_per_token":"0.1"}"#, "input_cost_per_token")]
#[case::boolean(r#"{"input_cost_per_token":true}"#, "input_cost_per_token")]
#[case::array(r#"{"input_cost_per_token":[]}"#, "input_cost_per_token")]
#[case::object(r#"{"input_cost_per_token":{}}"#, "input_cost_per_token")]
#[case::weekday_rejects_unknown_number(
    r#"{"off_peak_pricing":{"windows":[{"hours_utc":"00:00-01:00","weekdays":[0]}]}}"#,
    "off_peak_pricing.windows[0].weekdays[0]"
)]
#[case::weekday_rejects_unknown_name(
    r#"{"off_peak_pricing":{"windows":[{"hours_utc":"00:00-01:00","weekdays":["funday"]}]}}"#,
    "off_peak_pricing.windows[0].weekdays[0]"
)]
fn model_price_types_price_field_tests_behavior(#[case] source: &str, #[case] expected: &str) {
    let raw: Result<Box<RawValue>, _> = serde_json::from_str(source);
    let parsed = match raw {
        Ok(value) => PriceFields::parse(value.as_ref()),
        Err(error) => panic!("test JSON must parse: {error}"),
    };
    let decoded = serde_json::from_str::<PriceFields>(source);
    assert_eq!(decoded.as_ref().ok(), parsed.as_ref().ok());
    if expected == "absent" {
        assert_eq!(parsed.unwrap(), PriceFields::default());
    } else if expected == "nested" {
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
    } else if expected
        .chars()
        .any(|character| character.is_ascii_alphabetic())
    {
        let error = parsed.unwrap_err();
        if expected.contains("duplicate") || expected.contains("unknown field") {
            assert!(error.to_string().contains(expected));
        } else {
            let PriceFieldError::Json { path, .. } = error;
            assert_eq!(path, expected);
        }
    } else {
        let numbers: BTreeMap<String, Box<RawValue>> = serde_json::from_str(source).unwrap();
        let number = numbers.values().next().unwrap();
        assert_eq!(
            super::decimal_lexeme::parse_decimal_lexeme(number.get())
                .unwrap()
                .to_string(),
            expected
        );
        let parsed_value = parsed.unwrap();
        assert_eq!(
            parsed_value
                .values
                .input
                .input_token
                .or(parsed_value.values.output.output_token)
                .unwrap()
                .to_string(),
            expected
        );
    }
}

#[path = "inventory/tests.rs"]
mod inventory;

#[rstest]
#[case::flat_null(r#"{"input_cost_per_token":null}"#)]
#[case::flat_overprecision(r#"{"input_cost_per_token":0.12345678901234567890123456789}"#)]
fn rejects_invalid_flat_prices(#[case] source: &str) {
    let raw: Box<RawValue> = serde_json::from_str(source).unwrap();
    PriceFields::parse(raw.as_ref()).unwrap_err();
}

#[rstest]
#[case::null(r#"{"off_peak_pricing":{"input_cost_per_token":null}}"#)]
#[case::overprecision(
    r#"{"off_peak_pricing":{"input_cost_per_token":0.12345678901234567890123456789}}"#
)]
fn rejects_invalid_nested_prices(#[case] source: &str) {
    let raw: Box<RawValue> = serde_json::from_str(source).unwrap();
    let error = PriceFields::parse(raw.as_ref()).unwrap_err();
    assert!(error.to_string().contains("input_cost_per_token"));
}

#[test]
fn off_peak_parses_weekday_timezone() {
    let off_peak = serde_json::from_str::<super::nested::OffPeakPricing>(
        r#"{"weekday_timezone":"Asia/Shanghai"}"#,
    )
    .unwrap();
    assert_eq!(off_peak.weekday_timezone, Some("Asia/Shanghai".to_owned()));
}

#[test]
fn price_window_treats_absent_weekdays_as_every_day() {
    let window =
        serde_json::from_str::<super::nested::PriceWindow>(r#"{"hours_utc":"00:00-01:00"}"#)
            .unwrap();
    assert_eq!(window.weekdays, None);
}

#[rstest]
#[case::number_monday("1", Weekday::Monday)]
#[case::number_sunday("7", Weekday::Sunday)]
#[case::name_monday(r#""MON""#, Weekday::Monday)]
#[case::name_monday_padded(r#"" Monday ""#, Weekday::Monday)]
#[case::name_tuesday(r#""tues""#, Weekday::Tuesday)]
#[case::name_wednesday(r#""Wednesday""#, Weekday::Wednesday)]
#[case::name_thursday(r#""thur""#, Weekday::Thursday)]
#[case::name_friday(r#""Fri""#, Weekday::Friday)]
#[case::name_saturday(r#""SATURDAY""#, Weekday::Saturday)]
#[case::name_sunday(r#""sun""#, Weekday::Sunday)]
fn weekday_accepts_schema_representations(#[case] source: &str, #[case] expected: Weekday) {
    assert_eq!(serde_json::from_str::<Weekday>(source).unwrap(), expected);
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
