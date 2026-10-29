#![cfg(test)]

use chrono::{DateTime, NaiveTime, Utc};
use rstest::rstest;
use rust_decimal::Decimal;

use super::block::is_off_peak;
use super::rate::{off_peak_rate, parse_rate};
use super::schema::{HoursField, OffPeakBlock, RateField};
use super::window::{WindowError, is_within_off_peak_window, matches_any_window, window_strings};
use super::*;

fn moment(text: &str) -> DateTime<Utc> {
    text.parse().unwrap()
}

fn dec(text: &str) -> Decimal {
    text.parse().unwrap()
}

fn hours_field(json: &str) -> HoursField {
    serde_json::from_str(json).unwrap()
}

fn block(json: &str) -> OffPeakBlock {
    serde_json::from_str(json).unwrap()
}

fn standard_rates() -> TokenRates {
    TokenRates {
        input_rate: dec("0.1"),
        output_rate: dec("0.2"),
        cache_read_rate: dec("0.01"),
        cache_creation_rate: dec("0.03"),
        reasoning_rate: None,
    }
}

#[rstest]
#[case::inclusive_start("\"09:00-17:00\"", "09:00:00", true)]
#[case::exclusive_end("\"09:00-17:00\"", "17:00:00", false)]
#[case::outside("\"09:00-17:00\"", "08:59:59", false)]
#[case::midnight_cross_before("\"22:00-02:00\"", "23:30:00", true)]
#[case::midnight_cross_after("\"22:00-02:00\"", "01:30:00", true)]
#[case::midnight_cross_outside("\"22:00-02:00\"", "12:00:00", false)]
#[case::equal_endpoints_all_day("\"00:00-00:00\"", "13:45:00", true)]
#[case::single_digit_forms("\"9:5-10:30\"", "09:10:00", true)]
#[case::three_components_malformed("\"10:00-12:00-14:00\"", "11:00:00", false)]
#[case::out_of_range_minute_malformed("\"09:00-09:60\"", "09:30:00", false)]
fn window_matching_follows_inclusive_start_exclusive_end(
    #[case] hours_json: &str,
    #[case] time_text: &str,
    #[case] expected: bool,
) {
    let field = hours_field(hours_json);
    let time: NaiveTime = time_text.parse().unwrap();
    assert_eq!(is_within_off_peak_window(&field, time).unwrap(), expected);
    assert_eq!(
        matches_any_window(&window_strings(Some(&field)), time),
        expected
    );
}

#[test]
fn mixed_sequences_keep_only_string_entries() {
    let field = hours_field(r#"["09:00-10:00", 5, false, null, "11:00-12:00"]"#);
    assert_eq!(
        window_strings(Some(&field)),
        vec!["09:00-10:00".to_owned(), "11:00-12:00".to_owned()],
    );
}

#[test]
fn low_level_window_helper_rejects_noniterable_input_the_adapter_tolerates() {
    let field = hours_field("5");
    assert_eq!(window_strings(Some(&field)), Vec::<String>::new());
    assert_eq!(
        is_within_off_peak_window(&field, moment("1970-01-01T00:00:00Z").time()),
        Err(WindowError::NotIterable)
    );
}

#[rstest]
#[case::missing_weekdays_matches_every_day(
    r#"{"hours_utc":"09:00-10:00"}"#,
    "2024-06-16T09:30:00Z",
    true
)]
#[case::empty_list_matches_nothing(
    r#"{"windows":[{"hours_utc":"09:00-10:00","weekdays":[]}]}"#,
    "2024-06-16T09:30:00Z",
    false
)]
#[case::int_weekday_matches(
    r#"{"windows":[{"hours_utc":"09:00-10:00","weekdays":[1,2,3,4,5]}]}"#,
    "2024-06-17T09:30:00Z",
    true
)]
#[case::alias_weekday_matches(
    r#"{"windows":[{"hours_utc":"09:00-10:00","weekdays":["Sunday"]}]}"#,
    "2024-06-16T09:30:00Z",
    true
)]
#[case::boolean_weekday_excluded(
    r#"{"windows":[{"hours_utc":"09:00-10:00","weekdays":[true,false]}]}"#,
    "2024-06-16T09:30:00Z",
    false
)]
#[case::string_weekdays_rejects_rule(
    r#"{"windows":[{"hours_utc":"09:00-10:00","weekdays":"mon"}]}"#,
    "2024-06-17T09:30:00Z",
    false
)]
fn weekday_rules_follow_litellm_normalization(
    #[case] off_peak_json: &str,
    #[case] moment_text: &str,
    #[case] expected: bool,
) {
    let off_peak = block(off_peak_json);
    assert_eq!(is_off_peak(&off_peak, moment(moment_text)), expected);
}

#[test]
fn flat_hours_wins_independently_of_an_empty_weekdays_window() {
    let off_peak = block(
        r#"{"hours_utc":"00:00-23:59","windows":[{"hours_utc":"00:00-23:59","weekdays":[]}]}"#,
    );
    assert!(is_off_peak(&off_peak, moment("2024-06-16T12:00:00Z")));
}

#[test]
fn weekday_timezone_shanghai_moves_a_utc_sunday_into_a_local_monday() {
    let off_peak = block(
        r#"{"windows":[{"hours_utc":"00:00-23:59","weekdays":[1],"weekday_timezone":null}],"weekday_timezone":"Asia/Shanghai"}"#,
    );
    // 2024-06-16 23:00 UTC is Sunday in UTC but already Monday 2024-06-17 07:00 in Shanghai (UTC+8).
    assert!(is_off_peak(&off_peak, moment("2024-06-16T23:00:00Z")));
}

