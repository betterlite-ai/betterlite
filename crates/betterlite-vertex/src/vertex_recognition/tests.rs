#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::reasoning_models::DropParams;
use http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use rstest::rstest;
use secrecy::SecretString;

use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::vertex_recognition::error::VertexRecognitionError;
use crate::vertex_recognition::request::{
    complete_url, map_openai_params, recognize_request, validate_environment,
    validate_speech_project,
};
use crate::vertex_recognition::response::{
    TranscriptionTask, decode_vertex_recognition, parse_duration_seconds,
};

fn audio() -> ProcessedAudioFile {
    ProcessedAudioFile {
        file_content: b"audio".to_vec(),
        filename: "clip.wav".to_owned(),
        content_type: "audio/wav".to_owned(),
    }
}

fn object(source: &str) -> Object {
    serde_json::from_str(source).unwrap()
}

fn reply(status: u16, body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

fn answer(status: u16, body: &str) -> Object {
    let decoded = decode_vertex_recognition(&reply(status, body)).unwrap();
    serde_json::from_value(serde_json::to_value(decoded).unwrap()).unwrap()
}

#[rstest]
#[case::default_location(
    None,
    None,
    "https://us-speech.googleapis.com/v2/projects/my-project/locations/us/recognizers/_:recognize"
)]
#[case::global(
    None,
    Some("global"),
    "https://speech.googleapis.com/v2/projects/my-project/locations/global/recognizers/_:recognize"
)]
#[case::region(
    None,
    Some("europe-west1"),
    "https://europe-west1-speech.googleapis.com/v2/projects/my-project/locations/europe-west1/recognizers/_:recognize"
)]
#[case::custom_base(
    Some("https://proxy.example///"),
    None,
    "https://proxy.example/v2/projects/my-project/locations/us/recognizers/_:recognize"
)]
#[case::custom_base_keeps_region(
    Some("https://proxy.example"),
    Some("global"),
    "https://proxy.example/v2/projects/my-project/locations/global/recognizers/_:recognize"
)]
fn recognition_url_cases(
    #[case] api_base: Option<&str>,
    #[case] location: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(
        complete_url(api_base, location, Some("my-project"), None).unwrap(),
        expected
    );
}

#[test]
fn a_credential_project_fills_a_missing_project() {
    let url = complete_url(None, None, None, Some("credential-project")).unwrap();
    assert_eq!(
        url,
        "https://us-speech.googleapis.com/v2/projects/credential-project/locations/us/recognizers/_:recognize"
    );
}

#[test]
fn a_missing_project_is_required() {
    let error = complete_url(None, None, None, None).unwrap_err();
    assert!(matches!(error, VertexRecognitionError::ProjectRequired));
    assert_eq!(error.status_code(), Some(400));
}

#[rstest]
#[case::empty("")]
#[case::parent("..")]
#[case::slash("bad/project")]
#[case::question("bad?project")]
#[case::hash("bad#project")]
#[case::backslash("bad\\project")]
#[case::colon("bad:project")]
#[case::space("bad project")]
#[case::tab("bad\tproject")]
#[case::newline("bad\nproject")]
#[case::carriage_return("bad\rproject")]
fn unsafe_projects_are_rejected(#[case] project: &str) {
    let error = validate_speech_project(project).unwrap_err();
    assert!(matches!(error, VertexRecognitionError::Project { .. }));
    assert_eq!(error.status_code(), Some(400));
}

#[rstest]
#[case::uppercase("US")]
#[case::slash("us/")]
#[case::digits_first("1us")]
fn invalid_locations_are_rejected(#[case] location: &str) {
    let error = complete_url(None, Some(location), Some("my-project"), None).unwrap_err();
    assert!(matches!(error, VertexRecognitionError::Location { .. }));
    assert_eq!(error.status_code(), Some(400));
}

