use betterlite_openai::openai_base_config::Object;
use rstest::rstest;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;

use super::super::error::PrepareError;
use super::super::options::{
    CLEANUP_FIELD, HandlerOptions, MAX_ATTEMPTS_FIELD, POLL_INTERVAL_FIELD, extract_options,
};

fn params(text: &str) -> Object {
    serde_json::from_str(text).expect("the case params are a JSON object")
}

fn options_of(text: &str) -> Result<HandlerOptions, PrepareError> {
    let mut params = params(text);
    extract_options(&mut params)
}

fn interval_of(text: &str) -> Decimal {
    options_of(text)
        .expect("the case interval is accepted")
        .poll_interval
}

fn attempts_of(text: &str) -> i64 {
    options_of(text)
        .expect("the case attempt count is accepted")
        .max_attempts
}

#[rstest]
#[case::absent("{}", 1.0_f64)]
#[case::zero("{\"soniox_polling_interval\": 0}", 0.5_f64)]
#[case::below_minimum("{\"soniox_polling_interval\": 0.4}", 0.5_f64)]
#[case::minimum("{\"soniox_polling_interval\": 0.5}", 0.5_f64)]
#[case::maximum("{\"soniox_polling_interval\": 60}", 60.0_f64)]
#[case::above_maximum("{\"soniox_polling_interval\": 60.5}", 60.0_f64)]
#[case::negative("{\"soniox_polling_interval\": -3}", 0.5_f64)]
#[case::numeric_text("{\"soniox_polling_interval\": \"2.5\"}", 2.5_f64)]
#[case::padded_text("{\"soniox_polling_interval\": \" 3 \"}", 3.0_f64)]
#[case::underscored_text("{\"soniox_polling_interval\": \"1_0\"}", 10.0_f64)]
#[case::leading_dot("{\"soniox_polling_interval\": \".5\"}", 0.5_f64)]
#[case::trailing_dot("{\"soniox_polling_interval\": \"5.\"}", 5.0_f64)]
#[case::signed_leading_dot("{\"soniox_polling_interval\": \"-.5\"}", 0.5_f64)]
#[case::finite_expansion("{\"soniox_polling_interval\": 5e999}", 1.0_f64)]
#[case::infinite_text("{\"soniox_polling_interval\": \"inf\"}", 1.0_f64)]
#[case::negative_infinite_text("{\"soniox_polling_interval\": \"-infinity\"}", 1.0_f64)]
#[case::nan_text("{\"soniox_polling_interval\": \"nan\"}", 1.0_f64)]
fn clamps_the_poll_interval(#[case] input: &str, #[case] seconds: f64) {
    let expected = Decimal::from_f64(seconds).expect("the expected interval is a decimal");

    assert_eq!(interval_of(input), expected);
}

#[rstest]
#[case::null("{\"soniox_polling_interval\": null}")]
#[case::invalid_text("{\"soniox_polling_interval\": \"abc\"}")]
#[case::empty_text("{\"soniox_polling_interval\": \"\"}")]
#[case::blank_text("{\"soniox_polling_interval\": \"   \"}")]
#[case::lone_dot("{\"soniox_polling_interval\": \".\"}")]
#[case::dot_exponent("{\"soniox_polling_interval\": \".e3\"}")]
#[case::lone_exponent("{\"soniox_polling_interval\": \"e5\"}")]
#[case::true_flag("{\"soniox_polling_interval\": true}")]
#[case::list("{\"soniox_polling_interval\": [1]}")]
#[case::object("{\"soniox_polling_interval\": {\"seconds\": 1}}")]
fn refuses_an_interval_that_cannot_convert(#[case] input: &str) {
    let error = options_of(input).expect_err("the interval is refused");
    assert!(error.to_string().contains(POLL_INTERVAL_FIELD), "{error}");
}

