#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_openai::api::transcription_contract::AudioTranscriptionData;
use betterlite_openai::openai_base_config::{Headers, Object};
use rstest::rstest;
use serde_json::Number;

use super::error::RecordsError;
use super::grpc_status::{
    CodeAttribute, DetailsAttribute, GrpcErrorObservation, RivaException, grpc_error_to_exception,
};
use super::options::map_openai_params;
use super::recognition::transform_request;
use super::response::{FinalResult, build_transcription_response};
use super::{RivaAudioTranscriptionConfig, SUPPORTED_OPENAI_PARAMS};

fn params(text: &str) -> Object {
    serde_json::from_str(text).expect("a JSON object")
}

fn payload(text: &str) -> Object {
    let optional = map_openai_params(&params(text)).expect("mapped params");
    let request = transform_request(&optional).expect("a request");
    let data = request.request_data().expect("request data");
    let AudioTranscriptionData::Object(object) = data.data else {
        panic!("a Riva request payload is a JSON object");
    };
    object
}

fn payload_error(text: &str) -> RecordsError {
    let optional = map_openai_params(&params(text)).expect("mapped params");
    transform_request(&optional).expect_err("a rejected option")
}

fn response(
    final_results: &str,
    response_format: &str,
    duration_seconds: Option<f64>,
    granularities: Option<&[&str]>,
) -> Object {
    let finals: Vec<FinalResult> =
        serde_json::from_str(final_results).expect("typed final results");
    let names = granularities.map(|names| {
        names
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<String>>()
    });
    let built =
        build_transcription_response(&finals, response_format, duration_seconds, names.as_deref())
            .expect("a transcription response");
    let encoded = serde_json::to_value(built).expect("a serializable response");
    serde_json::from_value(encoded).expect("a JSON response")
}

fn endpointing_config(built: &Object) -> &Object {
    built["recognition_config"]["endpointing_config"]
        .as_object()
        .expect("an endpointing config")
}

fn default_payload() -> Object {
    params(
        r#"{
            "recognition_config": {
                "language_code": "en-US",
                "sample_rate_hertz": 16000,
                "encoding": "LINEAR_PCM",
                "audio_channel_count": 1,
                "enable_automatic_punctuation": true,
                "enable_word_time_offsets": false,
                "max_alternatives": 1,
                "model": "",
                "verbatim_transcripts": false,
                "profanity_filter": false
            },
            "response_format": "json",
            "timestamp_granularities": null
        }"#,
    )
}

#[test]
fn recognition_defaults_match_the_source() {
    assert_eq!(payload("{}"), default_payload());
}

#[test]
fn the_supported_openai_params_are_language_format_and_timestamps() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        ["language", "response_format", "timestamp_granularities"]
    );
}

