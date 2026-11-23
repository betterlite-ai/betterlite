use std::fs;
use std::path::PathBuf;
use std::str::from_utf8;

use chrono::NaiveDate;
use rstest::rstest;

use super::super::dates::parse_legacy_date;
use super::super::query::{LegacyQueryError, LegacySpendQuery, decode_legacy_spend_query};

// every corpus file is a raw query string the decoder must answer without panicking
#[rstest]
fn replays_the_legacy_spend_query_corpus(
    #[files("fuzz/corpus/legacy_spend_query/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let raw = from_utf8(&bytes).unwrap();
    drop(decode_legacy_spend_query(raw.as_bytes()));
}

#[test]
fn reads_every_parameter_and_defaults_summarize_to_true() {
    let query = decode_legacy_spend_query(
        b"api_key=sk-live&user_id=alice&request_id=r1&start_date=2024-01-02&end_date=2024-01-03",
    )
    .unwrap();
    assert_eq!(
        query,
        LegacySpendQuery {
            api_key: Some(String::from("sk-live")),
            end_date: Some(String::from("2024-01-03")),
            request_id: Some(String::from("r1")),
            start_date: Some(String::from("2024-01-02")),
            summarize: true,
            user_id: Some(String::from("alice")),
        }
    );
}

#[test]
fn an_empty_query_reads_only_the_default() {
    let query = decode_legacy_spend_query(b"").unwrap();
    assert!(query.summarize);
    assert_eq!(query.api_key, None);
    assert_eq!(query.end_date, None);
    assert_eq!(query.request_id, None);
    assert_eq!(query.start_date, None);
    assert_eq!(query.user_id, None);
}

#[test]
fn decodes_percent_encoding_and_plus_signs() {
    let query = decode_legacy_spend_query(b"api_key=a%26b%3Dc&user_id=a+b").unwrap();
    assert_eq!(query.api_key.as_deref(), Some("a&b=c"));
    assert_eq!(query.user_id.as_deref(), Some("a b"));
}

#[test]
fn the_last_repeated_parameter_wins() {
    let query =
        decode_legacy_spend_query(b"summarize=true&summarize=0&user_id=a&user_id=b").unwrap();
    assert!(!query.summarize);
    assert_eq!(query.user_id.as_deref(), Some("b"));
}

#[test]
fn unknown_parameters_are_ignored() {
    let query = decode_legacy_spend_query(b"page=2&span_type=llm&summarize=false").unwrap();
    assert!(!query.summarize);
    assert_eq!(query.api_key, None);
}

#[rstest]
#[case::zero("0", false)]
#[case::off("off", false)]
#[case::f("f", false)]
#[case::false_word("false", false)]
#[case::n("n", false)]
#[case::no("no", false)]
#[case::one("1", true)]
#[case::on("on", true)]
#[case::t("t", true)]
#[case::true_word("true", true)]
#[case::y("y", true)]
#[case::yes("yes", true)]
#[case::upper("TRUE", true)]
fn reads_every_boolean_spelling(#[case] text: &str, #[case] expected: bool) {
    let raw = format!("summarize={text}");
    let query = decode_legacy_spend_query(raw.as_bytes()).unwrap();
    assert_eq!(query.summarize, expected);
}

#[rstest]
#[case::word("maybe")]
#[case::empty("")]
#[case::number("2")]
#[case::spaced(" true")]
fn refuses_a_value_that_is_not_a_boolean(#[case] text: &str) {
    let raw = format!("summarize={text}");
    let error = decode_legacy_spend_query(raw.as_bytes()).unwrap_err();
    assert_eq!(error, LegacyQueryError);
}

#[test]
fn parses_the_calendar_edges() {
    assert_eq!(
        parse_legacy_date("2024-02-29").unwrap(),
        NaiveDate::from_ymd_opt(2024, 2, 29).unwrap()
    );
    assert_eq!(
        parse_legacy_date("9999-12-31").unwrap(),
        NaiveDate::from_ymd_opt(9999, 12, 31).unwrap()
    );
}

#[rstest]
#[case::impossible_day("2024-02-30")]
#[case::impossible_month("2024-13-01")]
#[case::time_suffix("2024-01-02T00:00:00")]
#[case::empty("")]
#[case::words("tomorrow")]
#[case::trailing_space("2024-01-02 ")]
fn refuses_text_that_is_not_a_date(#[case] text: &str) {
    let error = parse_legacy_date(text).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("time data '{text}' does not match format '%Y-%m-%d'")
    );
}

// the leniency chrono adds over strptime, listed in docs/deviations/B1730.yaml
#[test]
fn accepts_unpadded_and_signed_years_that_python_refuses() {
    assert_eq!(
        parse_legacy_date("2024-1-2").unwrap(),
        NaiveDate::from_ymd_opt(2024, 1, 2).unwrap()
    );
    assert_eq!(
        parse_legacy_date("+2024-01-02").unwrap(),
        NaiveDate::from_ymd_opt(2024, 1, 2).unwrap()
    );
}
