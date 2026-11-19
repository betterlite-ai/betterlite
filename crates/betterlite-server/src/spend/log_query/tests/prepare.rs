use std::str::from_utf8;

use axum::body::to_bytes;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use rstest::rstest;

use super::super::dates::DateFormatError;
use super::super::prepare::{PrepareError, SpendLogRoute, SpendLogWindow};
use super::{DATES, decode, utc};

#[test]
fn prepare_requires_both_dates_on_the_public_route() {
    assert_eq!(
        decode("request_id=r").prepare(SpendLogRoute::PublicV2),
        Err(PrepareError::MissingDates)
    );
    assert_eq!(
        decode("start_date=2024-01-02+00:00:00").prepare(SpendLogRoute::PublicV2),
        Err(PrepareError::MissingDates)
    );
    assert_eq!(
        decode("end_date=2024-01-02+00:00:00").prepare(SpendLogRoute::PublicV2),
        Err(PrepareError::MissingDates)
    );
}

#[test]
fn prepare_requires_both_dates_on_the_ui_route_without_a_request_id() {
    assert_eq!(
        decode("").prepare(SpendLogRoute::Ui),
        Err(PrepareError::MissingDates)
    );
    assert_eq!(
        decode("search=x").prepare(SpendLogRoute::Ui),
        Err(PrepareError::MissingDates)
    );
}

#[rstest]
#[case::ui(SpendLogRoute::Ui)]
#[case::public_v2(SpendLogRoute::PublicV2)]
fn prepare_accepts_second_precision_timestamps_on_both_routes(#[case] route: SpendLogRoute) {
    let query = decode("start_date=2024-01-02+03:04:05&end_date=2024-01-03+04:05:06");
    let window = query.prepare(route).unwrap().window.unwrap();
    assert_eq!(window.start_inclusive, utc("2024-01-02 03:04:05"));
    assert_eq!(window.end_inclusive, utc("2024-01-03 04:05:06"));
}

#[test]
fn prepare_accepts_the_field_edges() {
    let query = decode("start_date=2024-01-01+00:00:00&end_date=2024-12-31+23:59:59");
    let window = query.prepare(SpendLogRoute::Ui).unwrap().window.unwrap();
    assert_eq!(window.start_inclusive, utc("2024-01-01 00:00:00"));
    assert_eq!(window.end_inclusive, utc("2024-12-31 23:59:59"));
}

#[test]
fn prepare_accepts_date_only_on_the_public_route() {
    let query = decode(DATES);
    let window = query
        .prepare(SpendLogRoute::PublicV2)
        .unwrap()
        .window
        .unwrap();
    assert_eq!(window.start_inclusive, utc("2024-01-02 00:00:00"));
    assert_eq!(window.end_inclusive, utc("2024-01-03 00:00:00"));
}

#[test]
fn prepare_rejects_date_only_on_the_ui_route() {
    let error = decode(DATES).prepare(SpendLogRoute::Ui).unwrap_err();
    assert_eq!(
        error,
        PrepareError::InvalidDate {
            source: DateFormatError {
                value: "2024-01-02".to_owned(),
                expected: "'YYYY-MM-DD HH:MM:SS'",
            },
        }
    );
}

#[test]
fn prepare_strips_surrounding_whitespace() {
    let query = decode("start_date=%202024-01-02+03:04:05%20&end_date=%092024-01-03+04:05:06%09");
    let window = query.prepare(SpendLogRoute::Ui).unwrap().window.unwrap();
    assert_eq!(window.start_inclusive, utc("2024-01-02 03:04:05"));
    assert_eq!(window.end_inclusive, utc("2024-01-03 04:05:06"));
}

#[test]
fn prepare_accepts_single_digit_fields_like_strptime() {
    let query = decode("start_date=2024-1-2+3:4:5&end_date=2024-12-3+4:05:6");
    let window = query.prepare(SpendLogRoute::Ui).unwrap().window.unwrap();
    assert_eq!(window.start_inclusive, utc("2024-01-02 03:04:05"));
    assert_eq!(window.end_inclusive, utc("2024-12-03 04:05:06"));
}

