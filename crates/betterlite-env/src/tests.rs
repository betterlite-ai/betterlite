#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::{EnvironmentLookup, flag, number};

#[rstest]
#[case::true_word(" TRUE ", Some(true))]
#[case::false_word(" FaLsE ", Some(false))]
#[case::one("1", Some(true))]
#[case::zero("0", Some(false))]
#[case::yes("YeS", Some(true))]
#[case::no("NO", Some(false))]
#[case::on("On", Some(true))]
#[case::off(" OFF ", Some(false))]
#[case::empty("", None)]
#[case::whitespace(" \t\n", None)]
fn reads_flags(#[case] text: &str, #[case] expected: Option<bool>) {
    assert_eq!(flag("FEATURE", text).unwrap(), expected);
}

#[rstest]
#[case::empty("", None)]
#[case::whitespace(" \t", None)]
#[case::decimal(" -1.25e2 ", Some(-125.0))]
#[case::positive("+3.5", Some(3.5))]
#[case::exponent("1E3", Some(1000.0))]
#[case::negative_zero("-0", Some(-0.0))]
fn reads_numbers(#[case] text: &str, #[case] expected: Option<f64>) {
    assert_eq!(number::<f64>("LIMIT", text).unwrap(), expected);
}

#[rstest]
#[case::unknown("maybe")]
#[case::abbreviation("t")]
#[case::underscore("1_000")]
#[case::infinity("inf")]
#[case::nan("NaN")]
#[case::hexadecimal("0x10")]
#[case::unicode_digit("١")]
#[case::zero_width_space("1\u{200b}")]
#[case::syntax("1e+")]
#[case::sign_only("+")]
fn rejects_invalid_values_without_disclosing_them(#[case] text: &str) {
    let number_error = number::<f64>("PRIVATE_LIMIT", text).unwrap_err();
    assert_eq!(
        number_error.to_string(),
        "environment variable PRIVATE_LIMIT must be a number"
    );
    let flag_error = flag("PRIVATE_FLAG", text).unwrap_err();
    assert_eq!(
        flag_error.to_string(),
        "environment variable PRIVATE_FLAG must be a bool"
    );
    assert!(!format!("{number_error:?} {flag_error:?}").contains(text));
}

#[test]
fn numbers_use_the_target_type() {
    assert_eq!(number::<i64>("SIGNED", " -42 ").unwrap(), Some(-42));
    assert_eq!(number::<u32>("UNSIGNED", " +42 ").unwrap(), Some(42));
    assert!(number::<u32>("UNSIGNED", "-1").is_err());
    assert!(number::<i64>("SIGNED", "1.5").is_err());
    assert!(number::<i64>("SIGNED", "1e3").is_err());
    assert!(number::<u32>("UNSIGNED", "4294967296").is_err());
    assert_eq!(number::<i64>("SIGNED", "").unwrap(), None);
}

#[test]
fn map_lookup_distinguishes_missing_and_empty() {
    let environment = BTreeMap::from([("EMPTY".to_owned(), String::new())]);
    assert_eq!(environment.lookup("EMPTY"), Some(String::new()));
    assert_eq!(environment.lookup("MISSING"), None);
    assert_eq!(environment.system_home(), None);
}

#[rstest]
fn replays_the_env_values_corpus(#[files("fuzz/corpus/env_values/*")] path: PathBuf) {
    let text = fs::read_to_string(path).unwrap();
    drop(flag("CORPUS", &text));
    drop(number::<i64>("CORPUS", &text));
    drop(number::<u32>("CORPUS", &text));
    drop(number::<f64>("CORPUS", &text));
}
