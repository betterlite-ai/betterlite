#![cfg(test)]

use std::collections::BTreeMap;

use chrono::NaiveTime;
use rstest::rstest;
use serde_json::json;

use super::clock::ClockError;
use super::{
    BudgetResetSettings, BudgetResetSettingsError, BudgetResetTimeError, get_budget_reset_settings,
    parse_budget_reset_time,
};
use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::settings::JsonValue;
use crate::types::Config;

fn load(source: &str) -> Result<Config, ConfigError> {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
}

fn object(source: &str) -> JsonValue {
    serde_json::from_str(source).unwrap()
}

fn clock(text: &str) -> Result<NaiveTime, BudgetResetTimeError> {
    parse_budget_reset_time(&json!(text))
}

fn clock_text(time: NaiveTime) -> String {
    time.format("%H:%M:%S").to_string()
}

#[test]
fn immutable_utc_midnight_defaults() {
    let defaults = BudgetResetSettings::default();
    assert_eq!(defaults.timezone, "UTC");
    assert_eq!(defaults.reset_time_of_day, NaiveTime::MIN);
    assert_eq!(
        get_budget_reset_settings(&json!(null), &json!(null)).unwrap(),
        defaults
    );
}

#[rstest]
#[case::null(json!(null))]
#[case::empty_string(json!(""))]
#[case::boolean_false(json!(false))]
#[case::integer_zero(json!(0_i32))]
#[case::float_zero(json!(0.0_f64))]
#[case::empty_array(json!([]))]
#[case::empty_object(object("{}"))]
fn timezone_falsey_defaults_utc(#[case] raw: JsonValue) {
    let settings = get_budget_reset_settings(&raw, &json!(null)).unwrap();
    assert_eq!(settings.timezone, "UTC");
}

#[rstest]
#[case::boolean_true(json!(true))]
#[case::integer_one(json!(1_i32))]
#[case::integer_negative(json!(-1_i32))]
#[case::float(json!(0.5_f64))]
#[case::array(json!(["UTC"]))]
#[case::object(object(r#"{"name": "UTC"}"#))]
fn timezone_truthy_nonstring_rejected(#[case] raw: JsonValue) {
    let error = get_budget_reset_settings(&raw, &json!(null)).unwrap_err();
    assert_eq!(error, BudgetResetSettingsError::TimezoneNotText);
    let source = format!("litellm_settings:\n  timezone: {raw}\n");
    let load_error = load(&source).unwrap_err();
    assert_eq!(load_error.category(), "invalid_scalar");
    assert!(matches!(
        &load_error,
        ConfigError::InvalidScalar { path, .. } if path == "litellm_settings.timezone"
    ));
}

#[rstest]
#[case::canonical("UTC")]
#[case::padded(" Asia/Kolkata ")]
#[case::lowercase("utc")]
#[case::trailing_space("America/New_York ")]
fn timezone_string_not_normalized(#[case] raw: &str) {
    let settings = get_budget_reset_settings(&json!(raw), &json!(null)).unwrap();
    assert_eq!(settings.timezone, raw);
}

#[rstest]
#[case::null(json!(null))]
#[case::empty_string(json!(""))]
fn reset_time_none_or_empty_midnight(#[case] raw: JsonValue) {
    let settings = get_budget_reset_settings(&json!("UTC"), &raw).unwrap();
    assert_eq!(settings.reset_time_of_day, NaiveTime::MIN);
}

#[rstest]
#[case::boolean_false(json!(false))]
#[case::boolean_true(json!(true))]
#[case::integer_zero(json!(0_i32))]
#[case::integer_twelve(json!(12_i32))]
#[case::float(json!(1.5_f64))]
#[case::array(json!([]))]
#[case::object(object("{}"))]
fn reset_time_nonstring_rejected(#[case] raw: JsonValue) {
    let error = get_budget_reset_settings(&json!("UTC"), &raw).unwrap_err();
    assert_eq!(
        error,
        BudgetResetSettingsError::ResetTime(BudgetResetTimeError::NotText)
    );
    assert!(error.to_string().contains("quoted 24-hour"));
    assert!(error.to_string().contains("HH:MM"));
}

