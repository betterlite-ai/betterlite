#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::{Headers, Object};
use rstest::rstest;
use serde_json::{Number, Value, json};

use super::error::RecordsError;
use super::grpc_status::{
    CodeAttribute, DetailsAttribute, GrpcErrorObservation, RivaException, grpc_error_to_exception,
};
use super::options::map_openai_params;
use super::recognition::transform_request;
use super::response::{FinalResult, build_transcription_response};
use super::{RivaAudioTranscriptionConfig, SUPPORTED_OPENAI_PARAMS};

fn params(value: Value) -> Object {
    let Value::Object(object) = value else {
        panic!("a test passes a JSON object");
    };
    object
}

fn payload(value: Value) -> Value {
    let optional = map_openai_params(&params(value)).expect("mapped params");
    let request = transform_request(&optional).expect("a request");
    let data = request.request_data().expect("request data");
    serde_json::to_value(data.data).expect("a JSON payload")
}

fn payload_error(value: Value) -> RecordsError {
    let optional = map_openai_params(&params(value)).expect("mapped params");
    transform_request(&optional).expect_err("a rejected option")
}

fn response(
    final_results: Value,
    response_format: &str,
    duration_seconds: Option<f64>,
    granularities: Option<&[&str]>,
) -> Value {
    let finals: Vec<FinalResult> =
        serde_json::from_value(final_results).expect("typed final results");
    let names = granularities.map(|names| {
        names
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<String>>()
    });
    let built =
        build_transcription_response(&finals, response_format, duration_seconds, names.as_deref())
            .expect("a transcription response");
    serde_json::to_value(built).expect("a JSON response")
}

fn default_payload() -> Value {
    json!({
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
    })
}

#[test]
fn recognition_defaults_match_the_source() {
    assert_eq!(payload(json!({})), default_payload());
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
    let built = payload(json!({"language": language}));
    assert_eq!(built["recognition_config"]["language_code"], expected);
}

#[rstest]
#[case::already_tagged("en-US", "en-US")]
#[case::different_region("fr-CA", "fr-CA")]
#[case::case_preserved("EN-gb", "EN-gb")]
#[case::script_subtag("zh-Hans", "zh-Hans")]
#[case::unknown_bare("xx", "xx")]
fn keeps_tagged_and_unknown_languages_verbatim(#[case] language: &str, #[case] expected: &str) {
    let built = payload(json!({"language": language}));
    assert_eq!(built["recognition_config"]["language_code"], expected);
}

#[rstest]
#[case::empty(json!(""))]
#[case::number(json!(5))]
#[case::boolean(json!(true))]
#[case::list(json!([]))]
#[case::null(Value::Null)]
fn empty_and_non_string_languages_become_en_us(#[case] language: Value) {
    let built = payload(json!({"language": language}));
    assert_eq!(built["recognition_config"]["language_code"], "en-US");
}

#[test]
fn map_keeps_every_non_null_input_and_skips_only_nulls() {
    let optional = map_openai_params(&params(json!({
        "language": null,
        "prompt": "words",
        "temperature": 0,
        "custom": false,
        "empty": ""
    })))
    .expect("mapped params");
    assert_eq!(
        optional,
        params(json!({"prompt": "words", "temperature": 0, "custom": false, "empty": ""}))
    );
    assert!(!optional.contains_key("language"));
    assert!(!optional.contains_key("language_code"));
}

#[rstest]
#[case::word(json!(["word"]), true)]
#[case::segment_and_word(json!(["segment", "word"]), true)]
#[case::segment(json!(["segment"]), false)]
#[case::empty(json!([]), false)]
#[case::capitalised(json!(["Word"]), false)]
fn word_granularities_enable_word_time_offsets(
    #[case] granularities: Value,
    #[case] expected: bool,
) {
    let built = payload(json!({"timestamp_granularities": granularities.clone()}));
    assert_eq!(
        built["recognition_config"]["enable_word_time_offsets"],
        expected
    );
    assert_eq!(built["timestamp_granularities"], granularities);
}