#[rstest]
#[case::english("en", "en-US")]
#[case::spanish("es", "es-ES")]
#[case::german("de", "de-DE")]
#[case::french("fr", "fr-FR")]
#[case::italian("it", "it-IT")]
#[case::portuguese("pt", "pt-BR")]
#[case::japanese("ja", "ja-JP")]
#[case::korean("ko", "ko-KR")]
#[case::chinese("zh", "zh-CN")]
#[case::russian("ru", "ru-RU")]
#[case::hindi("hi", "hi-IN")]
#[case::arabic("ar", "ar-SA")]
#[case::uppercase("EN", "en-US")]
#[case::mixed_case("Pt", "pt-BR")]
fn maps_bare_language_codes_to_their_bcp47_defaults(
    #[case] language: &str,
    #[case] expected: &str,
) {
    let built = payload(&format!(r#"{{"language": "{language}"}}"#));
    assert_eq!(built["recognition_config"]["language_code"], expected);
}

#[rstest]
#[case::already_tagged("en-US", "en-US")]
#[case::different_region("fr-CA", "fr-CA")]
#[case::case_preserved("EN-gb", "EN-gb")]
#[case::script_subtag("zh-Hans", "zh-Hans")]
#[case::unknown_bare("xx", "xx")]
fn keeps_tagged_and_unknown_languages_verbatim(#[case] language: &str, #[case] expected: &str) {
    let built = payload(&format!(r#"{{"language": "{language}"}}"#));
    assert_eq!(built["recognition_config"]["language_code"], expected);
}

#[rstest]
#[case::empty("\"\"")]
#[case::number("5")]
#[case::boolean("true")]
#[case::list("[]")]
#[case::null("null")]
fn empty_and_non_string_languages_become_en_us(#[case] language: &str) {
    let built = payload(&format!(r#"{{"language": {language}}}"#));
    assert_eq!(built["recognition_config"]["language_code"], "en-US");
}

#[test]
fn map_keeps_every_non_null_input_and_skips_only_nulls() {
    let optional = map_openai_params(&params(
        r#"{"language": null, "prompt": "words", "temperature": 0, "custom": false, "empty": ""}"#,
    ))
    .expect("mapped params");
    assert_eq!(
        optional,
        params(r#"{"prompt": "words", "temperature": 0, "custom": false, "empty": ""}"#)
    );
    assert!(!optional.contains_key("language"));
    assert!(!optional.contains_key("language_code"));
}

#[rstest]
#[case::word(&["word"], true)]
#[case::segment_and_word(&["segment", "word"], true)]
#[case::segment(&["segment"], false)]
#[case::empty(&[], false)]
#[case::capitalised(&["Word"], false)]
fn word_granularities_enable_word_time_offsets(
    #[case] granularities: &[&str],
    #[case] expected: bool,
) {
    let listed = serde_json::to_string(granularities).expect("a granularity list");
    let built = payload(&format!(r#"{{"timestamp_granularities": {listed}}}"#));
    assert_eq!(
        built["recognition_config"]["enable_word_time_offsets"],
        expected
    );
    let echoed = built["timestamp_granularities"]
        .as_array()
        .expect("a granularity list");
    assert_eq!(echoed.len(), granularities.len());
    for (name, wanted) in echoed.iter().zip(granularities) {
        assert_eq!(name.as_str(), Some(*wanted));
    }
}

#[rstest]
#[case::existing_true(
    r#"{"enable_word_time_offsets": true, "timestamp_granularities": ["segment"]}"#,
    true
)]
#[case::word_overrides_false(
    r#"{"enable_word_time_offsets": false, "timestamp_granularities": ["word"]}"#,
    true
)]
#[case::flag_alone(r#"{"enable_word_time_offsets": true}"#, true)]
#[case::flag_false_alone(r#"{"enable_word_time_offsets": false}"#, false)]
fn other_timestamp_requests_do_not_clear_an_existing_flag(
    #[case] value: &str,
    #[case] expected: bool,
) {
    let built = payload(value);
    assert_eq!(
        built["recognition_config"]["enable_word_time_offsets"],
        expected
    );
}

#[test]
fn a_null_timestamp_granularities_keeps_the_payload_null() {
    let built = payload(r#"{"timestamp_granularities": null}"#);
    assert!(built["timestamp_granularities"].is_null());
    assert_eq!(
        built["recognition_config"]["enable_word_time_offsets"],
        false
    );
}

#[rstest]
#[case::complete(
    r#"{"chunking_strategy": {"type": "server_vad", "threshold": 0.5, "silence_duration_ms": 200, "prefix_padding_ms": 300}}"#,
    r#"{"start_threshold": 0.5, "stop_threshold": 0.5, "stop_history": 200, "stop_history_eou": 300}"#
)]
#[case::threshold_only(
    r#"{"chunking_strategy": {"type": "server_vad", "threshold": 0.5}}"#,
    r#"{"start_threshold": 0.5, "stop_threshold": 0.5}"#
)]
#[case::integer_threshold(
    r#"{"chunking_strategy": {"type": "server_vad", "threshold": 1}}"#,
    r#"{"start_threshold": 1.0, "stop_threshold": 1.0}"#
)]
#[case::silence_only(
    r#"{"chunking_strategy": {"type": "server_vad", "silence_duration_ms": 100}}"#,
    r#"{"stop_history": 100}"#
)]
#[case::padding_only(
    r#"{"chunking_strategy": {"type": "server_vad", "prefix_padding_ms": 50}}"#,
    r#"{"stop_history_eou": 50}"#
)]
fn server_vad_chunking_becomes_endpointing(#[case] value: &str, #[case] expected: &str) {
    let built = payload(value);
    assert_eq!(endpointing_config(&built), &params(expected));
}

