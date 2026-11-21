use chrono::NaiveDate;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::benchmark_aggregates::decode::{
    DecodeError, MAX_ROUTER_READ_BYTES, decode_session_agg_rows,
};
use crate::benchmark_aggregates::row::SessionAggRow;
use crate::benchmark_aggregates::window::{BenchmarkWindow, WindowError};

const REQUIRED_ROW: &str = r#"{"router_name":"r","router_type":"complexity","tier_turns":{},"sessions":0,"turns":0,"unordered_turns":0,"covered_turns":0,"cache_hits":0,"same_model_turns":0,"same_model_hits":0,"first_visit_turns":0,"first_visit_hits":0,"return_turns":0,"return_hits":0,"return_expired_misses":0,"return_within_ttl_misses":0,"ttl_5m_turns":0,"ttl_1h_turns":0,"total_tokens":0,"spend":0.0,"saved_spend":0.0,"classifier_cost":0.0,"classifier_cost_recorded_turns":0,"session_seconds":0.0}"#;

fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

#[test]
fn the_default_window_ends_today_and_spans_thirty_days() {
    let window = BenchmarkWindow::resolve(None, None, date(2024, 3, 15)).unwrap();
    assert_eq!(window.start_date, "2024-02-14");
    assert_eq!(window.end_date, "2024-03-15");
    assert_eq!(window.query_start, "2024-02-14T00:00:00");
    assert_eq!(window.query_end, "2024-03-16T00:00:00");
}

#[test]
fn the_default_window_starts_on_the_leap_day_when_it_ends_in_march() {
    let window = BenchmarkWindow::resolve(None, Some("2024-03-15"), date(2024, 1, 1)).unwrap();
    assert_eq!(window.start_date, "2024-02-14");
    assert_eq!(window.end_date, "2024-03-15");
}

#[test]
fn a_single_day_window_queries_the_day_after() {
    let window =
        BenchmarkWindow::resolve(Some("2024-02-29"), Some("2024-02-29"), date(2024, 3, 15))
            .unwrap();
    assert_eq!(window.start_date, "2024-02-29");
    assert_eq!(window.end_date, "2024-02-29");
    assert_eq!(window.query_start, "2024-02-29T00:00:00");
    assert_eq!(window.query_end, "2024-03-01T00:00:00");
}

#[test]
fn an_end_on_new_years_eve_queries_the_next_year() {
    let window =
        BenchmarkWindow::resolve(Some("2024-12-31"), Some("2024-12-31"), date(2024, 3, 15))
            .unwrap();
    assert_eq!(window.query_end, "2025-01-01T00:00:00");
}

#[test]
fn one_digit_month_and_day_are_accepted() {
    let window =
        BenchmarkWindow::resolve(Some("2024-1-5"), Some("2024-1-6"), date(2024, 3, 15)).unwrap();
    assert_eq!(window.start_date, "2024-01-05");
    assert_eq!(window.end_date, "2024-01-06");
}

#[test]
fn a_space_padded_day_is_read_like_pythons_strptime() {
    let window =
        BenchmarkWindow::resolve(Some("2024-1- 5"), Some("2024-1-6"), date(2024, 3, 15)).unwrap();
    assert_eq!(window.start_date, "2024-01-05");
    assert_eq!(window.end_date, "2024-01-06");
}

#[test]
fn an_early_year_prints_an_unpadded_response_date_and_a_padded_query_bound() {
    let window =
        BenchmarkWindow::resolve(Some("0001-01-02"), Some("0001-01-02"), date(2024, 1, 1)).unwrap();
    assert_eq!(window.start_date, "1-01-02");
    assert_eq!(window.end_date, "1-01-02");
    assert_eq!(window.query_start, "0001-01-02T00:00:00");
    assert_eq!(window.query_end, "0001-01-03T00:00:00");
}

#[test]
fn a_default_start_on_the_first_calendar_day_is_accepted() {
    let window = BenchmarkWindow::resolve(None, Some("0001-01-31"), date(2024, 1, 1)).unwrap();
    assert_eq!(window.start_date, "1-01-01");
    assert_eq!(window.end_date, "1-01-31");
    assert_eq!(window.query_start, "0001-01-01T00:00:00");
    assert_eq!(window.query_end, "0001-02-01T00:00:00");
}

#[test]
fn an_explicit_start_survives_the_last_calendar_day() {
    let window =
        BenchmarkWindow::resolve(Some("9999-12-30"), Some("9999-12-30"), date(2024, 1, 1)).unwrap();
    assert_eq!(window.query_end, "9999-12-31T00:00:00");
}

