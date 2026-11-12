#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;

use super::{
    FileSearchParams, TokenCounts, extract_file_search_params, extract_token_counts,
    safe_convert_to_int,
};
use crate::decisions::Json;
use crate::tool_signals::error::{QuantityField, ToolSignalError};

fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("the fixture is valid JSON")
}

fn decimal(text: &str) -> Decimal {
    text.parse().expect("the test literal is a decimal")
}

#[rstest]
#[case::numbers(r#"{"storage_gb":2.5,"days":3}"#, Some("2.5"), Some("3"))]
#[case::numeric_strings(r#"{"storage_gb":"2.5","days":"3"}"#, Some("2.5"), Some("3"))]
#[case::invalid_day(r#"{"storage_gb":2.5,"days":"abc"}"#, Some("2.5"), None)]
#[case::null_storage(r#"{"storage_gb":null,"days":true}"#, None, Some("1"))]
#[case::containers(r#"{"storage_gb":[1],"days":{}}"#, None, None)]
#[case::missing("{}", None, None)]
fn storage_and_days_convert_independently(
    #[case] source: &str,
    #[case] storage: Option<&str>,
    #[case] days: Option<&str>,
) {
    let params = extract_file_search_params(Some(&json(source))).expect("the quantities convert");
    assert_eq!(
        params,
        FileSearchParams {
            storage_gb: storage.map(decimal),
            days: days.map(decimal),
        }
    );
}

#[rstest]
#[case::null("null")]
#[case::text(r#""not an object""#)]
#[case::array("[]")]
fn a_non_mapping_file_search_usage_yields_no_quantities(#[case] source: &str) {
    assert_eq!(
        extract_file_search_params(Some(&json(source))).expect("a non-mapping is ignored"),
        FileSearchParams::default()
    );
}

#[test]
fn an_absent_file_search_usage_yields_no_quantities() {
    assert_eq!(
        extract_file_search_params(None).expect("an absent usage is ignored"),
        FileSearchParams::default()
    );
}

#[rstest]
#[case::exponent_overflow("1e400", QuantityField::StorageGb)]
#[case::text_infinity(r#""inf""#, QuantityField::StorageGb)]
#[case::text_nan(r#""nan""#, QuantityField::Days)]
fn non_finite_quantities_are_typed_errors(#[case] source: &str, #[case] field: QuantityField) {
    let key = match field {
        QuantityField::StorageGb => "storage_gb",
        QuantityField::Days => "days",
    };
    let usage = json(&format!(r#"{{"{key}":{source}}}"#));
    let error = extract_file_search_params(Some(&usage)).expect_err("a non-finite quantity fails");
    assert_eq!(error, ToolSignalError::NonFiniteQuantity { field });
    assert_eq!(
        error.to_string(),
        format!("the {key} file-search quantity is not a finite number")
    );
}

#[test]
fn a_quantity_past_the_decimal_range_is_a_typed_error() {
    let usage = json(r#"{"storage_gb":1e300}"#);
    assert_eq!(
        extract_file_search_params(Some(&usage)),
        Err(ToolSignalError::QuantityOutOfRange {
            field: QuantityField::StorageGb
        })
    );
}

#[rstest]
#[case::numbers(r#"{"input_tokens":10,"output_tokens":7}"#, Some(10), Some(7))]
#[case::numeric_strings(r#"{"input_tokens":"10","output_tokens":"7"}"#, Some(10), Some(7))]
#[case::floats_truncate(r#"{"input_tokens":3.9,"output_tokens":-2.7}"#, Some(3), Some(-2))]
#[case::booleans(r#"{"input_tokens":true,"output_tokens":false}"#, Some(1), Some(0))]
#[case::null_stays_absent(r#"{"input_tokens":null,"output_tokens":null}"#, None, None)]
#[case::containers(r#"{"input_tokens":[1],"output_tokens":"abc"}"#, None, None)]
#[case::missing("{}", None, None)]
fn token_counts_read_each_field_with_int_rules(
    #[case] source: &str,
    #[case] input: Option<i64>,
    #[case] output: Option<i64>,
) {
    assert_eq!(
        extract_token_counts(Some(&json(source))).expect("the counts convert"),
        TokenCounts {
            input_tokens: input,
            output_tokens: output,
        }
    );
}

#[rstest]
#[case::null("null")]
#[case::text(r#""abc""#)]
fn a_non_mapping_computer_use_usage_yields_no_counts(#[case] source: &str) {
    assert_eq!(
        extract_token_counts(Some(&json(source))).expect("a non-mapping is ignored"),
        TokenCounts::default()
    );
    assert_eq!(
        extract_token_counts(None).expect("an absent usage is ignored"),
        TokenCounts::default()
    );
}

#[rstest]
#[case::integer("3", Some(3))]
#[case::negative("-3", Some(-3))]
#[case::explicit_plus(r#""+3""#, Some(3))]
#[case::underscored(r#"" 1_0 ""#, Some(10))]
#[case::zero("-0", Some(0))]
#[case::integer_float("3.0", Some(3))]
#[case::truncated_float("3.9", Some(3))]
#[case::negative_float("-2.7", Some(-2))]
#[case::boolean("true", Some(1))]
#[case::null("null", None)]
#[case::array("[]", None)]
#[case::object("{}", None)]
#[case::decimal_string(r#""3.0""#, None)]
#[case::empty_text(r#""""#, None)]
#[case::blank_text(r#""  ""#, None)]
#[case::invalid_text(r#""abc""#, None)]
#[case::signed_text(r#""-1_0""#, Some(-10))]
#[case::non_ascii_digits("\"\\u0661\\u0662\"", None)]
fn int_conversion_follows_the_source_rules(#[case] source: &str, #[case] expected: Option<i64>) {
    assert_eq!(
        safe_convert_to_int(&json(source)).expect("the value converts"),
        expected
    );
}

#[rstest]
#[case::huge_integer("9223372036854775808")]
#[case::huge_float("1e19")]
#[case::overflowing_float("1e400")]
#[case::huge_text(r#""999999999999999999999999""#)]
fn counters_past_the_signed_range_are_typed_errors(#[case] source: &str) {
    assert_eq!(
        safe_convert_to_int(&json(source)),
        Err(ToolSignalError::CounterOutOfRange)
    );
}