#[rstest]
#[case::existing_true(json!({"enable_word_time_offsets": true, "timestamp_granularities": ["segment"]}), true)]
#[case::word_overrides_false(json!({"enable_word_time_offsets": false, "timestamp_granularities": ["word"]}), true)]
#[case::flag_alone(json!({"enable_word_time_offsets": true}), true)]
#[case::flag_false_alone(json!({"enable_word_time_offsets": false}), false)]
fn other_timestamp_requests_do_not_clear_an_existing_flag(
    #[case] value: Value,
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
    let built = payload(json!({"timestamp_granularities": null}));
    assert_eq!(built["timestamp_granularities"], Value::Null);
    assert_eq!(
        built["recognition_config"]["enable_word_time_offsets"],
        false
    );
}

#[rstest]
#[case::complete(
    json!({"chunking_strategy": {"type": "server_vad", "threshold": 0.5, "silence_duration_ms": 200, "prefix_padding_ms": 300}}),
    json!({"start_threshold": 0.5, "stop_threshold": 0.5, "stop_history": 200, "stop_history_eou": 300})
)]
#[case::threshold_only(
    json!({"chunking_strategy": {"type": "server_vad", "threshold": 0.5}}),
    json!({"start_threshold": 0.5, "stop_threshold": 0.5})
)]
#[case::integer_threshold(
    json!({"chunking_strategy": {"type": "server_vad", "threshold": 1}}),
    json!({"start_threshold": 1.0, "stop_threshold": 1.0})
)]
#[case::silence_only(
    json!({"chunking_strategy": {"type": "server_vad", "silence_duration_ms": 100}}),
    json!({"stop_history": 100})
)]
#[case::padding_only(
    json!({"chunking_strategy": {"type": "server_vad", "prefix_padding_ms": 50}}),
    json!({"stop_history_eou": 50})
)]
fn server_vad_chunking_becomes_endpointing(#[case] value: Value, #[case] expected: Value) {
    let built = payload(value);
    assert_eq!(built["recognition_config"]["endpointing_config"], expected);
}

#[rstest]
#[case::absent(json!({}))]
#[case::auto(json!({"chunking_strategy": "auto"}))]
#[case::null(json!({"chunking_strategy": null}))]
#[case::empty_server_vad(json!({"chunking_strategy": {"type": "server_vad"}}))]
#[case::semantic(json!({"chunking_strategy": {"type": "semantic_vad"}}))]
#[case::text(json!({"chunking_strategy": "fixed"}))]
#[case::number(json!({"chunking_strategy": 5}))]
fn chunking_that_derives_nothing_omits_endpointing(#[case] value: Value) {
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
    let built = payload(json!({
        "endpointing_config": {},
        "chunking_strategy": {"type": "server_vad", "threshold": 0.5}
    }));
    assert_eq!(built["recognition_config"]["endpointing_config"], json!({}));
}

#[test]
fn an_explicit_endpointing_mapping_is_copied() {
    let built = payload(json!({
        "endpointing_config": {"start_threshold": 0.25, "stop_history": 100}
    }));
    assert_eq!(
        built["recognition_config"]["endpointing_config"],
        json!({"start_threshold": 0.25, "stop_history": 100})
    );
}

#[test]
fn an_explicit_endpointing_mapping_keeps_unknown_settings() {
    let built = payload(json!({
        "endpointing_config": {"start_threshold": 0.1, "other_setting": "x"}
    }));
    assert_eq!(
        built["recognition_config"]["endpointing_config"],
        json!({"start_threshold": 0.1, "other_setting": "x"})
    );
}

#[rstest]
#[case::null(json!({"endpointing_config": null, "chunking_strategy": {"type": "server_vad", "threshold": 0.5}}))]
#[case::text(json!({"endpointing_config": "x", "chunking_strategy": {"type": "server_vad", "threshold": 0.5}}))]
fn a_non_mapping_endpointing_falls_back_to_chunking(#[case] value: Value) {
    let built = payload(value);
    assert_eq!(
        built["recognition_config"]["endpointing_config"],
        json!({"start_threshold": 0.5, "stop_threshold": 0.5})
    );
}

#[rstest]
#[case::text(json!({"chunking_strategy": {"type": "server_vad", "threshold": "0.5"}}))]
#[case::null(json!({"chunking_strategy": {"type": "server_vad", "threshold": null}}))]
#[case::boolean(json!({"chunking_strategy": {"type": "server_vad", "threshold": true}}))]
fn a_chunking_threshold_must_be_a_number(#[case] value: Value) {
    let error = payload_error(value);
    assert!(matches!(
        error,
        RecordsError::Number {
            field: "chunking_strategy.threshold"
        }
    ));
}