#[rstest]
#[case::absent("{}")]
#[case::auto(r#"{"chunking_strategy": "auto"}"#)]
#[case::null(r#"{"chunking_strategy": null}"#)]
#[case::empty_server_vad(r#"{"chunking_strategy": {"type": "server_vad"}}"#)]
#[case::semantic(r#"{"chunking_strategy": {"type": "semantic_vad"}}"#)]
#[case::text(r#"{"chunking_strategy": "fixed"}"#)]
#[case::number(r#"{"chunking_strategy": 5}"#)]
fn chunking_that_derives_nothing_omits_endpointing(#[case] value: &str) {
    let built = payload(value);
    assert!(
        !built["recognition_config"]
            .as_object()
            .expect("a recognition config")
            .contains_key("endpointing_config")
    );
}

#[test]
fn an_explicit_endpointing_mapping_wins_even_when_empty() {
    let built = payload(
        r#"{"endpointing_config": {}, "chunking_strategy": {"type": "server_vad", "threshold": 0.5}}"#,
    );
    assert_eq!(endpointing_config(&built), &params("{}"));
}

#[test]
fn an_explicit_endpointing_mapping_is_copied() {
    let built =
        payload(r#"{"endpointing_config": {"start_threshold": 0.25, "stop_history": 100}}"#);
    assert_eq!(
        endpointing_config(&built),
        &params(r#"{"start_threshold": 0.25, "stop_history": 100}"#)
    );
}

#[test]
fn an_explicit_endpointing_mapping_keeps_unknown_settings() {
    let built =
        payload(r#"{"endpointing_config": {"start_threshold": 0.1, "other_setting": "x"}}"#);
    assert_eq!(
        endpointing_config(&built),
        &params(r#"{"start_threshold": 0.1, "other_setting": "x"}"#)
    );
}

#[rstest]
#[case::null(
    r#"{"endpointing_config": null, "chunking_strategy": {"type": "server_vad", "threshold": 0.5}}"#
)]
#[case::text(
    r#"{"endpointing_config": "x", "chunking_strategy": {"type": "server_vad", "threshold": 0.5}}"#
)]
fn a_non_mapping_endpointing_falls_back_to_chunking(#[case] value: &str) {
    let built = payload(value);
    assert_eq!(
        endpointing_config(&built),
        &params(r#"{"start_threshold": 0.5, "stop_threshold": 0.5}"#)
    );
}

#[rstest]
#[case::text(r#"{"chunking_strategy": {"type": "server_vad", "threshold": "0.5"}}"#)]
#[case::null(r#"{"chunking_strategy": {"type": "server_vad", "threshold": null}}"#)]
#[case::boolean(r#"{"chunking_strategy": {"type": "server_vad", "threshold": true}}"#)]
fn a_chunking_threshold_must_be_a_number(#[case] value: &str) {
    let error = payload_error(value);
    assert!(matches!(
        error,
        RecordsError::Number {
            field: "chunking_strategy.threshold"
        }
    ));
}

#[rstest]
#[case::negative(
    r#"{"chunking_strategy": {"type": "server_vad", "silence_duration_ms": -5}}"#,
    "chunking_strategy.silence_duration_ms"
)]
#[case::text(
    r#"{"chunking_strategy": {"type": "server_vad", "silence_duration_ms": "200"}}"#,
    "chunking_strategy.silence_duration_ms"
)]
#[case::fraction(
    r#"{"chunking_strategy": {"type": "server_vad", "silence_duration_ms": 1.5}}"#,
    "chunking_strategy.silence_duration_ms"
)]
#[case::negative_padding(
    r#"{"chunking_strategy": {"type": "server_vad", "prefix_padding_ms": -1}}"#,
    "chunking_strategy.prefix_padding_ms"
)]
fn a_chunking_history_must_be_a_non_negative_integer(
    #[case] value: &str,
    #[case] field: &'static str,
) {
    let error = payload_error(value);
    assert!(matches!(error, RecordsError::Integer { field: rejected } if rejected == field));
}

#[rstest]
#[case::named(r#"{"riva_model_name": "parakeet"}"#, "parakeet")]
#[case::absent("{}", "")]
#[case::api_model_only(r#"{"model": "openai/whisper-1"}"#, "")]
#[case::empty_name(r#"{"riva_model_name": ""}"#, "")]
#[case::named_beats_api_model(r#"{"riva_model_name": "x", "model": "ignored"}"#, "x")]
fn riva_model_name_is_the_only_recognition_model_source(
    #[case] value: &str,
    #[case] expected: &str,
) {
    let built = payload(value);
    assert_eq!(built["recognition_config"]["model"], expected);
}

