#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_env::EnvError;
use rstest::rstest;
use rust_decimal::Decimal;

use super::{REPLICATE_PRICE_ENV, decode_replicate_price, load_replicate_gpu_price};
use crate::loader::ConfigError;

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
#[case::nothing_set_uses_litellms_default(&[], money(14, 4))]
#[case::an_override_replaces_the_default(&[(REPLICATE_PRICE_ENV, "0.002")], money(2, 3))]
#[case::an_explicit_zero_is_used(&[(REPLICATE_PRICE_ENV, "0")], money(0, 0))]
#[case::a_negative_price_is_kept(&[(REPLICATE_PRICE_ENV, "-0.5")], money(-5, 1))]
#[case::empty_text_counts_as_unset(&[(REPLICATE_PRICE_ENV, "")], money(14, 4))]
#[case::blank_text_counts_as_unset(&[(REPLICATE_PRICE_ENV, "  ")], money(14, 4))]
#[case::exponent_text_is_read(&[(REPLICATE_PRICE_ENV, "1.4e-3")], money(14, 4))]
fn loads_replicate_gpu_price(#[case] pairs: &[(&str, &str)], #[case] expected: Decimal) {
    assert_eq!(
        load_replicate_gpu_price(&environment(pairs)).unwrap(),
        expected
    );
}

#[rstest]
#[case::words("abc")]
#[case::nan("nan")]
#[case::infinity("-inf")]
#[case::two_points("1.2.3")]
#[case::beyond_the_decimal_range("1e40")]
#[case::separators("1_0")]
fn a_bad_replicate_price_names_the_variable_and_not_the_value(#[case] text: &str) {
    let error = load_replicate_gpu_price(&environment(&[(REPLICATE_PRICE_ENV, text)])).unwrap_err();
    assert!(matches!(
        &error,
        ConfigError::Environment { source: EnvError::Number { name } } if name == REPLICATE_PRICE_ENV
    ));
    assert!(!error.to_string().contains(text));
}

#[rstest]
#[case::empty("", None)]
#[case::plain("12", Some(money(12, 0)))]
#[case::fraction(".5", Some(money(5, 1)))]
#[case::plus_sign("+3", Some(money(3, 0)))]
#[case::negative("-0.25", Some(money(-25, 2)))]
#[case::upper_exponent("2E2", Some(money(200, 0)))]
fn decode_replicate_price_reads_a_finite_decimal(
    #[case] text: &str,
    #[case] expected: Option<Decimal>,
) {
    assert_eq!(decode_replicate_price(text).unwrap(), expected);
}

#[rstest]
fn replays_the_replicate_price_corpus(#[files("fuzz/corpus/replicate_price/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(text) = str::from_utf8(&bytes) else {
        return;
    };
    if let Ok(Some(price)) = decode_replicate_price(text) {
        assert!(!text.trim().is_empty());
        assert_eq!(
            decode_replicate_price(&price.to_string()).unwrap(),
            Some(price)
        );
    }
}
