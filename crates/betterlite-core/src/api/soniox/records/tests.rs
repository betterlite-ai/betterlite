#![cfg(test)]

use betterlite_openai::openai_base_config::{Headers, Object};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{from_value, to_value};

use super::auth::{
    MISSING_KEY, resolve_api_base, resolve_api_key, soniox_exception, validate_environment,
};
use super::error::RecordsError;
use super::params::map_openai_params;
use super::response::{
    SonioxTranscriptPayload, SonioxTranscriptionResponse, TRANSCRIBE_TASK, VERBOSE_JSON,
    build_response_from_payload,
};
use super::tokens::{
    SonioxToken, render_soniox_tokens, render_soniox_tokens_as_srt, render_soniox_tokens_as_vtt,
};
use super::{
    SONIOX_DEFAULT_ATTEMPTS, SONIOX_DEFAULT_CLEANUP, SONIOX_DEFAULT_POLL_INTERVAL,
    SONIOX_HANDLER_ONLY_PARAMS, SONIOX_MAX_ATTEMPTS, SONIOX_MAX_POLL_INTERVAL,
    SONIOX_MIN_POLL_INTERVAL, SONIOX_PASSTHROUGH_PARAMS, SUPPORTED_OPENAI_PARAMS,
};

fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("a JSON object")
}

fn tokens(text: &str) -> Vec<SonioxToken> {
    serde_json::from_str(text).expect("typed tokens")
}

fn mapped(non_default: &str, optional: &str) -> Object {
    let mut options = object(optional);
    map_openai_params(&object(non_default), &mut options).expect("mapped params");
    options
}

fn map_error(non_default: &str, optional: &str) -> RecordsError {
    let mut options = object(optional);
    map_openai_params(&object(non_default), &mut options).expect_err("a rejected option")
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value)
}

fn headers(entries: &[(&str, &str)]) -> Headers {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn validated(
    explicit: Option<&str>,
    environment: Option<&str>,
    caller: &[(&str, &str)],
) -> Headers {
    let resolved = environment.map(secret);
    validate_environment(headers(caller), explicit, resolved.as_ref()).expect("validated headers")
}

fn response_from(payload: &str, response_format: Option<&str>) -> SonioxTranscriptionResponse {
    let parsed = SonioxTranscriptPayload::try_from(object(payload)).expect("a payload");
    build_response_from_payload(&parsed, None, response_format).expect("a response")
}

fn built_object(payload: &str, response_format: Option<&str>) -> Object {
    let encoded = to_value(response_from(payload, response_format)).expect("a response object");
    from_value(encoded).expect("a JSON object")
}

#[test]
fn supported_params_are_language_and_response_format() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["language", "response_format"]);
}

#[test]
fn passthrough_and_handler_only_params_match_the_source() {
    assert_eq!(
        SONIOX_PASSTHROUGH_PARAMS,
        [
            "language_hints",
            "language_hints_strict",
            "enable_language_identification",
            "enable_speaker_diarization",
            "context",
            "translation",
            "client_reference_id",
            "webhook_url",
            "webhook_auth_header_name",
            "webhook_auth_header_value",
            "audio_url",
            "file_id",
        ]
    );
    assert_eq!(
        SONIOX_HANDLER_ONLY_PARAMS,
        [
            "soniox_polling_interval",
            "soniox_max_polling_attempts",
            "soniox_cleanup",
            "filename",
        ]
    );
}

#[test]
fn polling_and_cleanup_constants_match_the_source() {
    assert_eq!(SONIOX_DEFAULT_POLL_INTERVAL.to_bits(), 1.0_f64.to_bits());
    assert_eq!(SONIOX_MIN_POLL_INTERVAL.to_bits(), 0.5_f64.to_bits());
    assert_eq!(SONIOX_MAX_POLL_INTERVAL.to_bits(), 60.0_f64.to_bits());
    assert_eq!(SONIOX_DEFAULT_ATTEMPTS, 1800);
    assert_eq!(SONIOX_MAX_ATTEMPTS, 6000);
    assert_eq!(SONIOX_DEFAULT_CLEANUP, ["file", "transcription"]);
}