#[rstest]
#[case::unpadded_hours("12:00", "12:00:00")]
#[case::unpadded_minutes("12:5", "12:05:00")]
#[case::unpadded_seconds("12:00:9", "12:00:09")]
#[case::padded_hms("01:02:03", "01:02:03")]
#[case::midnight("00:00", "00:00:00")]
#[case::last_second("23:59:59", "23:59:59")]
fn padded_hm_and_hms(#[case] raw: &str, #[case] expected: &str) {
    assert_eq!(clock_text(clock(raw).unwrap()), expected);
}

#[rstest]
#[case::hm("1:2", "01:02:00")]
#[case::hms("1:2:3", "01:02:03")]
#[case::mixed_padding("7:05:9", "07:05:09")]
#[case::single_digits("0:0", "00:00:00")]
fn unpadded_components(#[case] raw: &str, #[case] expected: &str) {
    assert_eq!(clock_text(clock(raw).unwrap()), expected);
}

#[rstest]
#[case::arabic_indic("\u{667}:\u{665}", Some("07:05:00"))]
#[case::extended_arabic_indic("\u{6f7}:\u{6f5}", Some("07:05:00"))]
#[case::devanagari("\u{96c}:\u{96b}", Some("06:05:00"))]
#[case::fullwidth_single("\u{ff17}:\u{ff15}", Some("07:05:00"))]
#[case::ascii_lead_arabic_tail("1\u{662}:30", Some("12:30:00"))]
#[case::ascii_lead_arabic_seconds("12:0\u{665}", Some("12:05:00"))]
#[case::arabic_two_digit_hour("\u{661}\u{662}:\u{663}\u{660}", None)]
#[case::devanagari_two_digit_hour("\u{967}\u{968}:\u{969}\u{96b}", None)]
#[case::arabic_two_digit_minute("12:\u{663}\u{660}", None)]
#[case::hour_two_arabic_tail("2\u{663}:00", None)]
#[case::superscript_is_not_a_decimal_digit("\u{b2}:00", None)]
#[case::circled_is_not_a_decimal_digit("\u{2460}:00", None)]
fn source_unicode_digit_acceptance(#[case] raw: &str, #[case] expected: Option<&str>) {
    match (clock(raw), expected) {
        (Ok(time), Some(text)) => assert_eq!(clock_text(time), text),
        (Err(BudgetResetTimeError::Clock(ClockError::Malformed)), None) => {}
        (other, _) => panic!("unexpected clock result for {raw}: {other:?}"),
    }
}

#[rstest]
#[case::hour_24("24:00")]
#[case::hour_25("25:00")]
#[case::minute_60("12:60")]
#[case::minute_99("12:99")]
#[case::second_60("12:00:60")]
#[case::leap_second("23:59:60")]
#[case::hour_99("99:00")]
fn clock_range_and_leap_second_rejection(#[case] raw: &str) {
    assert_eq!(
        clock(raw).unwrap_err(),
        BudgetResetTimeError::Clock(ClockError::Malformed)
    );
}

#[rstest]
#[case::leading_space(" 12:00")]
#[case::trailing_space("12:00 ")]
#[case::leading_tab("\t12:00")]
#[case::meridiem("12:30pm")]
#[case::meridiem_spaced("12:30 PM")]
#[case::utc_suffix("12:00Z")]
#[case::hms_utc_suffix("12:00:00Z")]
#[case::offset_suffix("12:00+05:00")]
#[case::hms_offset_suffix("12:00:00-05:00")]
#[case::zone_suffix("12:00:00 EST")]
fn clock_whitespace_and_suffix_rejection(#[case] raw: &str) {
    assert_eq!(
        clock(raw).unwrap_err(),
        BudgetResetTimeError::Clock(ClockError::Malformed)
    );
}

