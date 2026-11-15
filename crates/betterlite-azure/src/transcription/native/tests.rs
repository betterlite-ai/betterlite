#![cfg(test)]

use std::io::{self, Read};
use std::path::{Path, PathBuf};

use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::{AudioFile, AuthorizedAudioFiles};
use betterlite_openai::api::transcription_contract::AudioTranscriptionData;
use betterlite_openai::openai_base_config::Headers;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;

use super::reply::{NativeTranscriptionReply, decode_native_transcription};
use super::{
    NativeError, SUPPORTED_OPENAI_PARAMS, complete_url, get_error_class, map_openai_params,
    resolve_stt_base_url, transform_request, validate_environment,
};

const MISSING_BASE: &str = "api_base is required for Azure AI Speech transcription. Use a Cognitive Services endpoint like https://{region}.api.cognitive.microsoft.com or an STT endpoint like https://{region}.stt.speech.microsoft.com.";
const NOT_SPEECH: &str = "Azure AI Speech transcription requires a Cognitive Services or STT Speech endpoint, not an Azure OpenAI endpoint.";

struct NoFiles;

impl AuthorizedAudioFiles for NoFiles {
    fn read(&self, _path: &Path) -> io::Result<Vec<u8>> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no authorized files",
        ))
    }

    fn open(&self, _path: &Path) -> io::Result<Box<dyn Read + Send>> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no authorized files",
        ))
    }
}

fn params(pairs: &[(&str, Json)]) -> IndexMap<String, Json> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn decode(body: &str) -> Result<NativeTranscriptionReply, NativeError> {
    decode_native_transcription(200, &headers(&[("x-request-id", "req-1")]), body.as_bytes())
}

fn native_status(error: &NativeError) -> (Option<u16>, String) {
    (error.status(), error.to_string())
}

#[rstest]
#[case::bare_domain(
    "https://api.cognitive.microsoft.com",
    "https://stt.speech.microsoft.com"
)]
#[case::regional(
    "https://eastus.api.cognitive.microsoft.com/",
    "https://eastus.stt.speech.microsoft.com"
)]
#[case::upper_case(
    "https://WESTUS.api.cognitive.microsoft.com///",
    "https://westus.stt.speech.microsoft.com"
)]
fn cognitive_hosts_become_regional_stt(#[case] base: &str, #[case] expected: &str) {
    assert_eq!(resolve_stt_base_url(base).unwrap(), expected);
}

#[rstest]
#[case::exact_host(
    "https://westus.stt.speech.microsoft.com",
    "https://westus.stt.speech.microsoft.com"
)]
#[case::port_path_query_dropped(
    "http://stt.speech.microsoft.com:8080/path?query=1",
    "http://stt.speech.microsoft.com"
)]
#[case::trailing_slash(
    "https://eastus.stt.speech.microsoft.com/",
    "https://eastus.stt.speech.microsoft.com"
)]
fn stt_hosts_keep_scheme_and_host(#[case] base: &str, #[case] expected: &str) {
    assert_eq!(resolve_stt_base_url(base).unwrap(), expected);
}

#[rstest]
#[case::custom("https://speech.test/custom", "https://speech.test/custom")]
#[case::trailing_slashes("https://speech.test/custom///", "https://speech.test/custom")]
#[case::bare_host("speech.test", "speech.test")]
#[case::empty("", "")]
fn custom_bases_keep_trimmed_text(#[case] base: &str, #[case] expected: &str) {
    assert_eq!(resolve_stt_base_url(base).unwrap(), expected);
}

#[test]
fn azure_openai_host_is_refused() {
    let error = resolve_stt_base_url("https://example.openai.azure.com/").unwrap_err();
    assert_eq!(native_status(&error), (Some(400), NOT_SPEECH.to_owned()));
    assert!(matches!(error, NativeError::AzureOpenAiEndpoint));
    // the suffix check needs the dot: a host that merely ends in the letters is a custom base
    assert_eq!(
        resolve_stt_base_url("https://example.openai.azure.com.evil").unwrap(),
        "https://example.openai.azure.com.evil"
    );
}