#[test]
fn recognition_inputs_are_used_as_given() {
    let built = payload(
        r#"{
            "sample_rate_hertz": 8000,
            "encoding": "MULAW",
            "audio_channel_count": 2,
            "max_alternatives": 3,
            "enable_automatic_punctuation": false,
            "verbatim_transcripts": true,
            "profanity_filter": true
        }"#,
    );
    let config = &built["recognition_config"];
    assert_eq!(config["sample_rate_hertz"], 8000_i32);
    assert_eq!(config["encoding"], "MULAW");
    assert_eq!(config["audio_channel_count"], 2_i32);
    assert_eq!(config["max_alternatives"], 3_i32);
    assert_eq!(config["enable_automatic_punctuation"], false);
    assert_eq!(config["verbatim_transcripts"], true);
    assert_eq!(config["profanity_filter"], true);
}

#[rstest]
#[case::absent("{}", "json")]
#[case::empty(r#"{"response_format": ""}"#, "json")]
#[case::verbose(r#"{"response_format": "verbose_json"}"#, "verbose_json")]
#[case::text(r#"{"response_format": "text"}"#, "text")]
#[case::subtitle(r#"{"response_format": "srt"}"#, "srt")]
fn a_response_format_falls_back_to_json_when_absent_or_empty(
    #[case] value: &str,
    #[case] expected: &str,
) {
    let built = payload(value);
    assert_eq!(built["response_format"], expected);
}

#[rstest]
#[case::response_format(r#"{"response_format": 5}"#, "response_format")]
#[case::language_code(r#"{"language_code": 5}"#, "language_code")]
#[case::riva_model_name(r#"{"riva_model_name": []}"#, "riva_model_name")]
#[case::encoding(r#"{"encoding": {}}"#, "encoding")]
#[case::sample_rate(r#"{"sample_rate_hertz": "16000"}"#, "sample_rate_hertz")]
#[case::channels(r#"{"audio_channel_count": -1}"#, "audio_channel_count")]
#[case::alternatives(r#"{"max_alternatives": 1.5}"#, "max_alternatives")]
#[case::punctuation(
    r#"{"enable_automatic_punctuation": "yes"}"#,
    "enable_automatic_punctuation"
)]
#[case::word_offsets(r#"{"enable_word_time_offsets": 1}"#, "enable_word_time_offsets")]
#[case::verbatim(r#"{"verbatim_transcripts": 1}"#, "verbatim_transcripts")]
#[case::profanity(r#"{"profanity_filter": 0}"#, "profanity_filter")]
fn a_recognition_option_of_the_wrong_type_names_the_field(
    #[case] value: &str,
    #[case] field: &'static str,
) {
    let error = payload_error(value);
    let named = match &error {
        RecordsError::Text { field: named }
        | RecordsError::Integer { field: named }
        | RecordsError::Boolean { field: named }
        | RecordsError::Number { field: named }
        | RecordsError::Granularities { field: named }
        | RecordsError::Timing { field: named }
        | RecordsError::Value { field: named, .. } => *named,
    };
    assert_eq!(named, field);
}

#[rstest]
#[case::text(r#"{"timestamp_granularities": "word"}"#, "timestamp_granularities")]
#[case::number(r#"{"timestamp_granularities": [5]}"#, "timestamp_granularities")]
#[case::object(
    r#"{"timestamp_granularities": {"word": true}}"#,
    "timestamp_granularities"
)]
fn a_timestamp_granularity_list_is_required(#[case] value: &str, #[case] field: &'static str) {
    let error = map_openai_params(&params(value)).expect_err("a rejected option");
    assert!(matches!(error, RecordsError::Granularities { field: rejected } if rejected == field));
}

#[test]
fn validate_environment_returns_the_headers_unchanged() {
    let headers = Headers::from([("authorization".to_owned(), "Bearer key".to_owned())]);
    assert_eq!(
        RivaAudioTranscriptionConfig::validate_environment(headers.clone()),
        headers
    );
}

