use betterlite_models::controls::json_value::JsonValue;
use rstest::rstest;

use super::super::tokens::{FloatMilliseconds, float_milliseconds};
use super::*;

#[rstest]
#[case::negative("-1500", Some("-1.5"))]
#[case::zero("0", Some("0.0"))]
#[case::true_flag("true", Some("0.001"))]
#[case::false_flag("false", Some("0.0"))]
#[case::fraction("1500.5", Some("1.5005"))]
#[case::numeric_text(r#""1500""#, Some("1.5"))]
#[case::underscored(r#""1_500""#, Some("1.5"))]
#[case::trailing_underscore(r#""1_""#, None)]
#[case::underscore_before_exponent(r#""1_e5""#, None)]
#[case::infinite(r#""inf""#, Some("null"))]
#[case::big("1e30", Some("1e+27"))]
#[case::tiny("1e-30", Some("1e-33"))]
#[case::tiny_at_decimal_scale("1e-28", Some("9.999999999999999e-32"))]
#[case::tiny_rounded_by_decimal("2.5e-28", Some("2.5e-31"))]
#[case::text(r#""abc""#, None)]
#[case::array("[]", None)]
#[case::object("{}", None)]
#[case::null("null", None)]
fn duration_is_best_effort(#[case] value: &str, #[case] expected: Option<&str>) {
    let payload = format!(
        r#"{{"transcription": {{"audio_duration_ms": {value}}}, "transcript": {{"tokens": []}}}}"#
    );
    let stored = response_from(&payload, None).extra.get("duration").cloned();
    let wanted = expected.map(|text| {
        object(&format!(r#"{{"duration": {text}}}"#))
            .remove("duration")
            .expect("the expected duration")
    });
    assert_eq!(stored, wanted, "{value}");
}

#[test]
fn decimal_timings_are_kept_only_when_the_float_division_agrees() {
    assert!(matches!(
        float_milliseconds(&JsonValue::Text("1500".to_owned())),
        Some(FloatMilliseconds::Exact(_))
    ));
    assert!(matches!(
        float_milliseconds(&JsonValue::Text("1e-28".to_owned())),
        Some(FloatMilliseconds::Approximate(_))
    ));
}

#[rstest]
#[case::unique(
    r#"[{"text": "a", "language": "en"}, {"text": "b", "language": "en"}]"#,
    Some("en")
)]
#[case::empty_then_english(
    r#"[{"text": "a", "language": ""}, {"text": "b", "language": "en"}]"#,
    Some("en")
)]
#[case::two(
    r#"[{"text": "a", "language": "en"}, {"text": "b", "language": "de"}]"#,
    None
)]
#[case::empty_only(r#"[{"text": "a", "language": ""}]"#, None)]
#[case::nulls(
    r#"[{"text": "a", "language": null}, {"text": "b", "language": "en"}]"#,
    Some("en")
)]
fn language_is_reported_for_one_truthy_value(#[case] listed: &str, #[case] expected: Option<&str>) {
    let payload = format!(r#"{{"transcript": {{"tokens": {listed}}}}}"#);
    let built = response_from(&payload, None);

    assert_eq!(
        built.extra.get("language").and_then(|value| value.as_str()),
        expected
    );
}

#[test]
fn verbose_words_keep_every_token_and_its_timings() {
    let payload = r#"{"transcript": {"tokens": [
        {"text": "Hello", "start_ms": 0, "end_ms": 500},
        {"text": "Hola", "start_ms": 500, "end_ms": 1000, "translation_status": "translation"},
        {"text": "Rest"},
        {"start_ms": 1500}
    ]}}"#;
    assert_eq!(
        built_object(payload, Some(VERBOSE_JSON)),
        object(
            r#"{
            "text": "HelloHolaRest",
            "usage": null,
            "task": "transcribe",
            "words": [
                {"word": "Hello", "start": 0.0, "end": 0.5},
                {"word": "Hola", "start": 0.5, "end": 1.0},
                {"word": "Rest"},
                {"word": "", "start": 1.5}
            ]
        }"#
        )
    );
}

#[test]
fn verbose_words_accept_boolean_timings() {
    let payload =
        r#"{"transcript": {"tokens": [{"text": "X", "start_ms": true, "end_ms": false}]}}"#;
    assert_eq!(
        built_object(payload, Some(VERBOSE_JSON)),
        object(
            r#"{
            "text": "X",
            "usage": null,
            "task": "transcribe",
            "words": [{"word": "X", "start": 0.001, "end": 0.0}]
        }"#
        )
    );
}

#[test]
fn verbose_words_take_a_timing_beyond_decimal_range() {
    let payload = r#"{"transcript": {"tokens": [{"text": "a", "start_ms": 1e30}]}}"#;
    let expected = r#"{"text": "a", "usage": null, "task": "transcribe", "words": [{"word": "a", "start": 1e+27}]}"#;
    assert_eq!(built_object(payload, Some(VERBOSE_JSON)), object(expected));
}