#[test]
fn completion_url_appends_endpoint_and_query() {
    let default = params(&[]);
    assert_eq!(
        complete_url(
            Some("https://eastus.api.cognitive.microsoft.com"),
            None,
            &default
        )
        .unwrap(),
        "https://eastus.stt.speech.microsoft.com/speech/recognition/conversation/cognitiveservices/v1?language=en-US&format=simple"
    );
    let verbose = params(&[
        ("language", text("zh CN")),
        ("response_format", text("verbose_json")),
    ]);
    assert_eq!(
        complete_url(
            Some("https://eastus.stt.speech.microsoft.com"),
            None,
            &verbose
        )
        .unwrap(),
        "https://eastus.stt.speech.microsoft.com/speech/recognition/conversation/cognitiveservices/v1?language=zh+CN&format=detailed"
    );
    let null = params(&[("language", Json::Null), ("response_format", Json::Null)]);
    assert!(
        complete_url(Some("https://eastus.stt.speech.microsoft.com"), None, &null)
            .unwrap()
            .ends_with("language=en-US&format=simple")
    );
}

#[rstest]
#[case::json("json")]
#[case::text("text")]
#[case::srt("srt")]
#[case::vtt("vtt")]
#[case::diarized_json("diarized_json")]
fn other_response_formats_map_to_simple(#[case] format: &str) {
    let mapped = params(&[("response_format", text(format))]);
    assert!(
        complete_url(
            Some("https://eastus.stt.speech.microsoft.com"),
            None,
            &mapped
        )
        .unwrap()
        .ends_with("&format=simple")
    );
}

#[test]
fn completion_url_requires_a_base() {
    let error = complete_url(None, None, &params(&[])).unwrap_err();
    assert_eq!(native_status(&error), (Some(400), MISSING_BASE.to_owned()));
    assert_eq!(
        complete_url(
            None,
            Some("https://eastus.api.cognitive.microsoft.com/"),
            &params(&[])
        )
        .unwrap(),
        "https://eastus.stt.speech.microsoft.com/speech/recognition/conversation/cognitiveservices/v1?language=en-US&format=simple"
    );
    assert_eq!(
        complete_url(
            Some("https://speech.test"),
            Some("https://ignored.test"),
            &params(&[])
        )
        .unwrap(),
        "https://speech.test/speech/recognition/conversation/cognitiveservices/v1?language=en-US&format=simple"
    );
    let wrong: IndexMap<String, Json> = params(&[("language", Json::Bool(true))]);
    let field = complete_url(Some("https://speech.test"), None, &wrong).unwrap_err();
    assert_eq!(
        native_status(&field),
        (
            None,
            "the transcription field `language` is not a string".to_owned()
        )
    );
}

#[test]
fn environment_headers_overwrite_key_and_keep_content_type() {
    let explicit = SecretString::from("fixture-key");
    let caller = headers(&[
        ("Ocp-Apim-Subscription-Key", "stale"),
        ("Content-Type", "audio/wav; codecs=1"),
        ("X-Trace", "1"),
    ]);
    let validated = validate_environment(&caller, Some(&explicit), None).unwrap();
    assert_eq!(
        validated,
        headers(&[
            ("Ocp-Apim-Subscription-Key", "fixture-key"),
            ("Content-Type", "audio/wav; codecs=1"),
            ("X-Trace", "1"),
            ("Accept", "application/json"),
        ])
    );
    let environment = SecretString::from("env-key");
    let defaulted = validate_environment(&headers(&[]), None, Some(&environment)).unwrap();
    assert_eq!(
        defaulted,
        headers(&[
            ("Ocp-Apim-Subscription-Key", "env-key"),
            ("Content-Type", "audio/wav"),
            ("Accept", "application/json"),
        ])
    );
    let error = validate_environment(&headers(&[]), None, None).unwrap_err();
    assert_eq!(
        native_status(&error),
        (
            Some(401),
            "api_key is required for Azure AI Speech transcription.".to_owned()
        )
    );
}