#[rstest]
#[case::separated(
    r#"[{"transcript": " Hello "}, {"transcript": "world"}]"#,
    "Hello world"
)]
#[case::adjacent(r#"[{"transcript": "a"}, {"transcript": "b"}]"#, "ab")]
#[case::blank_and_spaced(r#"[{"transcript": ""}, {"transcript": "  x  "}]"#, "x")]
#[case::blank_only(r#"[{"transcript": "   "}]"#, "")]
#[case::none("[]", "")]
fn final_transcripts_join_without_separators_and_strip(#[case] finals: &str, #[case] text: &str) {
    let built = response(finals, "json", None, None);
    assert_eq!(built["text"], text);
}

#[rstest]
#[case::json("json")]
#[case::text("text")]
#[case::srt("srt")]
#[case::vtt("vtt")]
#[case::verbose("verbose_json")]
fn every_response_carries_the_transcribe_task(#[case] response_format: &str) {
    let built = response(
        r#"[{"transcript": "hi"}]"#,
        response_format,
        Some(1.0),
        None,
    );
    assert_eq!(built["task"], "transcribe");
}

#[test]
fn verbose_json_lists_requested_word_timings() {
    let built = response(
        r#"[
            {"transcript": " a", "words": [{"word": "a", "start_time_ms": 1234, "end_time_ms": 2500}]},
            {"transcript": " b c", "words": [
                {"word": "b", "start_time_ms": 2500, "end_time_ms": 3000},
                {"word": "", "start_time_ms": -500, "end_time_ms": 0}
            ]}
        ]"#,
        "verbose_json",
        Some(4.0),
        Some(&["word"][..]),
    );
    assert_eq!(
        built,
        params(
            r#"{
                "text": "a b c",
                "task": "transcribe",
                "words": [
                    {"word": "a", "start": 1.234, "end": 2.5},
                    {"word": "b", "start": 2.5, "end": 3.0},
                    {"word": "", "start": -0.5, "end": 0.0}
                ],
                "duration": 4.0
            }"#
        )
    );
}

