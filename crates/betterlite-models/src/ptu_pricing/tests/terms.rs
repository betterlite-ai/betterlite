use std::str::FromStr as _;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, TimeZone as _, Utc};
use rstest::rstest;
use rust_decimal::Decimal;

use super::{declaration, model_info};
use crate::ptu_pricing::{PtuInstantSource, PtuModelInfo, PtuTerms, ptu_terms};

fn terms(fields: &str) -> Option<PtuTerms> {
    let source = declaration(fields);
    let info = model_info(&source);
    ptu_terms(&PtuModelInfo::from(&info))
}

fn rate_terms(rate: &str) -> Option<PtuTerms> {
    terms(&format!(
        r#""ptu_count":1,"cost_per_ptu_per_hour":{rate},"team_id":"team-1""#
    ))
}

fn count_terms(count: &str) -> Option<PtuTerms> {
    terms(&format!(
        r#""ptu_count":{count},"cost_per_ptu_per_hour":1,"team_id":"team-1""#
    ))
}

#[rstest]
#[case::count_absent(r#""cost_per_ptu_per_hour":2,"team_id":"team-1""#)]
#[case::rate_absent(r#""ptu_count":2,"team_id":"team-1""#)]
#[case::team_absent(r#""ptu_count":2,"cost_per_ptu_per_hour":1"#)]
#[case::team_null(r#""ptu_count":2,"cost_per_ptu_per_hour":1,"team_id":null"#)]
#[case::team_empty(r#""ptu_count":2,"cost_per_ptu_per_hour":1,"team_id":"""#)]
#[case::count_null(r#""ptu_count":null,"cost_per_ptu_per_hour":1,"team_id":"team-1""#)]
#[case::rate_null(r#""ptu_count":2,"cost_per_ptu_per_hour":null,"team_id":"team-1""#)]
fn terms_rejects_incomplete_declarations(#[case] fields: &str) {
    assert!(terms(fields).is_none());
}

#[rstest]
#[case::integer("3", 3)]
#[case::truncated_float("2.9", 2)]
#[case::numeric_text(r#""12""#, 12)]
#[case::underscored_text(r#""1_000""#, 1000)]
#[case::padded_text(r#"" 7 ""#, 7)]
#[case::exponent_number("1e3", 1000)]
fn terms_coerce_the_count(#[case] count: &str, #[case] expected: u32) {
    assert_eq!(count_terms(count).unwrap().ptu_count, expected);
}

#[rstest]
#[case::word(r#""three""#)]
#[case::fraction_text(r#""1.5""#)]
#[case::bool("true")]
#[case::array("[]")]
#[case::object("{}")]
fn terms_rejects_failed_count_conversions(#[case] count: &str) {
    assert!(count_terms(count).is_none());
}

#[rstest]
#[case::zero("0")]
#[case::negative("-1")]
#[case::past_the_maximum("1000001")]
#[case::far_past_the_maximum("1e9")]
fn terms_rejects_out_of_range_counts(#[case] count: &str) {
    assert!(count_terms(count).is_none());
}

#[test]
fn terms_accepts_the_count_bounds() {
    assert_eq!(count_terms("1").unwrap().ptu_count, 1);
    assert_eq!(count_terms("1000000").unwrap().ptu_count, 1_000_000_u32);
}

#[rstest]
#[case::decimal_number("0.5", "0.5")]
#[case::numeric_text(r#""0.5""#, "0.5")]
#[case::exponent_text(r#""1e3""#, "1000")]
#[case::underscored_text(r#""1_0.5""#, "10.5")]
#[case::padded_text(r#"" 2 ""#, "2")]
#[case::trailing_point(r#""5.""#, "5")]
#[case::leading_point(r#"".5""#, "0.5")]
fn terms_coerce_the_hourly_rate(#[case] rate: &str, #[case] expected: &str) {
    let terms = rate_terms(rate).unwrap();
    assert_eq!(
        terms.cost_per_ptu_per_hour,
        Decimal::from_str(expected).unwrap()
    );
}

#[rstest]
#[case::word(r#""free""#)]
#[case::trailing_underscore(r#""1_""#)]
#[case::doubled_underscore(r#""1__0""#)]
#[case::bool("true")]
#[case::array("[]")]
fn terms_rejects_failed_rate_conversions(#[case] rate: &str) {
    assert!(rate_terms(rate).is_none());
}

#[rstest]
#[case::negative("-0.1")]
#[case::past_the_maximum("1000000.1")]
#[case::far_past_the_maximum("1e9")]
fn terms_rejects_out_of_range_rates(#[case] rate: &str) {
    assert!(rate_terms(rate).is_none());
}

#[test]
fn terms_accepts_the_rate_bounds() {
    assert_eq!(
        rate_terms("0").unwrap().cost_per_ptu_per_hour,
        Decimal::ZERO
    );
    assert_eq!(
        rate_terms("1000000").unwrap().cost_per_ptu_per_hour,
        Decimal::from_str("1000000").unwrap()
    );
}