#[rstest]
#[case::supported_language(r#"{"language": "en"}"#, "{}", r#"{"language": "en"}"#)]
#[case::supported_format(
    r#"{"response_format": "text"}"#,
    "{}",
    r#"{"response_format": "text"}"#
)]
#[case::supported_json(
    r#"{"response_format": "json"}"#,
    "{}",
    r#"{"response_format": "json"}"#
)]
#[case::null_format(r#"{"response_format": null}"#, "{}", r#"{"response_format": null}"#)]
#[case::unsupported_name(r#"{"temperature": 1}"#, "{}", "{}")]
#[case::overwrites_optional(
    r#"{"language": "en"}"#,
    r#"{"language": "de"}"#,
    r#"{"language": "en"}"#
)]
fn map_openai_params_cases(
    #[case] non_default: &str,
    #[case] optional: &str,
    #[case] expected: &str,
) {
    let mapped = map_openai_params(
        &object(non_default),
        &object(optional),
        DropParams::default(),
    );
    assert_eq!(mapped.unwrap(), object(expected));
}

#[test]
fn an_unsupported_response_format_is_rejected_without_drop_params() {
    let error = map_openai_params(
        &object(r#"{"response_format": "srt"}"#),
        &Object::new(),
        DropParams::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        VertexRecognitionError::ResponseFormat { .. }
    ));
    assert_eq!(error.status_code(), Some(400));
}

#[test]
fn an_unsupported_response_format_leaves_a_supplied_option_too() {
    let error = map_openai_params(
        &Object::new(),
        &object(r#"{"response_format": "vtt"}"#),
        DropParams::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        VertexRecognitionError::ResponseFormat { .. }
    ));
}