#[rstest]
#[case::negative(json!({"chunking_strategy": {"type": "server_vad", "silence_duration_ms": -5}}), "chunking_strategy.silence_duration_ms")]
#[case::text(json!({"chunking_strategy": {"type": "server_vad", "silence_duration_ms": "200"}}), "chunking_strategy.silence_duration_ms")]
#[case::fraction(json!({"chunking_strategy": {"type": "server_vad", "silence_duration_ms": 1.5}}), "chunking_strategy.silence_duration_ms")]
#[case::negative_padding(json!({"chunking_strategy": {"type": "server_vad", "prefix_padding_ms": -1}}), "chunking_strategy.prefix_padding_ms")]
fn a_chunking_history_must_be_a_non_negative_integer(
    #[case] value: Value,
    #[case] field: &'static str,
) {
    let error = payload_error(value);
    assert!(matches!(error, RecordsError::Integer { field: rejected } if rejected == field));
}

#[rstest]
#[case::named(json!({"riva_model_name": "parakeet"}), "parakeet")]
#[case::absent(json!({}), "")]
#[case::api_model_only(json!({"model": "openai/whisper-1"}), "")]
#[case::empty_name(json!({"riva_model_name": ""}), "")]
#[case::named_beats_api_model(json!({"riva_model_name": "x", "model": "ignored"}), "x")]
fn riva_model_name_is_the_only_recognition_model_source(
    #[case] value: Value,
    #[case] expected: &str,
) {
    let built = payload(value);
    assert_eq!(built["recognition_config"]["model"], expected);
}

#[test]
fn recognition_inputs_are_used_as_given() {
    let built = payload(json!({
        "sample_rate_hertz": 8000,
        "encoding": "MULAW",
        "audio_channel_count": 2,
        "max_alternatives": 3,
        "enable_automatic_punctuation": false,
        "verbatim_transcripts": true,
        "profanity_filter": true
    }));
    let config = &built["recognition_config"];
    assert_eq!(config["sample_rate_hertz"], 8000);
    assert_eq!(config["encoding"], "MULAW");
    assert_eq!(config["audio_channel_count"], 2);
    assert_eq!(config["max_alternatives"], 3);
    assert_eq!(config["enable_automatic_punctuation"], false);
    assert_eq!(config["verbatim_transcripts"], true);
    assert_eq!(config["profanity_filter"], true);
}

#[rstest]
#[case::absent(json!({}), "json")]
#[case::empty(json!({"response_format": ""}), "json")]
#[case::verbose(json!({"response_format": "verbose_json"}), "verbose_json")]
#[case::text(json!({"response_format": "text"}), "text")]
#[case::subtitle(json!({"response_format": "srt"}), "srt")]
fn a_response_format_falls_back_to_json_when_absent_or_empty(
    #[case] value: Value,
    #[case] expected: &str,
) {
    let built = payload(value);
    assert_eq!(built["response_format"], expected);
}

#[rstest]
#[case::response_format(json!({"response_format": 5}), "response_format")]
#[case::language_code(json!({"language_code": 5}), "language_code")]
#[case::riva_model_name(json!({"riva_model_name": []}), "riva_model_name")]
#[case::encoding(json!({"encoding": {}}), "encoding")]
#[case::sample_rate(json!({"sample_rate_hertz": "16000"}), "sample_rate_hertz")]
#[case::channels(json!({"audio_channel_count": -1}), "audio_channel_count")]
#[case::alternatives(json!({"max_alternatives": 1.5}), "max_alternatives")]
#[case::punctuation(json!({"enable_automatic_punctuation": "yes"}), "enable_automatic_punctuation")]
#[case::word_offsets(json!({"enable_word_time_offsets": 1}), "enable_word_time_offsets")]
#[case::verbatim(json!({"verbatim_transcripts": 1}), "verbatim_transcripts")]
#[case::profanity(json!({"profanity_filter": 0}), "profanity_filter")]
fn a_recognition_option_of_the_wrong_type_names_the_field(
    #[case] value: Value,
    #[case] field: &'static str,
) {
    let error = payload_error(value);
    let named = match &error {
        RecordsError::Text { field }
        | RecordsError::Integer { field }
        | RecordsError::Boolean { field }
        | RecordsError::Number { field }
        | RecordsError::Granularities { field }
        | RecordsError::Timing { field }
        | RecordsError::Value { field, .. } => *field,
    };
    assert_eq!(named, field);
}