#[rstest]
#[case::fraction("12:00:00.5")]
#[case::long_fraction("12:00:00.000000")]
#[case::fourth_component("12:00:00:00")]
#[case::trailing_colon("12:00:")]
#[case::double_colon("12::00")]
#[case::leading_colon(":12:00")]
#[case::no_components("12")]
#[case::sign("+12:00")]
#[case::negative("-12:00")]
fn clock_fraction_timezone_and_extra_component_rejection(#[case] raw: &str) {
    assert_eq!(
        clock(raw).unwrap_err(),
        BudgetResetTimeError::Clock(ClockError::Malformed)
    );
}

#[test]
fn loads_timezone_and_reset_time() {
    let config =
        load("litellm_settings:\n  timezone: Asia/Kolkata\n  budget_reset_time: '6:5'\n").unwrap();
    assert_eq!(config.sections.budget_reset.timezone, "Asia/Kolkata");
    assert_eq!(
        config.sections.budget_reset.reset_time_of_day,
        NaiveTime::from_hms_opt(6, 5, 0).unwrap()
    );
}

#[test]
fn omitted_reset_settings_defaults() {
    let config = load("{}\n").unwrap();
    assert_eq!(config.sections.budget_reset, BudgetResetSettings::default());
}

#[test]
fn reset_settings_env_reference() {
    let lookup = BTreeMap::from([
        ("RESET_ZONE".to_owned(), "America/New_York".to_owned()),
        ("RESET_CLOCK".to_owned(), "7:30".to_owned()),
    ]);
    let config = ConfigLoader { lookup }
        .load("litellm_settings:\n  timezone: os.environ/RESET_ZONE\n  budget_reset_time: os.environ/RESET_CLOCK\n")
        .unwrap();
    assert_eq!(config.sections.budget_reset.timezone, "America/New_York");
    assert_eq!(
        config.sections.budget_reset.reset_time_of_day,
        NaiveTime::from_hms_opt(7, 30, 0).unwrap()
    );
}

#[test]
fn malformed_clock_stops_startup() {
    let error = load("litellm_settings:\n  budget_reset_time: '25:00'\n").unwrap_err();
    assert_eq!(error.category(), "invalid_scalar");
    assert!(matches!(
        &error,
        ConfigError::InvalidScalar { path, .. } if path == "litellm_settings.budget_reset_time"
    ));
}

#[test]
fn constructor_settings_not_first_global_read() {
    let first = get_budget_reset_settings(&json!("UTC"), &json!("06:00")).unwrap();
    let second = get_budget_reset_settings(&json!("Asia/Kolkata"), &json!("18:30")).unwrap();
    assert_ne!(first, second);
    assert_eq!(
        first,
        get_budget_reset_settings(&json!("UTC"), &json!("06:00")).unwrap()
    );
    let first_config = load("litellm_settings:\n  budget_reset_time: '06:00'\n").unwrap();
    let second_config = load("litellm_settings:\n  budget_reset_time: '18:30'\n").unwrap();
    assert_eq!(
        first_config.sections.budget_reset.reset_time_of_day,
        NaiveTime::from_hms_opt(6, 0, 0).unwrap()
    );
    assert_eq!(
        second_config.sections.budget_reset.reset_time_of_day,
        NaiveTime::from_hms_opt(18, 30, 0).unwrap()
    );
}

#[test]
fn clock_error_does_not_echo_input() {
    let raw = "25:99 do-not-echo";
    let error = parse_budget_reset_time(&json!(raw)).unwrap_err();
    assert_eq!(error, BudgetResetTimeError::Clock(ClockError::Malformed));
    assert!(!error.to_string().contains(raw));
    assert!(!error.to_string().contains("do-not-echo"));
    let source = format!("litellm_settings:\n  budget_reset_time: '{raw}'\n");
    let load_error = load(&source).unwrap_err();
    assert_eq!(load_error.category(), "invalid_scalar");
    assert!(!load_error.to_string().contains("do-not-echo"));
}