#[test]
fn terms_keeps_the_exact_hourly_rate() {
    let exact = "0.123456789012345678901234567";
    let terms = rate_terms(&format!(r#""{exact}""#)).unwrap();
    assert_eq!(
        terms.cost_per_ptu_per_hour,
        Decimal::from_str(exact).unwrap()
    );
}

// A rate float() holds but Decimal cannot represent stays unchargeable rather than rounding to zero.
#[test]
fn terms_rejects_a_rate_beyond_the_decimal_scale() {
    assert!(rate_terms(r#""1e-30""#).is_none());
}

#[rstest]
#[case::date_only("2026-01-01", "2026-01-01T00:00:00Z")]
#[case::naive_datetime("2026-01-01T10:00:00", "2026-01-01T10:00:00Z")]
#[case::utc_datetime("2026-01-01T10:00:00Z", "2026-01-01T10:00:00Z")]
#[case::offset_datetime("2026-01-01T10:00:00+05:30", "2026-01-01T04:30:00Z")]
#[case::fractional_datetime("2026-01-01T10:00:00.123456Z", "2026-01-01T10:00:00.123456Z")]
fn terms_converts_the_start_to_utc(#[case] source: &str, #[case] expected: &str) {
    let fields = format!(
        r#""ptu_count":1,"cost_per_ptu_per_hour":1,"team_id":"team-1","ptu_effective_from":"{source}""#
    );
    let expected_instant = DateTime::parse_from_rfc3339(expected).unwrap();
    assert_eq!(
        terms(&fields).unwrap().effective_from,
        expected_instant.with_timezone(&Utc)
    );
}

#[rstest]
#[case::absent(None)]
#[case::null(Some("null"))]
#[case::utc(Some(r#""2026-06-01T00:00:00Z""#))]
#[case::offset(Some(r#""2026-06-01T05:30:00+05:30""#))]
fn terms_reads_the_optional_end(#[case] end: Option<&str>) {
    let fields = end.map_or_else(
        || r#""ptu_count":1,"cost_per_ptu_per_hour":1,"team_id":"team-1""#.to_owned(),
        |end_text| {
            format!(
                r#""ptu_count":1,"cost_per_ptu_per_hour":1,"team_id":"team-1","ptu_effective_to":{end_text}"#
            )
        },
    );
    let terms = terms(&fields).unwrap();
    let expected_end = end.filter(|value| *value != "null").map(|value| {
        DateTime::parse_from_rfc3339(value.trim_matches('"'))
            .unwrap()
            .with_timezone(&Utc)
    });
    assert_eq!(terms.effective_to, expected_end);
}

#[rstest]
#[case::invalid_end(r#""not-a-date""#)]
#[case::end_equal_to_start(r#""2026-01-01T00:00:00Z""#)]
#[case::end_before_start(r#""2025-12-31T23:59:59Z""#)]
#[case::numeric_end("12345")]
fn terms_rejects_an_unusable_end(#[case] end: &str) {
    let fields = format!(
        r#""ptu_count":1,"cost_per_ptu_per_hour":1,"team_id":"team-1","ptu_effective_to":{end}"#
    );
    assert!(terms(&fields).is_none());
}

#[rstest]
#[case::start_absent("")]
#[case::start_null(r#","ptu_effective_from":null"#)]
#[case::start_invalid(r#","ptu_effective_from":"soon""#)]
#[case::start_numeric(r#","ptu_effective_from":17"#)]
fn terms_requires_a_parseable_start(#[case] start: &str) {
    let fields = format!(r#""ptu_count":1,"cost_per_ptu_per_hour":1,"team_id":"team-1"{start}"#);
    let source = format!("{{{fields}}}");
    let info = model_info(&source);
    assert!(ptu_terms(&PtuModelInfo::from(&info)).is_none());
}

#[test]
fn terms_reads_a_typed_yaml_date_as_utc_midnight() {
    let source = declaration(r#""ptu_count":1,"cost_per_ptu_per_hour":1,"team_id":"team-1""#);
    let info = model_info(&source);
    let mut typed = PtuModelInfo::from(&info);
    typed.ptu_effective_from = Some(PtuInstantSource::Date(
        NaiveDate::from_ymd_opt(2027, 2, 3).unwrap(),
    ));
    let terms = ptu_terms(&typed).unwrap();
    assert_eq!(
        terms.effective_from,
        Utc.with_ymd_and_hms(2027, 2, 3, 0, 0, 0).unwrap()
    );
}

#[test]
fn terms_reads_a_naive_datetime_as_utc() {
    let source = declaration(r#""ptu_count":1,"cost_per_ptu_per_hour":1,"team_id":"team-1""#);
    let info = model_info(&source);
    let mut typed = PtuModelInfo::from(&info);
    typed.ptu_effective_from = Some(PtuInstantSource::NaiveDateTime(
        NaiveDateTime::parse_from_str("2027-02-03T05:06:07", "%Y-%m-%dT%H:%M:%S").unwrap(),
    ));
    let terms = ptu_terms(&typed).unwrap();
    assert_eq!(
        terms.effective_from,
        Utc.with_ymd_and_hms(2027, 2, 3, 5, 6, 7).unwrap()
    );
}

#[test]
fn terms_reads_an_aware_datetime_as_its_utc_instant() {
    let source = declaration(r#""ptu_count":1,"cost_per_ptu_per_hour":1,"team_id":"team-1""#);
    let info = model_info(&source);
    let mut typed = PtuModelInfo::from(&info);
    let offset = FixedOffset::east_opt(5 * 3600 + 1800).unwrap();
    typed.ptu_effective_from = Some(PtuInstantSource::DateTime(
        offset.with_ymd_and_hms(2027, 2, 3, 10, 0, 0).unwrap(),
    ));
    let terms = ptu_terms(&typed).unwrap();
    assert_eq!(
        terms.effective_from,
        Utc.with_ymd_and_hms(2027, 2, 3, 4, 30, 0).unwrap()
    );
}