#[rstest]
#[case::request_drop(DropParams {
    global: false,
    request: true,
})]
#[case::global_drop(DropParams {
    global: true,
    request: false,
})]
fn drop_params_removes_the_unsupported_response_format(#[case] policy: DropParams) {
    let mapped = map_openai_params(
        &object(r#"{"response_format": "srt", "language": "en"}"#),
        &Object::new(),
        policy,
    )
    .unwrap();
    assert_eq!(mapped, object(r#"{"language": "en"}"#));
}

#[test]
fn header_overrides_replace_caller_values() {
    let mut caller = HeaderMap::new();
    caller.insert(AUTHORIZATION, HeaderValue::from_static("Bearer caller-key"));
    caller.insert(
        HeaderName::from_static("x-goog-user-project"),
        HeaderValue::from_static("caller-project"),
    );
    caller.insert(CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    caller.insert(
        HeaderName::from_static("x-extra"),
        HeaderValue::from_static("kept"),
    );
    let headers =
        validate_environment(caller, &SecretString::from("token"), "forced-project").unwrap();
    assert_eq!(
        headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
        "Bearer token"
    );
    assert_eq!(
        headers
            .get("x-goog-user-project")
            .unwrap()
            .to_str()
            .unwrap(),
        "forced-project"
    );
    assert_eq!(
        headers.get(CONTENT_TYPE).unwrap().to_str().unwrap(),
        "application/json"
    );
    assert_eq!(headers.get("x-extra").unwrap().to_str().unwrap(), "kept");
    assert_eq!(headers.len(), 4);
}

#[rstest]
#[case::absent(None, "auto")]
#[case::empty(Some(Json::Text(String::new())), "auto")]
#[case::not_text(Some(Json::Bool(true)), "auto")]
#[case::bare(Some(Json::Text("en".to_owned())), "en-US")]
#[case::qualified(Some(Json::Text("en-gb".to_owned())), "en-gb")]
#[case::unknown(Some(Json::Text("zz".to_owned())), "zz")]
fn recognize_request_languages(#[case] language: Option<Json>, #[case] expected: &str) {
    let request = recognize_request("gemini-2.0-flash", &audio(), language.as_ref());
    assert_eq!(request.config.language_codes, [expected]);
}

#[test]
fn the_request_matches_the_recognize_body() {
    let request = recognize_request("vertex_ai/gemini-2.0-flash", &audio(), None);
    assert_eq!(
        serde_json::to_string(&request).unwrap(),
        r#"{"config":{"model":"gemini-2.0-flash","languageCodes":["auto"],"features":{"enableAutomaticPunctuation":true},"autoDecodingConfig":{}},"content":"YXVkaW8="}"#
    );
}

#[test]
fn the_request_strips_one_vertex_ai_prefix() {
    let request = recognize_request("vertex_ai/vertex_ai/model", &audio(), None);
    assert_eq!(request.config.model, "vertex_ai/model");
}

#[test]
fn the_response_keeps_the_first_alternative_of_each_result() {
    let body = r#"{"results":[
        {"alternatives":[{"transcript":"hello"},{"transcript":"ignored"}],"languageCode":"en"},
        {"alternatives":[{"transcript":""},{"transcript":"never"}],"languageCode":"de"},
        {"alternatives":[],"languageCode":""},
        {"alternatives":[{"transcript":"world"}]}
    ]}"#;
    let value = answer(200, body);
    assert_eq!(value["text"], "hello world");
    assert_eq!(value["task"], "transcribe");
    assert_eq!(value["language"], "en");
    assert!(value.get("duration").is_none());
}

#[test]
fn a_later_alternative_never_fills_an_empty_first() {
    let body = r#"{"results":[{"alternatives":[{"transcript":null},{"transcript":"never"}]}]}"#;
    assert_eq!(answer(200, body)["text"], "");
}

#[test]
fn missing_results_and_alternatives_default_to_empty() {
    assert_eq!(answer(200, "{}")["text"], "");
    assert_eq!(answer(200, r#"{"results":[]}"#)["text"], "");
    assert_eq!(
        answer(200, r#"{"results":[{"alternatives":[]}]}"#)["text"],
        ""
    );
    assert_eq!(answer(200, r#"{"metadata":null}"#)["text"], "");
}

#[test]
fn extra_provider_fields_are_ignored_and_stored() {
    let body = r#"{"results":[{"alternatives":[{"transcript":"hi","confidence":0.5}],"resultEndTime":"1.5s"}],"requestId":"r1","metadata":{"totalBilledDuration":"2s","model":"m"}}"#;
    let value = answer(200, body);
    assert_eq!(value["text"], "hi");
    assert_eq!(value["duration"].to_string(), "2.0");
    let decoded = decode_vertex_recognition(&reply(200, body)).unwrap();
    let request_id = decoded.metadata.native_fields.get("requestId");
    assert_eq!(
        request_id.map(ToString::to_string).as_deref(),
        Some(r#""r1""#)
    );
    let result = &decoded.metadata.results[0];
    let end_time = result.native_fields.get("resultEndTime");
    assert_eq!(
        end_time.map(ToString::to_string).as_deref(),
        Some(r#""1.5s""#)
    );
    let confidence = result.alternatives[0].native_fields.get("confidence");
    assert_eq!(confidence.map(ToString::to_string).as_deref(), Some("0.5"));
    let metadata = decoded.metadata.metadata.unwrap();
    let model = metadata.native_fields.get("model");
    assert_eq!(model.map(ToString::to_string).as_deref(), Some(r#""m""#));
}

#[test]
fn a_null_results_field_is_a_shape_error() {
    let error = decode_vertex_recognition(&reply(200, r#"{"results":null}"#)).unwrap_err();
    assert!(matches!(error, VertexRecognitionError::Shape { .. }));
    assert_eq!(error.status_code(), None);
    let nested = decode_vertex_recognition(&reply(200, r#"{"results":[{"alternatives":null}]}"#))
        .unwrap_err();
    assert!(matches!(nested, VertexRecognitionError::Shape { .. }));
}

#[test]
fn a_reply_over_the_size_limit_is_refused() {
    let at_limit = format!("{{}}{}", " ".repeat(MAX_RESPONSE_BYTES - 2));
    assert_eq!(at_limit.len(), MAX_RESPONSE_BYTES);
    decode_vertex_recognition(&reply(200, &at_limit)).unwrap();
    let error = decode_vertex_recognition(&reply(200, &format!("{at_limit} "))).unwrap_err();
    assert!(matches!(
        error,
        VertexRecognitionError::TooLarge { size } if size == MAX_RESPONSE_BYTES + 1
    ));
    assert_eq!(error.status_code(), None);
}

#[test]
fn a_non_json_reply_keeps_the_reply_status() {
    let error = decode_vertex_recognition(&reply(201, "not json")).unwrap_err();
    assert!(matches!(
        error,
        VertexRecognitionError::NotJson { status: 201, .. }
    ));
    assert_eq!(error.status_code(), Some(201));
    let trailing = decode_vertex_recognition(&reply(200, r#"{"results":[]} tail"#)).unwrap_err();
    assert!(matches!(trailing, VertexRecognitionError::NotJson { .. }));
}

#[test]
fn the_response_hides_the_provider_envelope() {
    let body = r#"{"results":[{"alternatives":[{"transcript":"hi"}],"languageCode":"en"}],"metadata":{"totalBilledDuration":"1s"}}"#;
    let decoded = decode_vertex_recognition(&reply(200, body)).unwrap();
    assert_eq!(decoded.task, TranscriptionTask::Transcribe);
    assert_eq!(decoded.metadata.results.len(), 1);
    assert_eq!(
        serde_json::to_string(&decoded).unwrap(),
        r#"{"text":"hi","task":"transcribe","language":"en","duration":1.0}"#
    );
}

#[rstest]
#[case::absent(None, None)]
#[case::no_suffix(Some("2.5"), None)]
#[case::suffix_only(Some("s"), None)]
#[case::text(Some("abc"), None)]
#[case::mixed(Some("2ms"), None)]
#[case::seconds(Some("2.5s"), Some("2.5"))]
#[case::integral(Some("2s"), Some("2"))]
#[case::negative(Some("-1.5s"), Some("-1.5"))]
#[case::scientific(Some("1e3s"), Some("1000"))]
#[case::not_a_number(Some("nans"), None)]
#[case::infinity(Some("infs"), None)]
#[case::infinity_word(Some("infinitys"), None)]
fn duration_cases(#[case] duration: Option<&str>, #[case] expected: Option<&str>) {
    let parsed = parse_duration_seconds(duration).map(|seconds| seconds.to_string());
    assert_eq!(parsed.as_deref(), expected);
}

#[test]
fn the_malformed_duration_is_omitted() {
    let body = r#"{"results":[],"metadata":{"totalBilledDuration":"2ms"}}"#;
    assert!(answer(200, body).get("duration").is_none());
    let valid = r#"{"results":[],"metadata":{"totalBilledDuration":"2.5s"}}"#;
    assert_eq!(answer(200, valid)["duration"].to_string(), "2.5");
}

const VALID_SEEDS: [(&str, &str, Option<&str>); 4] = [
    ("valid-empty.json", "", None),
    ("valid-extra-fields.json", "hi", Some("1.0")),
    ("valid-malformed-duration.json", "hi", None),
    ("valid-transcripts.json", "hello", Some("2.5")),
];

#[rstest]
fn replays_the_vertex_recognition_corpus(
    #[files("fuzz/corpus/vertex_recognition/*")] path: PathBuf,
) {
    let name = path.file_name().unwrap().to_str().unwrap();
    let body = fs::read_to_string(&path).unwrap();
    if name == "invalid-null-results.json" {
        let error = decode_vertex_recognition(&reply(200, &body)).unwrap_err();
        assert!(matches!(error, VertexRecognitionError::Shape { .. }));
        return;
    }
    let (_, text, duration) = VALID_SEEDS.iter().find(|seed| seed.0 == name).unwrap();
    let value = answer(200, &body);
    assert_eq!(value["text"], *text);
    let rendered = value.get("duration").map(ToString::to_string);
    assert_eq!(rendered.as_deref(), *duration);
}