#[rstest]
#[case("2024-13-01")]
#[case("2024-00-10")]
#[case("2024-02-30")]
#[case("2024-1")]
#[case("24-01-01")]
#[case("02024-01-01")]
#[case("2024-001-01")]
#[case("2024-01-001")]
#[case("2024-01-01x")]
#[case("2024-01-01-01")]
#[case("abcd-01-01")]
#[case("+202-01-01")]
#[case("2024-+1-01")]
#[case("2024-01-+1")]
#[case("2024-01- 12")]
#[case("0000-01-01")]
#[case("")]
fn an_unparseable_day_is_refused(#[case] value: &str) {
    let error =
        BenchmarkWindow::resolve(Some(value), Some("2024-01-02"), date(2024, 1, 2)).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("Invalid date format: {value}. Expected: 'YYYY-MM-DD'")
    );
}

#[test]
fn an_unparseable_end_day_names_its_own_value() {
    let error = BenchmarkWindow::resolve(None, Some("2024-02-31"), date(2024, 1, 2)).unwrap_err();
    assert_eq!(
        error,
        WindowError::InvalidDate {
            value: "2024-02-31".to_owned()
        }
    );
}

#[test]
fn a_window_ending_before_it_starts_is_refused() {
    let error = BenchmarkWindow::resolve(Some("2024-03-01"), Some("2024-02-29"), date(2024, 3, 1))
        .unwrap_err();
    assert_eq!(error, WindowError::ReversedRange);
    assert_eq!(
        error.to_string(),
        "end_date must not be earlier than start_date"
    );
}

#[test]
fn a_default_start_before_the_calendar_is_refused() {
    assert_eq!(
        BenchmarkWindow::resolve(None, Some("0001-01-30"), date(2024, 1, 1)),
        Err(WindowError::CalendarRange)
    );
}

#[test]
fn an_end_on_the_last_calendar_day_is_refused() {
    assert_eq!(
        BenchmarkWindow::resolve(Some("9999-12-31"), Some("9999-12-31"), date(2024, 1, 1)),
        Err(WindowError::CalendarRange)
    );
}

#[test]
fn an_empty_list_decodes_to_no_rows() {
    assert_eq!(decode_session_agg_rows(b"[]").unwrap(), Vec::new());
}

#[test]
fn a_document_that_is_not_a_row_list_is_refused() {
    let error = decode_session_agg_rows(b"{}").unwrap_err();
    assert!(matches!(&error, DecodeError::Invalid { .. }));
}

#[test]
fn a_row_missing_a_required_field_is_refused() {
    let error = decode_session_agg_rows(b"[{\"router_name\":\"r\"}]").unwrap_err();
    assert!(matches!(&error, DecodeError::Invalid { .. }));
}

#[test]
fn a_row_of_the_wrong_field_type_is_refused() {
    let error = decode_session_agg_rows(b"[{\"router_name\":1}]").unwrap_err();
    assert!(matches!(&error, DecodeError::Invalid { .. }));
}

#[test]
fn a_row_without_savings_estimates_takes_the_defaults() {
    let document = format!("[{REQUIRED_ROW}]");
    let rows = decode_session_agg_rows(document.as_bytes()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].savings_estimated_turns, 0);
    assert_eq!(rows[0].savings_estimated_actual_spend, Decimal::ZERO);
    assert_eq!(rows[0].savings_estimated_classifier_cost, None);
    assert_eq!(rows[0].savings_estimated_saved_spend, Decimal::ZERO);
}

#[test]
fn an_unknown_row_key_is_ignored() {
    let body = REQUIRED_ROW.strip_prefix('{').unwrap();
    let document = format!("[{{\"extra\":1,{body}]");
    let rows = decode_session_agg_rows(document.as_bytes()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].router_name, "r");
}

#[test]
fn a_document_at_the_limit_decodes() {
    let mut document = serde_json::to_vec(&[SessionAggRow::default()]).unwrap();
    document.resize(MAX_ROUTER_READ_BYTES, b' ');
    assert_eq!(document.len(), MAX_ROUTER_READ_BYTES);
    assert_eq!(decode_session_agg_rows(&document).unwrap().len(), 1);
}

#[test]
fn a_document_one_byte_past_the_limit_is_refused() {
    let mut document = serde_json::to_vec(&[SessionAggRow::default()]).unwrap();
    let past_limit = MAX_ROUTER_READ_BYTES.saturating_add(1);
    document.resize(past_limit, b' ');
    let error = decode_session_agg_rows(&document).unwrap_err();
    assert!(matches!(
        &error,
        DecodeError::TooLarge { bytes, limit }
            if *bytes == past_limit && *limit == MAX_ROUTER_READ_BYTES
    ));
    assert_eq!(
        error.to_string(),
        "the router read payload has 1048577 bytes, past the 1048576-byte limit"
    );
}

#[test]
fn the_limit_is_a_megabyte() {
    assert_eq!(MAX_ROUTER_READ_BYTES, 1 << 20);
}