#[test]
fn invalid_weekday_timezone_falls_back_to_utc() {
    let off_peak = block(
        r#"{"windows":[{"hours_utc":"00:00-23:59","weekdays":[1]}],"weekday_timezone":"Not/AZone"}"#,
    );
    // Monday on the UTC calendar, which the fallback must use since the zone is invalid.
    assert!(is_off_peak(&off_peak, moment("2024-06-17T12:00:00Z")));
    assert!(!is_off_peak(&off_peak, moment("2024-06-16T12:00:00Z")));
}

#[test]
fn weekday_timezone_respects_a_dst_transition() {
    let off_peak = block(
        r#"{"windows":[{"hours_utc":"08:00-09:00","weekdays":[2]}],"weekday_timezone":"America/New_York"}"#,
    );
    // 2024-03-12 08:30 UTC is 04:30 EDT (DST already in effect), still Tuesday locally.
    assert!(is_off_peak(&off_peak, moment("2024-03-12T08:30:00Z")));
}

#[rstest]
#[case::number("0.05", Some(dec("0.05")))]
#[case::numeric_string(r#""0.05""#, Some(dec("0.05")))]
#[case::boolean_falls_back("true", None)]
#[case::malformed_text_falls_back(r#""not-a-number""#, None)]
fn rate_parsing_matches_litellms_tolerance(#[case] json: &str, #[case] expected: Option<Decimal>) {
    let field: RateField = serde_json::from_str(json).unwrap();
    assert_eq!(parse_rate(Some(&field), "rate").unwrap(), expected);
}

#[test]
fn nonfinite_rate_text_is_a_typed_error_not_a_silent_fallback() {
    let field: RateField = serde_json::from_str(r#""inf""#).unwrap();
    assert!(matches!(
        parse_rate(Some(&field), "rate"),
        Err(OffPeakRateError::NonFinite { field: "rate" })
    ));
}

#[test]
fn unrepresentable_rate_magnitude_is_a_typed_error() {
    let field: RateField = serde_json::from_str(r#""1e50""#).unwrap();
    assert!(matches!(
        parse_rate(Some(&field), "rate"),
        Err(OffPeakRateError::Unrepresentable { field: "rate", .. })
    ));
}

#[test]
fn off_peak_rate_falls_back_to_the_standard_rate_when_absent() {
    assert_eq!(off_peak_rate(None, "rate", dec("0.4")).unwrap(), dec("0.4"));
}

#[test]
fn apply_off_peak_pricing_passes_through_when_the_block_is_absent() {
    let rates = standard_rates();
    assert_eq!(
        apply_off_peak_pricing(None, moment("2024-06-16T12:00:00Z"), rates).unwrap(),
        rates
    );
}

#[test]
fn apply_off_peak_pricing_passes_through_outside_the_window() {
    let off_peak = block(r#"{"hours_utc":"09:00-10:00"}"#);
    let rates = standard_rates();
    let result =
        apply_off_peak_pricing(Some(&off_peak), moment("2024-06-16T12:00:00Z"), rates).unwrap();
    assert_eq!(result, rates);
}

#[test]
fn apply_off_peak_pricing_replaces_rates_independently_and_keeps_missing_reasoning() {
    let off_peak = block(
        r#"{"hours_utc":"09:00-10:00","input_cost_per_token":0.001,"output_cost_per_token":"0.002",
           "cache_read_input_token_cost":true,"cache_creation_input_token_cost":"bad"}"#,
    );
    let rates = standard_rates();
    let result =
        apply_off_peak_pricing(Some(&off_peak), moment("2024-06-16T09:30:00Z"), rates).unwrap();
    assert_eq!(result.input_rate, dec("0.001"));
    assert_eq!(result.output_rate, dec("0.002"));
    assert_eq!(result.cache_read_rate, rates.cache_read_rate);
    assert_eq!(result.cache_creation_rate, rates.cache_creation_rate);
    assert_eq!(result.reasoning_rate, None);
    assert_eq!(result.billed_reasoning_rate(), dec("0.002"));
}

#[test]
fn apply_off_peak_to_base_costs_preserves_an_explicit_one_hour_rate_of_zero() {
    let off_peak = block(r#"{"hours_utc":"09:00-10:00","cache_creation_input_token_cost":0.09}"#);
    let base_costs = BaseCosts {
        prompt_rate: dec("0.1"),
        completion_rate: dec("0.2"),
        cache_creation_rate: dec("0.03"),
        cache_creation_above_1hr_rate: Some(Decimal::ZERO),
        cache_read_rate: dec("0.01"),
    };
    let resolved =
        apply_off_peak_to_base_costs(Some(&off_peak), moment("2024-06-16T09:30:00Z"), base_costs)
            .unwrap();
    assert_eq!(resolved.cache_creation_rate, dec("0.09"));
    assert_eq!(resolved.cache_creation_above_1hr_rate, Decimal::ZERO);
}

#[test]
fn apply_off_peak_to_base_costs_falls_back_to_the_replaced_cache_creation_rate_when_absent() {
    let off_peak = block(r#"{"hours_utc":"09:00-10:00","cache_creation_input_token_cost":0.09}"#);
    let base_costs = BaseCosts {
        prompt_rate: dec("0.1"),
        completion_rate: dec("0.2"),
        cache_creation_rate: dec("0.03"),
        cache_creation_above_1hr_rate: None,
        cache_read_rate: dec("0.01"),
    };
    let resolved =
        apply_off_peak_to_base_costs(Some(&off_peak), moment("2024-06-16T09:30:00Z"), base_costs)
            .unwrap();
    assert_eq!(resolved.cache_creation_above_1hr_rate, dec("0.09"));
}