#[test]
fn refuses_an_interval_from_an_integer_past_f64() {
    let mut digits = String::from("1");
    digits.push_str(&String::from("0").repeat(400));
    let input = format!("{{\"soniox_polling_interval\": {digits}}}");

    let error = options_of(&input).expect_err("the integer does not fit an f64");

    assert!(error.to_string().contains(POLL_INTERVAL_FIELD), "{error}");
}

#[rstest]
#[case::absent("{}", 1800)]
#[case::zero("{\"soniox_max_polling_attempts\": 0}", 1)]
#[case::minimum("{\"soniox_max_polling_attempts\": 1}", 1)]
#[case::maximum("{\"soniox_max_polling_attempts\": 6000}", 6000)]
#[case::above_maximum("{\"soniox_max_polling_attempts\": 6001}", 6000)]
#[case::negative("{\"soniox_max_polling_attempts\": -5}", 1)]
#[case::numeric_text("{\"soniox_max_polling_attempts\": \"42\"}", 42)]
#[case::invalid_text("{\"soniox_max_polling_attempts\": \"abc\"}", 1800)]
#[case::exponent_text("{\"soniox_max_polling_attempts\": \"1e5\"}", 1800)]
#[case::decimal_text("{\"soniox_max_polling_attempts\": \"1.5\"}", 1800)]
#[case::padded_text("{\"soniox_max_polling_attempts\": \" 12 \"}", 12)]
#[case::underscored_text("{\"soniox_max_polling_attempts\": \"1_0\"}", 10)]
#[case::leading_zeros("{\"soniox_max_polling_attempts\": \"007\"}", 7)]
#[case::signed_text("{\"soniox_max_polling_attempts\": \"+5\"}", 5)]
#[case::negative_text("{\"soniox_max_polling_attempts\": \"-5\"}", 1)]
#[case::leading_underscore("{\"soniox_max_polling_attempts\": \"_5\"}", 1800)]
#[case::trailing_underscore("{\"soniox_max_polling_attempts\": \"5_\"}", 1800)]
#[case::double_underscore("{\"soniox_max_polling_attempts\": \"1__0\"}", 1800)]
#[case::non_ascii_digits("{\"soniox_max_polling_attempts\": \"\u{661}\u{662}\"}", 1800)]
#[case::infinite_text("{\"soniox_max_polling_attempts\": \"inf\"}", 1800)]
#[case::fraction("{\"soniox_max_polling_attempts\": 3.9}", 3)]
#[case::negative_fraction("{\"soniox_max_polling_attempts\": -2.7}", 1)]
#[case::overflowing_float("{\"soniox_max_polling_attempts\": 1e400}", 1800)]
fn converts_the_attempt_count(#[case] input: &str, #[case] expected: i64) {
    assert_eq!(attempts_of(input), expected);
}

#[rstest]
#[case::null("{\"soniox_max_polling_attempts\": null}")]
#[case::true_flag("{\"soniox_max_polling_attempts\": true}")]
#[case::false_flag("{\"soniox_max_polling_attempts\": false}")]
#[case::list("{\"soniox_max_polling_attempts\": [1]}")]
#[case::object("{\"soniox_max_polling_attempts\": {\"count\": 1}}")]
fn refuses_an_attempt_count_that_cannot_convert(#[case] input: &str) {
    let error = options_of(input).expect_err("the attempt count is refused");
    assert!(error.to_string().contains(MAX_ATTEMPTS_FIELD), "{error}");
}

#[test]
fn clamps_an_attempt_count_from_an_integer_past_i64() {
    let digits = String::from("9").repeat(40);
    let positive = format!("{{\"soniox_max_polling_attempts\": {digits}}}");
    let negative = format!("{{\"soniox_max_polling_attempts\": -{digits}}}");

    assert_eq!(attempts_of(&positive), 6000);
    assert_eq!(attempts_of(&negative), 1);
}