#[rstest]
#[case::text(json!({"timestamp_granularities": "word"}), "timestamp_granularities")]
#[case::number(json!({"timestamp_granularities": [5]}), "timestamp_granularities")]
#[case::object(json!({"timestamp_granularities": {"word": true}}), "timestamp_granularities")]
fn a_timestamp_granularity_list_is_required(#[case] value: Value, #[case] field: &'static str) {
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
#[case::separated(json!([{"transcript": " Hello "}, {"transcript": "world"}]), "Hello world")]
#[case::adjacent(json!([{"transcript": "a"}, {"transcript": "b"}]), "ab")]
#[case::blank_and_spaced(json!([{"transcript": ""}, {"transcript": "  x  "}]), "x")]
#[case::blank_only(json!([{"transcript": "   "}]), "")]
#[case::none(json!([]), "")]
fn final_transcripts_join_without_separators_and_strip(#[case] finals: Value, #[case] text: &str) {
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
        json!([{"transcript": "hi"}]),
        response_format,
        Some(1.0),
        None,
    );
    assert_eq!(built["task"], "transcribe");
}

#[test]
fn verbose_json_lists_requested_word_timings() {
    let built = response(
        json!([
            {"transcript": " a", "words": [{"word": "a", "start_time_ms": 1234, "end_time_ms": 2500}]},
            {"transcript": " b c", "words": [
                {"word": "b", "start_time_ms": 2500, "end_time_ms": 3000},
                {"word": "", "start_time_ms": -500, "end_time_ms": 0}
            ]}
        ]),
        "verbose_json",
        Some(4.0),
        Some(&["word"][..]),
    );
    assert_eq!(
        built,
        json!({
            "text": "a b c",
            "task": "transcribe",
            "words": [
                {"word": "a", "start": 1.234, "end": 2.5},
                {"word": "b", "start": 2.5, "end": 3.0},
                {"word": "", "start": -0.5, "end": 0.0}
            ],
            "duration": 4.0
        })
    );
}

#[rstest]
#[case::segment(Some(&["segment"][..]))]
#[case::absent(None)]
fn verbose_json_without_a_word_request_has_no_words(#[case] granularities: Option<&[&str]>) {
    let built = response(
        json!([{"transcript": "hi", "words": [{"word": "hi", "start_time_ms": 0, "end_time_ms": 100}]}]),
        "verbose_json",
        Some(1.5),
        granularities,
    );
    assert_eq!(
        built,
        json!({"text": "hi", "task": "transcribe", "duration": 1.5})
    );
}

#[test]
fn verbose_json_with_an_empty_word_list_has_no_words() {
    let built = response(
        json!([{"transcript": "hi", "words": []}]),
        "verbose_json",
        Some(1.5),
        Some(&["word"][..]),
    );
    assert_eq!(
        built,
        json!({"text": "hi", "task": "transcribe", "duration": 1.5})
    );
}

#[rstest]
#[case::present(Some(0.0), json!(0.0))]
#[case::absent(None, Value::Null)]
fn verbose_json_includes_only_a_non_null_duration(
    #[case] duration: Option<f64>,
    #[case] expected: Value,
) {
    let built = response(
        json!([{"transcript": "hi"}]),
        "verbose_json",
        duration,
        None,
    );
    assert_eq!(
        built.get("duration").cloned().unwrap_or(Value::Null),
        expected
    );
}

#[test]
fn plain_formats_drop_words_and_duration() {
    let built = response(
        json!([{"transcript": "hi", "words": [{"word": "hi", "start_time_ms": 0, "end_time_ms": 100}]}]),
        "json",
        Some(1.5),
        Some(&["word"][..]),
    );
    assert_eq!(built, json!({"text": "hi", "task": "transcribe"}));
}

#[test]
fn empty_final_results_yield_an_empty_text() {
    let built = response(json!([]), "json", None, None);
    assert_eq!(built, json!({"text": "", "task": "transcribe"}));
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
#[case::numeric_name(returned_code(Json::Number(Number::from(5))))]
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
#[case::number(DetailsAttribute::Returns { value: Json::Number(Number::from(5)) }, "boom")]
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