#[test]
fn prepare_accepts_a_whitespace_run_between_date_and_time() {
    let query = decode("start_date=2024-01-02++03:04:05&end_date=2024-01-03%0904:05:06");
    let window = query.prepare(SpendLogRoute::Ui).unwrap().window.unwrap();
    assert_eq!(window.start_inclusive, utc("2024-01-02 03:04:05"));
    assert_eq!(window.end_inclusive, utc("2024-01-03 04:05:06"));
}

#[test]
fn prepare_accepts_date_only_single_digit_month_and_day() {
    let query = decode("start_date=2024-1-2&end_date=2024-12-3");
    let window = query
        .prepare(SpendLogRoute::PublicV2)
        .unwrap()
        .window
        .unwrap();
    assert_eq!(window.start_inclusive, utc("2024-01-02 00:00:00"));
    assert_eq!(window.end_inclusive, utc("2024-12-03 00:00:00"));
}

#[rstest]
#[case::missing_seconds("2024-01-02 03:04")]
#[case::t_separator("2024-01-02T03:04:05")]
#[case::day_out_of_range("2024-02-30 03:04:05")]
#[case::day_zero("2024-01-00 03:04:05")]
#[case::day_above_range("2024-01-32 03:04:05")]
#[case::month_out_of_range("2024-13-02 03:04:05")]
#[case::month_zero("2024-00-02 03:04:05")]
#[case::hour_out_of_range("2024-01-02 24:04:05")]
#[case::minute_out_of_range("2024-01-02 03:60:05")]
#[case::leap_second("2024-01-02 03:04:60")]
#[case::year_zero("0000-01-02 03:04:05")]
#[case::short_year("999-01-02 03:04:05")]
#[case::two_digit_year("24-01-02 03:04:05")]
#[case::wide_month("2024-001-02 03:04:05")]
#[case::wide_day("2024-01-002 03:04:05")]
#[case::wide_hour("2024-01-02 003:04:05")]
#[case::wide_second("2024-01-02 03:04:005")]
#[case::day_single_zero("2024-01-0 03:04:05")]
#[case::day_space_before_zero("2024-01- 0 03:04:05")]
#[case::day_space_before_two_digits("2024-01- 15 03:04:05")]
#[case::no_separator("2024-01-0203:04:05")]
fn prepare_rejects_dates_outside_the_shape(#[case] date: &str) {
    let encoded = date.replace(' ', "+");
    let raw = format!("start_date={encoded}&end_date=2024-01-03+00:00:00");
    let error = decode(&raw).prepare(SpendLogRoute::PublicV2).unwrap_err();
    assert_eq!(
        error,
        PrepareError::InvalidDate {
            source: DateFormatError {
                value: date.to_owned(),
                expected: "'YYYY-MM-DD' or 'YYYY-MM-DD HH:MM:SS'",
            },
        }
    );
}

#[test]
fn prepare_accepts_a_space_before_a_single_digit_day_on_the_ui_route() {
    let query = decode("start_date=2024-01-+5+03:04:05&end_date=2024-01-+9+04:05:06");
    let window = query.prepare(SpendLogRoute::Ui).unwrap().window.unwrap();
    assert_eq!(window.start_inclusive, utc("2024-01-05 03:04:05"));
    assert_eq!(window.end_inclusive, utc("2024-01-09 04:05:06"));
}

#[test]
fn prepare_accepts_a_space_before_a_single_digit_day_on_the_public_route() {
    let query = decode("start_date=2024-01-+5&end_date=2024-01-+9");
    let window = query
        .prepare(SpendLogRoute::PublicV2)
        .unwrap()
        .window
        .unwrap();
    assert_eq!(window.start_inclusive, utc("2024-01-05 00:00:00"));
    assert_eq!(window.end_inclusive, utc("2024-01-09 00:00:00"));
}

#[test]
fn prepare_rejects_unicode_digits_that_strptime_would_accept() {
    let error = decode("start_date=%D9%A2%D9%A0%D9%A2%D9%A4-01-05&end_date=2024-01-03")
        .prepare(SpendLogRoute::PublicV2)
        .unwrap_err();
    assert_eq!(
        error,
        PrepareError::InvalidDate {
            source: DateFormatError {
                value: "٢٠٢٤-01-05".to_owned(),
                expected: "'YYYY-MM-DD' or 'YYYY-MM-DD HH:MM:SS'",
            },
        }
    );
}

