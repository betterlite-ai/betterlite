#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_env::EnvError;
use betterlite_models::tool_fees::prices::ToolPrices;
use rstest::rstest;
use rust_decimal::Decimal;

use super::{
    AZURE_COMPUTER_INPUT_ENV, AZURE_COMPUTER_OUTPUT_ENV, AZURE_FILE_SEARCH_ENV,
    AZURE_VECTOR_STORE_ENV, OPENAI_FILE_SEARCH_ENV, decode_tool_price, load_tool_prices,
};
use crate::loader::ConfigError;

const PINNED: ToolPrices = ToolPrices::PINNED;

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
        .collect()
}

fn money(units: i64, scale: u32) -> Decimal {
    Decimal::new(units, scale)
}

#[rstest]
#[case::nothing_set_uses_the_pinned_defaults(&[], PINNED)]
#[case::pinned_values_are_the_litellm_ones(
    &[
        (OPENAI_FILE_SEARCH_ENV, "0.0025"),
        (AZURE_FILE_SEARCH_ENV, "0.1"),
        (AZURE_VECTOR_STORE_ENV, "0.1"),
        (AZURE_COMPUTER_INPUT_ENV, "3"),
        (AZURE_COMPUTER_OUTPUT_ENV, "12"),
    ],
    PINNED
)]
#[case::openai_file_search_override(
    &[(OPENAI_FILE_SEARCH_ENV, "0.5")],
    ToolPrices { openai_search_per_1k: money(5, 1), ..PINNED }
)]
#[case::azure_file_search_override(
    &[(AZURE_FILE_SEARCH_ENV, "2")],
    ToolPrices { azure_search_per_gb_day: money(2, 0), ..PINNED }
)]
#[case::azure_vector_store_override(
    &[(AZURE_VECTOR_STORE_ENV, "0.25")],
    ToolPrices { azure_vector_per_gb_day: money(25, 2), ..PINNED }
)]
#[case::azure_computer_input_override(
    &[(AZURE_COMPUTER_INPUT_ENV, "7")],
    ToolPrices { azure_computer_input_per_1k: money(7, 0), ..PINNED }
)]
#[case::azure_computer_output_override(
    &[(AZURE_COMPUTER_OUTPUT_ENV, "9.5")],
    ToolPrices { azure_computer_output_per_1k: money(95, 1), ..PINNED }
)]
#[case::signed_and_exponent_values_pass(
    &[(OPENAI_FILE_SEARCH_ENV, "-1.5"), (AZURE_FILE_SEARCH_ENV, "+4"), (AZURE_VECTOR_STORE_ENV, "1e-3")],
    ToolPrices {
        openai_search_per_1k: money(-15, 1),
        azure_search_per_gb_day: money(4, 0),
        azure_vector_per_gb_day: money(1, 3),
        ..PINNED
    }
)]
#[case::an_explicit_zero_is_used(
    &[(AZURE_COMPUTER_INPUT_ENV, "0")],
    ToolPrices { azure_computer_input_per_1k: money(0, 0), ..PINNED }
)]
#[case::empty_and_blank_text_count_as_unset(
    &[(OPENAI_FILE_SEARCH_ENV, ""), (AZURE_FILE_SEARCH_ENV, "   ")],
    PINNED
)]
#[case::surrounding_whitespace_is_trimmed(
    &[(AZURE_COMPUTER_OUTPUT_ENV, " 8 ")],
    ToolPrices { azure_computer_output_per_1k: money(8, 0), ..PINNED }
)]
fn loads_builtin_tool_prices(#[case] pairs: &[(&str, &str)], #[case] expected: ToolPrices) {
    let loaded = load_tool_prices(&environment(pairs)).unwrap();
    assert_eq!(loaded, expected);
}

#[rstest]
#[case::words("abc")]
#[case::nan("nan")]
#[case::infinity("inf")]
#[case::hexadecimal("0x10")]
#[case::two_points("1.2.3")]
#[case::two_signs("--1")]
#[case::bare_exponent("1e")]
#[case::huge_exponent("1e99999999999999999999")]
#[case::beyond_the_decimal_range("1e40")]
#[case::underscore_separators("1_000")]
fn a_bad_price_names_the_variable_and_not_the_value(#[case] text: &str) {
    let error = load_tool_prices(&environment(&[(AZURE_VECTOR_STORE_ENV, text)])).unwrap_err();
    assert!(matches!(
        &error,
        ConfigError::Environment { source: EnvError::Number { name } } if name == AZURE_VECTOR_STORE_ENV
    ));
    assert!(!error.to_string().contains(text));
}

#[rstest]
#[case::empty("", None)]
#[case::blank(" \t ", None)]
#[case::plain("12", Some(money(12, 0)))]
#[case::fraction(".5", Some(money(5, 1)))]
#[case::plus_sign("+3", Some(money(3, 0)))]
#[case::negative("-0.25", Some(money(-25, 2)))]
#[case::upper_exponent("2E2", Some(money(200, 0)))]
fn decode_tool_price_reads_a_finite_decimal(#[case] text: &str, #[case] expected: Option<Decimal>) {
    assert_eq!(
        decode_tool_price(OPENAI_FILE_SEARCH_ENV, text).unwrap(),
        expected
    );
}

#[rstest]
fn replays_the_tool_prices_corpus(#[files("fuzz/corpus/tool_prices/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(text) = str::from_utf8(&bytes) else {
        return;
    };
    let decoded = decode_tool_price(OPENAI_FILE_SEARCH_ENV, text);
    if let Ok(Some(price)) = decoded {
        assert!(!text.trim().is_empty());
        assert_eq!(
            decode_tool_price(OPENAI_FILE_SEARCH_ENV, &price.to_string()).unwrap(),
            Some(price)
        );
    }
}