#[rstest]
#[case::absent("{}", &["file", "transcription"][..])]
#[case::null("{\"soniox_cleanup\": null}", &[][..])]
#[case::empty_list("{\"soniox_cleanup\": []}", &[][..])]
#[case::one_text("{\"soniox_cleanup\": \"file\"}", &["file"][..])]
#[case::default_pair(
    "{\"soniox_cleanup\": [\"file\", \"transcription\"]}",
    &["file", "transcription"][..]
)]
#[case::custom("{\"soniox_cleanup\": [\"custom\"]}", &["custom"][..])]
fn collects_the_cleanup_steps(#[case] input: &str, #[case] expected: &[&str]) {
    let options = options_of(input).expect("the cleanup value is accepted");
    let cleanup: Vec<&str> = options.cleanup.iter().map(String::as_str).collect();

    assert_eq!(cleanup, expected);
}

#[rstest]
#[case::number("{\"soniox_cleanup\": 1}")]
#[case::true_flag("{\"soniox_cleanup\": true}")]
#[case::object("{\"soniox_cleanup\": {\"step\": \"file\"}}")]
#[case::mixed_list("{\"soniox_cleanup\": [\"file\", 2]}")]
#[case::null_entry("{\"soniox_cleanup\": [null]}")]
fn refuses_cleanup_that_is_not_names(#[case] input: &str) {
    let error = options_of(input).expect_err("the cleanup value is refused");
    assert!(error.to_string().contains(CLEANUP_FIELD), "{error}");
}

#[test]
fn keeps_the_text_options() {
    let input = r#"{"filename": "clip.wav", "audio_url": "https://a.test/x.wav", "file_id": "file_1", "response_format": "verbose_json"}"#;

    let options = options_of(input).expect("the text options are accepted");

    assert_eq!(options.filename_override.as_deref(), Some("clip.wav"));
    assert_eq!(options.audio_url.as_deref(), Some("https://a.test/x.wav"));
    assert_eq!(options.file_id.as_deref(), Some("file_1"));
    assert_eq!(options.response_format.as_deref(), Some("verbose_json"));
}

#[rstest]
#[case::absent("{}")]
#[case::null(
    "{\"filename\": null, \"audio_url\": null, \"file_id\": null, \"response_format\": null}"
)]
fn treats_absent_and_null_text_options_as_none(#[case] input: &str) {
    let options = options_of(input).expect("the text options are accepted");

    assert_eq!(options.filename_override, None);
    assert_eq!(options.audio_url, None);
    assert_eq!(options.file_id, None);
    assert_eq!(options.response_format, None);
}

#[rstest]
#[case::filename("filename", "{\"filename\": 1}")]
#[case::audio_url("audio_url", "{\"audio_url\": 2}")]
#[case::file_id("file_id", "{\"file_id\": true}")]
#[case::response_format("response_format", "{\"response_format\": []}")]
fn refuses_a_text_option_that_is_not_text(#[case] field: &str, #[case] input: &str) {
    let error = options_of(input).expect_err("the text option is refused");
    let message = error.to_string();

    assert!(message.contains(field), "{message}");
}

#[test]
fn removes_language_and_keeps_the_rest() {
    let mut params = params(
        r#"{"language": "en", "language_hints": ["en"], "context": {"topic": "a"}, "client_reference_id": null, "model": "stt-async-v4"}"#,
    );

    let options = extract_options(&mut params).expect("the params are accepted");

    assert!(!params.contains_key("language"));
    assert!(params.contains_key("language_hints"));
    assert!(params.contains_key("context"));
    assert!(params.contains_key("client_reference_id"));
    assert!(params.contains_key("model"));
    assert_eq!(options.max_attempts, 1800);
}

#[test]
fn reports_the_interval_before_the_attempts() {
    let input = r#"{"soniox_polling_interval": "abc", "soniox_max_polling_attempts": null}"#;

    let error = options_of(input).expect_err("both values are refused");

    assert!(error.to_string().contains(POLL_INTERVAL_FIELD), "{error}");
}