#[test]
fn supported_params_and_mapping() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS.to_vec(),
        vec!["language", "response_format"]
    );
    let non_default = params(&[
        ("response_format", text("verbose_json")),
        ("prompt", text("ignored")),
        ("language", text("fr")),
    ]);
    let existing = params(&[("language", text("en")), ("response_format", text("json"))]);
    assert_eq!(
        map_openai_params(&non_default, existing),
        params(&[
            ("language", text("fr")),
            ("response_format", text("verbose_json"))
        ])
    );
    // keys the owned config does not support never arrive, whatever drop_params says
    assert_eq!(
        map_openai_params(&non_default, params(&[])),
        params(&[
            ("response_format", text("verbose_json")),
            ("language", text("fr"))
        ])
    );
}

#[rstest]
#[case::display_only(r#"{"DisplayText":"hello"}"#)]
#[case::null_status(r#"{"RecognitionStatus":null,"DisplayText":"hello"}"#)]
#[case::success_status(r#"{"RecognitionStatus":"Success","DisplayText":"hello"}"#)]
fn native_reply_accepts_missing_and_success_status(#[case] body: &str) {
    let reply = decode(body).unwrap();
    assert_eq!(reply.response.text.as_deref(), Some("hello"));
    assert_eq!(
        reply.hidden_params,
        serde_json::from_str::<Json>(body).unwrap()
    );
}

#[rstest]
#[case::empty_display_text(r#"{"DisplayText":""}"#, "")]
#[case::nbest_display(r#"{"NBest":[{"Display":"best"}]}"#, "best")]
#[case::nbest_lexical(r#"{"NBest":[{"Display":"","Lexical":"lexical"}]}"#, "lexical")]
#[case::nbest_null_display(r#"{"NBest":[{"Display":null,"Lexical":"lexical"}]}"#, "lexical")]
#[case::nbest_empty_display(r#"{"NBest":[{"Display":""}]}"#, "")]
#[case::nbest_empty_item(r#"{"NBest":[{}]}"#, "")]
#[case::nbest_empty_list(r#"{"NBest":[]}"#, "")]
#[case::nbest_text_item(r#"{"NBest":["best"]}"#, "")]
#[case::nbest_object(r#"{"NBest":{"Display":"best"}}"#, "")]
#[case::empty_object(r#"{}"#, "")]
fn native_reply_falls_back_to_nbest(#[case] body: &str, #[case] expected: &str) {
    assert_eq!(
        decode(body).unwrap().response.text.as_deref(),
        Some(expected),
        "{body}"
    );
}