#[test]
fn verbose_words_divide_a_timing_at_decimal_scale_as_a_float() {
    let payload = r#"{"transcript": {"tokens": [{"text": "a", "start_ms": 1e-28}]}}"#;
    let expected = r#"{"text": "a", "usage": null, "task": "transcribe", "words": [{"word": "a", "start": 9.999999999999999e-32}]}"#;
    assert_eq!(built_object(payload, Some(VERBOSE_JSON)), object(expected));
}

#[test]
fn float_timings_are_ignored_by_subtitles_and_converted_for_words() {
    let payload = r#"{"transcript": {"tokens": [{"text": "a", "start_ms": 1.5, "end_ms": 2.5}]}}"#;
    assert_eq!(
        response_from(payload, Some("json")).text.as_deref(),
        Some("a")
    );
    assert_eq!(
        response_from(payload, Some("srt")).text.as_deref(),
        Some("")
    );
    assert_eq!(
        built_object(payload, Some(VERBOSE_JSON)),
        object(
            r#"{"text": "a", "usage": null, "task": "transcribe", "words": [{"word": "a", "start": 0.0015, "end": 0.0025}]}"#
        )
    );
}

#[test]
fn verbose_words_carry_a_null_text() {
    let payload = r#"{"transcript": {"text": "T", "tokens": [{"text": null, "start_ms": 0, "end_ms": 500}]}}"#;
    assert_eq!(
        built_object(payload, Some(VERBOSE_JSON)),
        object(
            r#"{"text": "T", "usage": null, "task": "transcribe", "words": [{"word": null, "start": 0.0, "end": 0.5}]}"#
        )
    );
}

#[test]
fn verbose_words_convert_numeric_text_and_non_finite_timings() {
    let payload = r#"{"transcript": {"text": "T", "tokens": [{"text": "a", "start_ms": "1.5", "end_ms": "inf"}, {"text": 7, "start_ms": 2, "end_ms": 2500}]}}"#;
    assert_eq!(
        built_object(payload, Some(VERBOSE_JSON)),
        object(
            r#"{"text": "T", "usage": null, "task": "transcribe", "words": [{"word": "a", "start": 0.0015, "end": null}, {"word": 7, "start": 0.002, "end": 2.5}]}"#
        )
    );
}

#[test]
fn unreadable_timings_and_token_texts_fail_the_response() {
    let timing = SonioxTranscriptPayload::try_from(object(
        r#"{"transcript": {"text": "T", "tokens": [{"text": "a", "start_ms": "abc"}]}}"#,
    ))
    .expect("a payload");
    let timing_error =
        build_response_from_payload(&timing, None, Some(VERBOSE_JSON)).expect_err("a timing error");
    assert!(matches!(
        timing_error,
        RecordsError::Timing { field: "words" }
    ));

    let text = SonioxTranscriptPayload::try_from(object(
        r#"{"transcript": {"tokens": [{"text": null}]}}"#,
    ))
    .expect("a payload");
    let text_error = build_response_from_payload(&text, None, None).expect_err("a text error");
    assert!(matches!(
        text_error,
        RecordsError::Text {
            field: "transcript"
        }
    ));
}

#[test]
fn verbose_words_need_a_nonempty_token_list() {
    let payload = r#"{"transcript": {"text": "T", "tokens": []}}"#;
    assert_eq!(
        built_object(payload, Some(VERBOSE_JSON)),
        object(r#"{"text": "T", "usage": null, "task": "transcribe"}"#)
    );
}

#[test]
fn token_numbers_beyond_i64_leave_the_subtitle_and_speaker_domain() {
    assert_eq!(
        srt(&tokens(
            r#"[{"text": "x", "start_ms": 18446744073709551616, "end_ms": 18446744073709551616}]"#
        )),
        ""
    );

    let payload = SonioxTranscriptPayload::try_from(object(
        r#"{"transcript": {"tokens": [{"text": "a", "speaker": 18446744073709551616}]}}"#,
    ))
    .expect("a payload");
    let error = build_response_from_payload(&payload, None, None).expect_err("a speaker error");
    assert!(matches!(
        error,
        RecordsError::Value {
            field: "transcript",
            ..
        }
    ));
}

#[test]
fn verbose_words_render_even_when_the_transcript_text_wins() {
    let payload = r#"{"transcript": {"text": "T", "tokens": [{"text": "a", "start_ms": 1}]}}"#;
    assert_eq!(
        built_object(payload, Some(VERBOSE_JSON)),
        object(
            r#"{
            "text": "T",
            "usage": null,
            "task": "transcribe",
            "words": [{"word": "a", "start": 0.001}]
        }"#
        )
    );
}
