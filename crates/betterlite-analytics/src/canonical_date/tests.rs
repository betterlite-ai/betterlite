#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::str;

use chrono::NaiveDate;
use rstest::rstest;

use crate::canonical_date::{
    CanonicalDateRange, InvalidDateRange, PublicErrorDetail, PublicFailure, ScopeDenial,
    ScopeDenied, parse_canonical_date, parse_canonical_date_range, raise_public,
};

fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

#[rstest]
#[case::regular_day("2026-09-24")]
#[case::leap_day("2024-02-29")]
#[case::first_day("0001-01-01")]
#[case::last_day("9999-12-31")]
fn parse_canonical_date_accepts_canonical_dates(#[case] value: &str) {
    assert_eq!(
        parse_canonical_date(value),
        Some(NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap())
    );
}

#[rstest]
#[case::single_digit_month("2026-9-24")]
#[case::single_digit_day("2026-09-4")]
#[case::compact("20260924")]
// full-width digits are what a pasted date from a CJK keyboard looks like
#[case::full_width("\u{ff12}\u{ff10}\u{ff12}\u{ff16}-\u{ff10}\u{ff19}-\u{ff12}\u{ff14}")]
#[case::full_width_month("2026-\u{ff10}\u{ff19}-24")]
#[case::week_date("2026-W39-4")]
#[case::timestamp("2026-09-24T00:00:00")]
#[case::leading_space(" 2026-09-24")]
#[case::trailing_space("2026-09-24 ")]
#[case::slashes("2026/09/24")]
#[case::first_slash("2026/09-24")]
#[case::second_slash("2026-09/24")]
#[case::signed_year("-026-09-24")]
#[case::non_ascii_month("2026-\u{e9}-24")]
#[case::punct_month_tens("2026-:9-24")]
#[case::punct_month_ones("2026-0:-24")]
#[case::empty("")]
#[case::impossible_day("2026-02-30")]
#[case::month_zero("2026-00-01")]
#[case::month_thirteen("2026-13-01")]
#[case::day_zero("2026-09-00")]
#[case::year_zero("0000-01-01")]
#[case::five_digit_year("10000-01-01")]
fn parse_canonical_date_rejects_non_canonical_spellings(#[case] value: &str) {
    assert_eq!(parse_canonical_date(value), None);
}

#[rstest]
#[case::missing_start(None, Some("2026-09-24"))]
#[case::missing_end(Some("2026-09-24"), None)]
#[case::missing_both(None, None)]
fn parse_canonical_date_range_requires_both_dates(
    #[case] start: Option<&str>,
    #[case] end: Option<&str>,
) {
    let error = parse_canonical_date_range(start, end).unwrap_err();
    assert_eq!(error.reason, "Please provide start_date and end_date");
}

#[rstest]
#[case::bad_start(Some("2026-9-24"), Some("2026-09-24"))]
#[case::bad_end(Some("2026-09-24"), Some("2026-9-24"))]
fn parse_canonical_date_range_rejects_non_canonical_dates(
    #[case] start: Option<&str>,
    #[case] end: Option<&str>,
) {
    let error = parse_canonical_date_range(start, end).unwrap_err();
    assert_eq!(
        error.reason,
        "start_date and end_date must be valid YYYY-MM-DD dates"
    );
}

#[test]
fn parse_canonical_date_range_returns_canonical_start_and_end() {
    let range = parse_canonical_date_range(Some("2026-09-01"), Some("2026-09-24")).unwrap();
    assert_eq!(
        range,
        CanonicalDateRange {
            start: date(2026, 9, 1),
            end: date(2026, 9, 24),
        }
    );
}

#[test]
fn parse_canonical_date_range_keeps_a_reversed_range() {
    // LiteLLM's parser has no order check; B815's aggregated range check rejects a reversed range
    let range = parse_canonical_date_range(Some("2026-09-24"), Some("2026-09-01")).unwrap();
    assert_eq!(range.start, date(2026, 9, 24));
    assert_eq!(range.end, date(2026, 9, 1));
}

#[rstest]
#[case::forbidden(ScopeDenial::Forbidden, 403)]
#[case::not_found(ScopeDenial::NotFound, 404)]
fn raise_public_lifts_scope_denied_to_its_status(
    #[case] status: ScopeDenial,
    #[case] expected: u16,
) {
    let raised = raise_public(PublicFailure::ScopeDenied(ScopeDenied {
        status,
        reason: "team is not visible to this key".to_owned(),
    }));
    assert_eq!(raised.status_code, expected);
    assert_eq!(
        raised.detail,
        PublicErrorDetail {
            error: "team is not visible to this key".to_owned(),
        }
    );
}

#[rstest]
#[case::missing("Please provide start_date and end_date")]
#[case::malformed("start_date and end_date must be valid YYYY-MM-DD dates")]
#[case::reversed("end_date must be on or after start_date")]
fn raise_public_lifts_invalid_date_range_to_bad_request(#[case] reason: &str) {
    let raised = raise_public(PublicFailure::InvalidDateRange(InvalidDateRange {
        reason: reason.to_owned(),
    }));
    assert_eq!(raised.status_code, 400);
    assert_eq!(
        raised.detail,
        PublicErrorDetail {
            error: reason.to_owned(),
        }
    );
}

#[test]
fn public_error_detail_serializes_the_fastapi_detail_body() {
    let detail = PublicErrorDetail {
        error: "nope".to_owned(),
    };
    assert_eq!(
        serde_json::to_string(&detail).unwrap(),
        r#"{"error":"nope"}"#
    );
}

#[rstest]
fn replays_the_canonical_date_corpus(#[files("fuzz/corpus/canonical_date/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(text) = str::from_utf8(&bytes) else {
        return;
    };
    let _parsed = parse_canonical_date(text);
    let _range = parse_canonical_date_range(Some(text), Some(text));
}