#[rstest]
#[case::segment(Some(&["segment"][..]))]
#[case::absent(None)]
fn verbose_json_without_a_word_request_has_no_words(#[case] granularities: Option<&[&str]>) {
    let built = response(
        r#"[{"transcript": "hi", "words": [{"word": "hi", "start_time_ms": 0, "end_time_ms": 100}]}]"#,
        "verbose_json",
        Some(1.5),
        granularities,
    );
    assert_eq!(
        built,
        params(r#"{"text": "hi", "task": "transcribe", "duration": 1.5}"#)
    );
}

#[test]
fn verbose_json_with_an_empty_word_list_has_no_words() {
    let built = response(
        r#"[{"transcript": "hi", "words": []}]"#,
        "verbose_json",
        Some(1.5),
        Some(&["word"][..]),
    );
    assert_eq!(
        built,
        params(r#"{"text": "hi", "task": "transcribe", "duration": 1.5}"#)
    );
}

#[rstest]
#[case::present(Some(0.0), r#"{"text": "hi", "task": "transcribe", "duration": 0.0}"#)]
#[case::absent(None, r#"{"text": "hi", "task": "transcribe"}"#)]
fn verbose_json_includes_only_a_non_null_duration(
    #[case] duration: Option<f64>,
    #[case] expected: &str,
) {
    let built = response(r#"[{"transcript": "hi"}]"#, "verbose_json", duration, None);
    assert_eq!(built, params(expected));
}

#[test]
fn plain_formats_drop_words_and_duration() {
    let built = response(
        r#"[{"transcript": "hi", "words": [{"word": "hi", "start_time_ms": 0, "end_time_ms": 100}]}]"#,
        "json",
        Some(1.5),
        Some(&["word"][..]),
    );
    assert_eq!(built, params(r#"{"text": "hi", "task": "transcribe"}"#));
}

#[test]
fn empty_final_results_yield_an_empty_text() {
    let built = response("[]", "json", None, None);
    assert_eq!(built, params(r#"{"text": "", "task": "transcribe"}"#));
}

fn observation(code: CodeAttribute, details: DetailsAttribute, text: &str) -> GrpcErrorObservation {
    GrpcErrorObservation {
        code,
        details,
        text: text.to_owned(),
    }
}

fn returned_code(name: Json) -> CodeAttribute {
    CodeAttribute::Returns { name }
}

#[rstest]
#[case::ok("OK", 200)]
#[case::cancelled("CANCELLED", 499)]
#[case::unknown("UNKNOWN", 500)]
#[case::invalid_argument("INVALID_ARGUMENT", 400)]
#[case::deadline("DEADLINE_EXCEEDED", 504)]
#[case::not_found("NOT_FOUND", 404)]
#[case::already_exists("ALREADY_EXISTS", 409)]
#[case::permission("PERMISSION_DENIED", 403)]
#[case::resource("RESOURCE_EXHAUSTED", 429)]
#[case::precondition("FAILED_PRECONDITION", 400)]
#[case::aborted("ABORTED", 409)]
#[case::out_of_range("OUT_OF_RANGE", 400)]
#[case::unimplemented("UNIMPLEMENTED", 501)]
#[case::internal("INTERNAL", 500)]
#[case::unavailable("UNAVAILABLE", 503)]
#[case::data_loss("DATA_LOSS", 500)]
#[case::unauthenticated("UNAUTHENTICATED", 401)]
fn every_grpc_status_name_maps_to_its_http_status(#[case] name: &str, #[case] status: u16) {
    let exception = grpc_error_to_exception(&observation(
        returned_code(Json::Text(name.to_owned())),
        DetailsAttribute::Missing,
        "text",
    ));
    assert_eq!(exception.status_code, status);
    assert_eq!(
        exception.message,
        format!("NVIDIA Riva gRPC error ({name}): text")
    );
}

#[rstest]
#[case::missing(CodeAttribute::Missing)]
#[case::not_callable(CodeAttribute::NotCallable)]
#[case::raises(CodeAttribute::Raises)]
#[case::numeric_name(returned_code(Json::Number(Number::from(5_i32))))]
#[case::null_name(returned_code(Json::Null))]
#[case::empty_name(returned_code(Json::Text(String::new())))]
fn an_unusable_status_name_leaves_the_status_at_500(#[case] code: CodeAttribute) {
    let exception = grpc_error_to_exception(&observation(code, DetailsAttribute::Missing, "boom"));
    assert_eq!(exception.status_code, 500);
    assert_eq!(exception.message, "NVIDIA Riva error: boom");
}

#[test]
fn an_unknown_status_name_keeps_its_500_and_its_prefix() {
    let exception = grpc_error_to_exception(&observation(
        returned_code(Json::Text("WEIRD".to_owned())),
        DetailsAttribute::Missing,
        "boom",
    ));
    assert_eq!(
        exception,
        RivaException {
            status_code: 500,
            message: "NVIDIA Riva gRPC error (WEIRD): boom".to_owned()
        }
    );
}

#[rstest]
#[case::nonempty(DetailsAttribute::Returns { value: Json::Text("detail".to_owned()) }, "detail")]
#[case::empty(DetailsAttribute::Returns { value: Json::Text(String::new()) }, "boom")]
#[case::number(DetailsAttribute::Returns { value: Json::Number(Number::from(5_i32)) }, "boom")]
#[case::null(DetailsAttribute::Returns { value: Json::Null }, "boom")]
#[case::missing(DetailsAttribute::Missing, "boom")]
#[case::not_callable(DetailsAttribute::NotCallable, "boom")]
#[case::raises(DetailsAttribute::Raises, "boom")]
fn only_a_nonempty_details_string_replaces_the_exception_text(
    #[case] details: DetailsAttribute,
    #[case] expected: &str,
) {
    let exception = grpc_error_to_exception(&observation(
        returned_code(Json::Text("UNAVAILABLE".to_owned())),
        details,
        "boom",
    ));
    assert_eq!(
        exception.message,
        format!("NVIDIA Riva gRPC error (UNAVAILABLE): {expected}")
    );
    assert_eq!(exception.status_code, 503);
}

#[test]
fn an_exception_text_naming_auth_maps_to_500_without_a_prefix() {
    let exception = grpc_error_to_exception(&observation(
        CodeAttribute::Missing,
        DetailsAttribute::Missing,
        "UNAUTHENTICATED: token rejected",
    ));
    assert_eq!(exception.status_code, 500);
    assert_eq!(
        exception.message,
        "NVIDIA Riva error: UNAUTHENTICATED: token rejected"
    );
}

#[test]
fn the_exception_displays_its_message() {
    let exception = grpc_error_to_exception(&observation(
        returned_code(Json::Text("CANCELLED".to_owned())),
        DetailsAttribute::Missing,
        "stopped",
    ));
    assert_eq!(
        exception.to_string(),
        "NVIDIA Riva gRPC error (CANCELLED): stopped"
    );
}