#[test]
fn a_truthy_language_becomes_the_first_hint() {
    assert_eq!(
        mapped(r#"{"language": "en"}"#, "{}"),
        object(r#"{"language_hints": ["en"]}"#)
    );
    assert_eq!(
        mapped(r#"{"language": "en"}"#, r#"{"language_hints": ["de"]}"#),
        object(r#"{"language_hints": ["en", "de"]}"#)
    );
}

#[test]
fn an_existing_hint_is_not_reordered() {
    assert_eq!(
        mapped(
            r#"{"language": "de"}"#,
            r#"{"language_hints": ["de", "fr"]}"#
        ),
        object(r#"{"language_hints": ["de", "fr"]}"#)
    );
}

#[test]
fn empty_existing_hints_are_replaced() {
    assert_eq!(
        mapped(r#"{"language": "en"}"#, r#"{"language_hints": []}"#),
        object(r#"{"language_hints": ["en"]}"#)
    );
    assert_eq!(
        mapped(r#"{"language": "en"}"#, r#"{"language_hints": null}"#),
        object(r#"{"language_hints": ["en"]}"#)
    );
}

#[test]
fn native_hints_in_the_same_call_override_the_language_merge() {
    assert_eq!(
        mapped(
            r#"{"language": "en", "language_hints": ["de", "fr"]}"#,
            "{}"
        ),
        object(r#"{"language_hints": ["de", "fr"]}"#)
    );
}

#[test]
fn a_null_native_hint_does_not_override_the_language_merge() {
    assert_eq!(
        mapped(r#"{"language": "en", "language_hints": null}"#, "{}"),
        object(r#"{"language_hints": ["en"]}"#)
    );
}

#[rstest]
#[case::null(r#"{"language": null}"#)]
#[case::empty(r#"{"language": ""}"#)]
#[case::absent("{}")]
fn a_falsy_language_leaves_the_hints_alone(#[case] non_default: &str) {
    assert_eq!(
        mapped(non_default, r#"{"language_hints": ["de"]}"#),
        object(r#"{"language_hints": ["de"]}"#)
    );
}

#[test]
fn response_format_is_copied_by_presence() {
    assert_eq!(
        mapped(r#"{"response_format": "srt"}"#, "{}"),
        object(r#"{"response_format": "srt"}"#)
    );
    assert_eq!(
        mapped(r#"{"response_format": null}"#, "{}"),
        object(r#"{"response_format": null}"#)
    );
    assert_eq!(mapped("{}", "{}"), object("{}"));
}

#[test]
fn passthrough_values_are_copied_unchanged() {
    let params = r#"{
        "language_hints_strict": true,
        "enable_language_identification": false,
        "enable_speaker_diarization": true,
        "context": {"terms": ["Soniox"]},
        "translation": {"type": "one_way", "target_language": "es"},
        "client_reference_id": "ref",
        "webhook_url": "https://hook.test",
        "webhook_auth_header_name": "X-Hook",
        "webhook_auth_header_value": "value",
        "audio_url": "https://audio.test/a.mp3",
        "file_id": "file-1"
    }"#;
    assert_eq!(mapped(params, "{}"), object(params));
}

#[test]
fn null_passthrough_values_are_skipped() {
    assert_eq!(
        mapped(r#"{"webhook_url": null, "translation": null}"#, "{}"),
        object("{}")
    );
}

#[test]
fn handler_only_values_are_copied_unchanged() {
    let params = r#"{
        "soniox_polling_interval": 2,
        "soniox_max_polling_attempts": 5,
        "soniox_cleanup": ["file"],
        "filename": "a.mp3"
    }"#;
    assert_eq!(mapped(params, "{}"), object(params));
}

#[test]
fn a_wrong_language_type_names_the_field() {
    let error = map_error(r#"{"language": 5}"#, "{}");
    assert!(matches!(
        error,
        RecordsError::Value {
            field: "language",
            ..
        }
    ));
}

#[test]
fn wrong_existing_hints_name_the_field() {
    let error = map_error(r#"{"language": "en"}"#, r#"{"language_hints": "de"}"#);
    assert!(matches!(
        error,
        RecordsError::Value {
            field: "language_hints",
            ..
        }
    ));
}

#[test]
fn unrelated_optional_params_stay_in_place() {
    assert_eq!(
        mapped(
            r#"{"language": "en"}"#,
            r#"{"custom": 1, "language_hints": ["de"]}"#
        ),
        object(r#"{"custom": 1, "language_hints": ["en", "de"]}"#)
    );
}

#[test]
fn explicit_keys_precede_the_environment() {
    let environment = secret("env-key");
    assert_eq!(
        resolve_api_key(Some("explicit"), Some(&environment)),
        Some("explicit".to_owned())
    );
}

#[test]
fn an_empty_explicit_key_falls_back_to_the_environment() {
    let environment = secret("env-key");
    assert_eq!(
        resolve_api_key(Some(""), Some(&environment)),
        Some("env-key".to_owned())
    );
}

#[test]
fn empty_keys_resolve_to_nothing() {
    let environment = secret("");
    assert_eq!(resolve_api_key(Some(""), Some(&environment)), None);
    assert_eq!(resolve_api_key(None, Some(&environment)), None);
    assert_eq!(resolve_api_key(None, None), None);
}

#[test]
fn bearer_authorization_is_built_before_caller_headers() {
    let result = validated(Some("explicit"), None, &[("X-Trace", "1")]);

    assert_eq!(
        result.get("Authorization").map(String::as_str),
        Some("Bearer explicit")
    );
    assert_eq!(result.get("X-Trace").map(String::as_str), Some("1"));
}

#[test]
fn a_caller_authorization_wins() {
    let result = validated(
        None,
        Some("env-key"),
        &[("Authorization", "caller"), ("X-Trace", "1")],
    );

    assert_eq!(
        result.get("Authorization").map(String::as_str),
        Some("caller")
    );
    assert_eq!(result.get("X-Trace").map(String::as_str), Some("1"));
}

#[test]
fn a_missing_key_is_a_401_with_the_source_message() {
    let error = validate_environment(Headers::new(), None, None).expect_err("a missing Soniox key");

    assert_eq!(error.status_code, 401);
    assert_eq!(error.message, MISSING_KEY);
    assert!(error.headers.is_none());
}

#[test]
fn empty_keys_are_missing_keys() {
    let environment = secret("");
    let error = validate_environment(Headers::new(), Some(""), Some(&environment))
        .expect_err("a missing Soniox key");

    assert_eq!(error.status_code, 401);
}

#[test]
fn an_explicit_base_precedes_the_environment_and_loses_slashes() {
    assert_eq!(
        resolve_api_base(Some("https://explicit.test///"), "https://env.test"),
        "https://explicit.test"
    );
    assert_eq!(
        resolve_api_base(Some(""), "https://env.test/"),
        "https://env.test"
    );
    assert_eq!(
        resolve_api_base(None, "https://env.test//"),
        "https://env.test"
    );
}

#[test]
fn errors_keep_their_message_status_and_headers() {
    let error = soniox_exception("boom", 429, Some(headers(&[("X-A", "1")])));

    assert_eq!(error.message, "boom");
    assert_eq!(error.status_code, 429);
    assert_eq!(
        error
            .headers
            .as_ref()
            .and_then(|values| values.get("X-A"))
            .map(String::as_str),
        Some("1")
    );
}

#[test]
fn plain_tokens_are_concatenated() {
    assert_eq!(
        render_soniox_tokens(&tokens(r#"[{"text": "a"}, {"text": "b"}]"#)),
        "ab"
    );
}

#[test]
fn speakers_emit_tags_and_later_changes_two_newlines() {
    let rendered = render_soniox_tokens(&tokens(
        r#"[
            {"text": "Hello", "speaker": 1, "language": "en"},
            {"text": " world", "speaker": 1, "language": "en"},
            {"text": "Hola", "speaker": 2, "language": "es"}
        ]"#,
    ));
    assert_eq!(
        rendered,
        "Speaker 1:\n[en] Hello world\n\nSpeaker 2:\n[es] Hola"
    );
}

#[test]
fn a_boolean_speaker_labels_as_python_and_equals_one() {
    let rendered = render_soniox_tokens(&tokens(
        r#"[{"text": "A", "speaker": true, "language": "en"}, {"text": "B", "speaker": 1, "language": "en"}]"#,
    ));
    assert_eq!(rendered, "Speaker True:\n[en] AB");
}

#[test]
fn a_text_speaker_keeps_its_text() {
    let rendered = render_soniox_tokens(&tokens(
        r#"[{"text": "A", "speaker": "S1"}, {"text": "B", "speaker": "S1"}]"#,
    ));
    assert_eq!(rendered, "Speaker S1:AB");
}

#[test]
fn a_first_language_tag_keeps_the_token_leading_space() {
    assert_eq!(
        render_soniox_tokens(&tokens(r#"[{"text": " hi", "language": "en"}]"#)),
        "\n[en]  hi"
    );
    assert_eq!(render_soniox_tokens(&tokens(r#"[{"text": " hi"}]"#)), " hi");
}

#[test]
fn a_language_change_inserts_a_tag_and_strips_the_token() {
    assert_eq!(
        render_soniox_tokens(&tokens(
            r#"[{"text": "a", "language": "en"}, {"text": " b", "language": "de"}]"#
        )),
        "\n[en] a\n[de] b"
    );
}

#[test]
fn a_language_tag_strips_python_whitespace_separators() {
    assert_eq!(
        render_soniox_tokens(&tokens(
            r#"[{"text": "a", "language": "en"}, {"text": "\u001c b", "language": "de"}]"#
        )),
        "\n[en] a\n[de] b"
    );
}

#[test]
fn a_translation_tag_prefixes_a_new_language() {
    assert_eq!(
        render_soniox_tokens(&tokens(
            r#"[{"text": " hola", "language": "es", "translation_status": "translation"}]"#
        )),
        "\n[Translation] [es] hola"
    );
}

#[test]
fn a_translation_in_the_same_language_adds_no_tag() {
    assert_eq!(
        render_soniox_tokens(&tokens(
            r#"[
                {"text": "hi", "language": "en"},
                {"text": " hola", "language": "en", "translation_status": "translation"}
            ]"#
        )),
        "\n[en] hi hola"
    );
}

#[test]
fn a_speaker_change_resets_the_language() {
    assert_eq!(
        render_soniox_tokens(&tokens(
            r#"[
                {"text": " a", "speaker": 1, "language": "en"},
                {"text": " b", "speaker": 2, "language": "en"}
            ]"#
        )),
        "Speaker 1:\n[en] a\n\nSpeaker 2:\n[en] b"
    );
}

#[test]
fn an_empty_language_is_still_a_language() {
    assert_eq!(
        render_soniox_tokens(&tokens(r#"[{"text": "a", "language": ""}]"#)),
        "\n[] a"
    );
}

#[test]
fn null_speakers_and_languages_are_ignored() {
    assert_eq!(
        render_soniox_tokens(&tokens(
            r#"[{"text": "a", "speaker": null, "language": null}]"#
        )),
        "a"
    );
}

#[test]
fn srt_renders_timed_tokens() {
    assert_eq!(
        render_soniox_tokens_as_srt(&tokens(
            r#"[{"text": "Hel", "start_ms": 0, "end_ms": 500}, {"text": "lo", "start_ms": 500, "end_ms": 1000}]"#
        )),
        "1\n00:00:00,000 --> 00:00:01,000\nHello\n"
    );
}

#[test]
fn translation_tokens_are_dropped_from_subtitles() {
    assert_eq!(
        render_soniox_tokens_as_srt(&tokens(
            r#"[{"text": " hola", "start_ms": 5, "language": "es", "translation_status": "translation"}]"#
        )),
        ""
    );
    assert_eq!(
        render_soniox_tokens_as_vtt(&tokens(
            r#"[{"text": " hola", "start_ms": 5, "language": "es", "translation_status": "translation"}]"#
        )),
        "WEBVTT\n"
    );
}

#[test]
fn mixed_subtitles_keep_only_the_original_language() {
    assert_eq!(
        render_soniox_tokens_as_srt(&tokens(
            r#"[
                {"text": "Hi", "start_ms": 0, "end_ms": 500},
                {"text": " hola", "start_ms": 500, "end_ms": 900, "translation_status": "translation"}
            ]"#
        )),
        "1\n00:00:00,000 --> 00:00:00,500\nHi\n"
    );
}

#[test]
fn a_null_translation_status_keeps_the_token() {
    assert_eq!(
        render_soniox_tokens_as_srt(&tokens(
            r#"[{"text": "Hi", "start_ms": 0, "end_ms": 500, "translation_status": null}]"#
        )),
        "1\n00:00:00,000 --> 00:00:00,500\nHi\n"
    );
}

#[test]
fn boolean_timings_are_python_integers() {
    assert_eq!(
        render_soniox_tokens_as_srt(&tokens(
            r#"[{"text": "X", "start_ms": true, "end_ms": false}]"#
        )),
        "1\n00:00:00,001 --> 00:00:00,000\nX\n"
    );
}

#[test]
fn untimed_tokens_render_no_srt_but_the_vtt_header() {
    assert_eq!(
        render_soniox_tokens_as_srt(&tokens(r#"[{"text": "X"}]"#)),
        ""
    );
    assert_eq!(
        render_soniox_tokens_as_vtt(&tokens(r#"[{"text": "X"}]"#)),
        "WEBVTT\n"
    );
}

#[test]
fn a_boolean_speaker_does_not_split_a_cue_from_one() {
    assert_eq!(
        render_soniox_tokens_as_srt(&tokens(
            r#"[
                {"text": "a", "start_ms": 0, "end_ms": 100, "speaker": true},
                {"text": "b", "start_ms": 150, "end_ms": 250, "speaker": 1}
            ]"#
        )),
        "1\n00:00:00,000 --> 00:00:00,250\nab\n"
    );
}

#[test]
fn a_text_speaker_splits_a_cue_from_a_number() {
    assert_eq!(
        render_soniox_tokens_as_srt(&tokens(
            r#"[
                {"text": "a", "start_ms": 0, "end_ms": 100, "speaker": "1"},
                {"text": "b", "start_ms": 150, "end_ms": 250, "speaker": 1}
            ]"#
        )),
        "1\n00:00:00,000 --> 00:00:00,100\na\n\n2\n00:00:00,150 --> 00:00:00,250\nb\n"
    );
}

#[test]
fn plain_rendering_concatenates_tokens_without_tags() {
    assert_eq!(
        built_object(
            r#"{"transcript": {"tokens": [{"text": "a"}, {"text": "b"}]}}"#,
            None
        ),
        object(r#"{"text": "ab", "usage": null, "task": "transcribe"}"#)
    );
}

#[test]
fn plain_rendering_prefers_a_truthy_transcript_text() {
    let payload = r#"{"transcript": {"text": "T", "tokens": [{"text": "a"}]}}"#;
    assert_eq!(
        built_object(payload, None),
        object(r#"{"text": "T", "usage": null, "task": "transcribe"}"#)
    );
}

#[test]
fn an_empty_transcript_text_falls_back_to_the_tokens() {
    let payload = r#"{"transcript": {"text": "", "tokens": [{"text": "a"}, {"text": "b"}]}}"#;
    assert_eq!(
        built_object(payload, None),
        object(r#"{"text": "ab", "usage": null, "task": "transcribe"}"#)
    );
}

#[test]
fn an_empty_transcript_renders_empty_text() {
    let payload = r#"{"transcription": null, "transcript": null}"#;
    assert_eq!(
        built_object(payload, None),
        object(r#"{"text": "", "usage": null, "task": "transcribe"}"#)
    );
}

#[test]
fn a_transcript_key_selects_the_envelope_even_when_null() {
    let payload = r#"{"transcript": null, "text": "top", "tokens": [{"text": "a"}]}"#;
    assert_eq!(
        built_object(payload, None),
        object(r#"{"text": "", "usage": null, "task": "transcribe"}"#)
    );
}

#[test]
fn a_payload_without_a_transcript_key_is_the_transcript() {
    let built = response_from(r#"{"text": "top", "tokens": [{"text": "a"}]}"#, None);

    assert_eq!(built.text.as_deref(), Some("top"));
    let raw = built
        .hidden_params
        .get("soniox_raw")
        .and_then(|value| value.get("transcript"))
        .and_then(|value| value.as_object())
        .expect("the raw transcript");
    assert_eq!(raw["text"], "top");
}

#[test]
fn srt_and_vtt_replace_the_plain_rendering() {
    let payload = r#"{"transcript": {"text": "plain", "tokens": [
        {"text": "Hi", "start_ms": 0, "end_ms": 500}
    ]}}"#;
    assert_eq!(
        built_object(payload, Some("srt")),
        object(
            r#"{"text": "1\n00:00:00,000 --> 00:00:00,500\nHi\n", "usage": null, "task": "transcribe"}"#
        )
    );
    assert_eq!(
        built_object(payload, Some("vtt")),
        object(
            r#"{"text": "WEBVTT\n\n00:00:00.000 --> 00:00:00.500\nHi\n", "usage": null, "task": "transcribe"}"#
        )
    );
}

#[test]
fn empty_tokens_fall_back_to_the_plain_text() {
    let payload = r#"{"transcript": {"text": "plain", "tokens": []}}"#;
    assert_eq!(
        built_object(payload, Some("vtt")),
        object(r#"{"text": "plain", "usage": null, "task": "transcribe"}"#)
    );
    assert_eq!(
        built_object(payload, Some("srt")),
        object(r#"{"text": "plain", "usage": null, "task": "transcribe"}"#)
    );
}

#[test]
fn a_json_format_keeps_the_plain_rendering() {
    let payload = r#"{"transcript": {"tokens": [{"text": "a", "language": "en"}]}}"#;
    assert_eq!(
        built_object(payload, Some("json")),
        object(r#"{"text": "\n[en] a", "usage": null, "task": "transcribe", "language": "en"}"#)
    );
}

#[test]
fn verbose_json_accumulates_metadata_with_the_text() {
    let payload = r#"{
        "transcription": {"audio_duration_ms": 1500},
        "transcript": {"tokens": [{"text": "a", "language": "en"}]}
    }"#;
    assert_eq!(
        built_object(payload, Some(VERBOSE_JSON)),
        object(
            r#"{"text": "\n[en] a", "usage": null, "task": "transcribe", "duration": 1.5, "language": "en"}"#
        )
    );
}

#[rstest]
#[case::negative("-1500", Some("-1.5"))]
#[case::zero("0", Some("0.0"))]
#[case::true_value("true", Some("0.001"))]
#[case::false_value("false", Some("0.0"))]
#[case::text(r#""abc""#, None)]
#[case::list("[]", None)]
#[case::map("{}", None)]
#[case::null("null", None)]
fn duration_is_best_effort(#[case] value: &str, #[case] expected: Option<&str>) {
    let payload = format!(
        r#"{{"transcription": {{"audio_duration_ms": {value}}}, "transcript": {{"tokens": []}}}}"#
    );
    let built = response_from(&payload, None);

    let shown = built.extra.get("duration").map(|value| value.to_string());
    assert_eq!(shown, expected.map(|value| value.to_owned()));
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
        {"text": "Rest"}
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
                {"word": "Rest"}
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
fn verbose_words_need_a_nonempty_token_list() {
    let payload = r#"{"transcript": {"text": "T", "tokens": []}}"#;
    assert_eq!(
        built_object(payload, Some(VERBOSE_JSON)),
        object(r#"{"text": "T", "usage": null, "task": "transcribe"}"#)
    );
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

#[test]
fn hidden_params_keep_both_selected_records() {
    let built = response_from(
        r#"{
            "transcription": {"id": "t1"},
            "transcript": {"text": "muted", "tokens": [{"text": "a"}]}
        }"#,
        None,
    );

    let raw = built
        .hidden_params
        .get("soniox_raw")
        .and_then(|value| value.as_object());
    assert_eq!(
        raw,
        Some(&object(
            r#"{"transcription": {"id": "t1"}, "transcript": {"text": "muted", "tokens": [{"text": "a"}]}}"#
        ))
    );
}

#[test]
fn a_supplied_response_is_updated_and_kept() {
    let usage = object(r#"{"type": "duration", "seconds": 3}"#);
    let supplied = SonioxTranscriptionResponse {
        text: Some("old".to_owned()),
        usage: Some(usage.clone()),
        extra: object(r#"{"custom": 7, "task": "stale", "duration": 99.0}"#),
        hidden_params: object(r#"{"kept": 1}"#),
    };
    let payload = SonioxTranscriptPayload::try_from(object(
        r#"{"transcription": {"audio_duration_ms": 2000}, "transcript": {"tokens": [{"text": "hi", "language": "en"}]}}"#,
    ))
    .expect("a payload");
    let built =
        build_response_from_payload(&payload, Some(supplied), None).expect("an updated response");

    assert_eq!(built.text.as_deref(), Some("\n[en] hi"));
    assert_eq!(
        built.extra.get("custom").and_then(|value| value.as_i64()),
        Some(7)
    );
    assert_eq!(
        built.extra.get("task").and_then(|value| value.as_str()),
        Some(TRANSCRIBE_TASK)
    );
    assert_eq!(
        built.extra.get("duration").and_then(|value| value.as_f64()),
        Some(2.0)
    );
    assert_eq!(built.usage, Some(usage));
    assert!(built.hidden_params.contains_key("kept"));
    assert!(built.hidden_params.contains_key("soniox_raw"));
}

#[test]
fn a_supplied_response_keeps_its_duration_when_none_is_new() {
    let supplied = SonioxTranscriptionResponse {
        text: None,
        usage: None,
        extra: object(r#"{"duration": 99.0}"#),
        hidden_params: object("{}"),
    };
    let payload = SonioxTranscriptPayload::try_from(object(
        r#"{"transcription": {"audio_duration_ms": "abc"}, "transcript": {"tokens": []}}"#,
    ))
    .expect("a payload");
    let built =
        build_response_from_payload(&payload, Some(supplied), None).expect("an updated response");

    assert_eq!(
        built.extra.get("duration").and_then(|value| value.as_f64()),
        Some(99.0)
    );
}