#[test]
fn prepare_does_not_check_the_window_order() {
    let query = decode("start_date=2024-01-05+00:00:00&end_date=2024-01-01+00:00:00");
    let window = query.prepare(SpendLogRoute::Ui).unwrap().window.unwrap();
    assert_eq!(window.start_inclusive, utc("2024-01-05 00:00:00"));
    assert_eq!(window.end_inclusive, utc("2024-01-01 00:00:00"));
}

#[test]
fn prepare_drops_the_window_for_a_ui_request_id_lookup() {
    let prepared = decode("request_id=r&start_date=nope&end_date=nope")
        .prepare(SpendLogRoute::Ui)
        .unwrap();
    assert_eq!(prepared.window, None);
    assert_eq!(prepared.search, None);
    assert_eq!(prepared.request_id.as_deref(), Some("r"));
    assert!(!prepared.search_owns_window);
}

#[test]
fn prepare_drops_the_window_even_when_dates_parse() {
    let prepared =
        decode("request_id=r&start_date=2024-01-02+00:00:00&end_date=2024-01-03+00:00:00")
            .prepare(SpendLogRoute::Ui)
            .unwrap();
    assert_eq!(prepared.window, None);
    assert_eq!(prepared.request_id.as_deref(), Some("r"));
}

#[test]
fn prepare_still_requires_dates_for_a_ui_request_id_with_search() {
    assert_eq!(
        decode("request_id=r&search=x").prepare(SpendLogRoute::Ui),
        Err(PrepareError::MissingDates)
    );
}

#[test]
fn prepare_keeps_a_ui_search_inside_its_own_window() {
    let query = decode("search=abc&start_date=2024-01-02+00:00:00&end_date=2024-01-03+00:00:00");
    let prepared = query.prepare(SpendLogRoute::Ui).unwrap();
    assert!(prepared.search_owns_window);
    assert_eq!(
        prepared.window,
        Some(SpendLogWindow {
            start_inclusive: utc("2024-01-02 00:00:00"),
            end_inclusive: utc("2024-01-03 00:00:00"),
        })
    );
    let search = prepared.search.unwrap();
    assert_eq!(search.value, "abc");
    assert_eq!(search.window.start_inclusive, utc("2024-01-02 00:00:00"));
    assert_eq!(search.window.end_inclusive, utc("2024-01-03 00:00:00"));
}

#[test]
fn prepare_keeps_the_outer_window_for_the_public_route_with_search() {
    let query = decode("search=abc&start_date=2024-01-02&end_date=2024-01-03");
    let prepared = query.prepare(SpendLogRoute::PublicV2).unwrap();
    assert!(!prepared.search_owns_window);
    assert!(prepared.window.is_some());
    assert!(prepared.search.is_some());
}

#[test]
fn prepare_reports_the_stripped_value_in_the_date_error() {
    let query = decode("start_date=%20nope%20&end_date=2024-01-03+00:00:00");
    let error = query.prepare(SpendLogRoute::PublicV2).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid date format: nope. Expected: 'YYYY-MM-DD' or 'YYYY-MM-DD HH:MM:SS'"
    );
}

#[tokio::test]
async fn prepare_missing_dates_error_is_a_bad_request_envelope() {
    let response = decode("")
        .prepare(SpendLogRoute::Ui)
        .unwrap_err()
        .into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        from_utf8(&body).unwrap(),
        r#"{"error":{"message":"Start date and end date are required","type":"bad_request","param":"None","code":"400"}}"#
    );
}

#[tokio::test]
async fn prepare_invalid_date_error_is_the_converted_proxy_envelope() {
    let response = decode(DATES)
        .prepare(SpendLogRoute::Ui)
        .unwrap_err()
        .into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        from_utf8(&body).unwrap(),
        r#"{"error":{"message":"Invalid date format: 2024-01-02. Expected: 'YYYY-MM-DD HH:MM:SS'","type":"internal_server_error","param":null,"code":"400"}}"#
    );
}