#[test]
fn native_reply_refuses_wrongly_typed_nbest_fields() {
    for body in [
        r#"{"NBest":[{"Display":{"nested":1}}]}"#,
        r#"{"NBest":[{"Display":7}]}"#,
    ] {
        let error = decode(body).unwrap_err();
        assert_eq!(
            native_status(&error),
            (
                None,
                "the transcription field `NBest[0].Display` is not a string".to_owned()
            ),
            "{body}"
        );
    }
    let lexical = decode(r#"{"NBest":[{"Lexical":5}]}"#).unwrap_err();
    assert_eq!(
        native_status(&lexical),
        (
            None,
            "the transcription field `NBest[0].Lexical` is not a string".to_owned()
        )
    );
}

#[test]
fn recognition_failure_keeps_raw_status_and_headers() {
    let failure_headers = headers(&[
        ("x-request-id", "req-1"),
        ("content-type", "application/json"),
    ]);
    let error = decode_native_transcription(
        200,
        &failure_headers,
        br#"{"RecognitionStatus":"InitialSilenceTimeout","DisplayText":"ignored"}"#,
    )
    .unwrap_err();
    assert_eq!(
        native_status(&error),
        (
            Some(200),
            "Azure AI Speech transcription failed with RecognitionStatus=InitialSilenceTimeout."
                .to_owned()
        )
    );
    assert_eq!(error.headers(), Some(&failure_headers));
    // LiteLLM accepts only the exact text; every other string is a failure
    assert!(decode(r#"{"RecognitionStatus":"success"}"#).is_err());
    assert!(decode(r#"{"RecognitionStatus":false}"#).is_err());
}

#[rstest]
#[case::text(Json::Text(String::from("Error")), "Error")]
#[case::number(Json::Number(Number::from(1)), "1")]
#[case::true_status(Json::Bool(true), "true")]
#[case::false_status(Json::Bool(false), "false")]
#[case::empty_list(Json::List(Vec::new()), "[]")]
#[case::object(
    Json::Object(IndexMap::from([(
        String::from("a"),
        Json::Number(Number::from(1)),
    )])),
    r#"{"a":1}"#
)]
fn recognition_failure_renders_any_status(#[case] status: Json, #[case] expected: &str) {
    let body = serde_json::to_string(&Json::Object(IndexMap::from([(
        String::from("RecognitionStatus"),
        status,
    )])))
    .unwrap();
    let failure_headers = headers(&[("x-request-id", "req-1")]);
    let error = decode_native_transcription(200, &failure_headers, body.as_bytes()).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("Azure AI Speech transcription failed with RecognitionStatus={expected}.")
    );
    assert_eq!(error.status(), Some(200));
}

#[test]
fn native_reply_ignores_the_http_status_of_a_passing_body() {
    // LiteLLM reads only the body, so a 503 that still carries DisplayText yields the text
    let reply =
        decode_native_transcription(503, &headers(&[]), br#"{"DisplayText":"hello"}"#).unwrap();
    assert_eq!(reply.response.text.as_deref(), Some("hello"));
}

#[test]
fn non_json_and_non_object_bodies_are_refused() {
    let json = decode_native_transcription(200, &headers(&[]), b"{not json").unwrap_err();
    assert!(matches!(json, NativeError::Json { .. }));
    let array = decode_native_transcription(200, &headers(&[]), br#"["x"]"#).unwrap_err();
    assert!(matches!(array, NativeError::NotObject));
    assert_eq!(array.status(), None);
    let text = decode_native_transcription(200, &headers(&[]), b"hello").unwrap_err();
    assert!(matches!(text, NativeError::Json { .. }));
}

#[test]
fn request_transform_uses_bytes_and_content_type() {
    let call = transform_request(AudioFile::Bytes(b"abc"), &NoFiles).unwrap();
    assert_eq!(call.data, AudioTranscriptionData::Bytes(b"abc".to_vec()));
    assert_eq!(call.files, None);
    assert_eq!(call.content_type.as_deref(), Some("audio/wav"));
    let unsupported = transform_request(AudioFile::Text("a.wav"), &NoFiles).unwrap_err();
    assert!(matches!(unsupported, NativeError::AudioFile(_)));
    let path = PathBuf::from("missing.wav");
    let missing = transform_request(AudioFile::Path(&path), &NoFiles).unwrap_err();
    assert!(matches!(missing, NativeError::AudioFile(_)));
}

#[test]
fn error_class_carries_message_status_and_headers() {
    let failure_headers = headers(&[("x-request-id", "req-1")]);
    let error = get_error_class(
        String::from("Azure AI Speech transcription failed."),
        502,
        failure_headers.clone(),
    );
    assert_eq!(error.to_string(), "Azure AI Speech transcription failed.");
    assert_eq!(error.status(), Some(502));
    assert_eq!(error.headers(), Some(&failure_headers));
}
