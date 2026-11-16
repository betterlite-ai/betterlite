#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Arc;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use http::HeaderMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::WatsonxAudioConfig;
use super::error::WatsonxAudioError;
use super::request::SUPPORTED_OPENAI_PARAMS;
use crate::api::audio_file_input::ProcessedAudioFile;
use crate::openai_base_config::Object;
use crate::watsonx_auth::WatsonxAuth;
use crate::watsonx_auth::environment::WatsonxEnvironment;
use crate::watsonx_auth::transport::WatsonxTransport;

struct NoTransport;

impl WatsonxTransport for NoTransport {
    fn send_buffered(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(Err(TransportError::Closed))
    }
}

fn environment(values: &[(&str, &str)]) -> WatsonxEnvironment {
    let find = |name: &str| {
        values
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| *value)
    };
    let secret = |name: &str| find(name).map(|value| SecretString::from(value.to_owned()));
    let text = |name: &str| find(name).map(str::to_owned);
    WatsonxEnvironment {
        watsonx_iam_url: text("WATSONX_IAM_URL"),
        wx_api_key: secret("WX_API_KEY"),
        watsonx_api_key: secret("WATSONX_API_KEY"),
        watsonx_apikey: secret("WATSONX_APIKEY"),
        watsonx_zenapikey: secret("WATSONX_ZENAPIKEY"),
        watsonx_token: secret("WATSONX_TOKEN"),
        watsonx_api_base: text("WATSONX_API_BASE"),
        watsonx_url: text("WATSONX_URL"),
        wx_url: text("WX_URL"),
        wml_url: text("WML_URL"),
        watsonx_project_id: text("WATSONX_PROJECT_ID"),
        wx_project_id: text("WX_PROJECT_ID"),
        project_id: text("PROJECT_ID"),
        watsonx_region: text("WATSONX_REGION"),
        wx_region: text("WX_REGION"),
        region: text("REGION"),
        watsonx_deployment_space_id: text("WATSONX_DEPLOYMENT_SPACE_ID"),
        watsonx_space_id: text("WATSONX_SPACE_ID"),
        wx_space_id: text("WX_SPACE_ID"),
        space_id: text("SPACE_ID"),
    }
}

fn auth(environment: WatsonxEnvironment) -> WatsonxAuth<NoTransport> {
    WatsonxAuth::new(environment, NoTransport, Arc::new(|| 0.0_f64)).unwrap()
}

fn config(auth: &WatsonxAuth<NoTransport>) -> WatsonxAudioConfig<'_, NoTransport> {
    WatsonxAudioConfig { auth }
}

fn options(wire: &str) -> Object {
    serde_json::from_str(wire).unwrap()
}

fn headers(wire: &[(&str, &str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in wire {
        headers.insert(
            http::header::HeaderName::try_from(*name).unwrap(),
            http::header::HeaderValue::try_from(*value).unwrap(),
        );
    }
    headers
}

fn header_map(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), value.to_str().unwrap().to_owned()))
        .collect()
}

fn header_wire(wire: &[(&str, &str)]) -> BTreeMap<String, String> {
    wire.iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn file() -> ProcessedAudioFile {
    ProcessedAudioFile {
        file_content: b"RIFF".to_vec(),
        filename: String::from("clip.mp3"),
        content_type: String::from("audio/mpeg"),
    }
}

fn fields(request: &super::request::WatsonxTranscriptionRequest) -> Value {
    json!(
        request
            .fields
            .iter()
            .map(|(name, value)| json!([name, value]))
            .collect::<Vec<_>>()
    )
}

#[test]
fn supported_params_match_litellm() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [
            "language",
            "prompt",
            "response_format",
            "temperature",
            "timestamp_granularities"
        ]
    );
}

#[tokio::test]
async fn auth_keeps_authorization_and_drops_content_type() {
    let auth = auth(WatsonxEnvironment::default());
    let resolved = config(&auth)
        .validate_environment(
            headers(&[("Authorization", "Bearer given")]),
            &Object::new(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        header_map(&resolved),
        header_wire(&[
            ("accept", "application/json"),
            ("authorization", "Bearer given"),
        ])
    );
}

#[tokio::test]
async fn auth_uses_token_and_drops_content_type() {
    let auth = auth(WatsonxEnvironment::default());
    let resolved = config(&auth)
        .validate_environment(headers(&[]), &options(r#"{"token":"t"}"#), None)
        .await
        .unwrap();
    assert_eq!(
        header_map(&resolved),
        header_wire(&[
            ("accept", "application/json"),
            ("authorization", "Bearer t"),
        ])
    );
}

#[tokio::test]
async fn auth_uses_zen_api_key() {
    let auth = auth(WatsonxEnvironment::default());
    let resolved = config(&auth)
        .validate_environment(headers(&[]), &options(r#"{"zen_api_key":"z"}"#), None)
        .await
        .unwrap();
    assert_eq!(
        resolved
            .get("authorization")
            .map(|value| value.to_str().unwrap()),
        Some("ZenApiKey z")
    );
}

#[tokio::test]
async fn auth_uses_environment_token() {
    let auth = auth(environment(&[("WATSONX_TOKEN", "envtoken")]));
    let resolved = config(&auth)
        .validate_environment(headers(&[]), &Object::new(), None)
        .await
        .unwrap();
    assert_eq!(
        resolved
            .get("authorization")
            .map(|value| value.to_str().unwrap()),
        Some("Bearer envtoken")
    );
}

#[tokio::test]
async fn auth_without_key_is_missing_iam_key() {
    let auth = auth(WatsonxEnvironment::default());
    let error = config(&auth)
        .validate_environment(headers(&[]), &Object::new(), None)
        .await
        .unwrap_err();
    assert!(matches!(error, WatsonxAudioError::Watsonx(_)));
    assert_eq!(error.status(), None);
}

#[rstest]
#[case::trailing_slash(
    "https://host/",
    r#"{}"#,
    "https://host/ml/v1/audio/transcriptions?version=2024-03-13"
)]
#[case::version_used(
    "https://host",
    r#"{"api_version":"v"}"#,
    "https://host/ml/v1/audio/transcriptions?version=v"
)]
#[case::empty_version(
    "https://host",
    r#"{"api_version":""}"#,
    "https://host/ml/v1/audio/transcriptions?version=2024-03-13"
)]
#[case::null_version(
    "https://host",
    r#"{"api_version":null}"#,
    "https://host/ml/v1/audio/transcriptions?version=2024-03-13"
)]
fn url_is_base_plus_audio_path(api_base: &str, given: &str, expected: &str) {
    let auth = auth(WatsonxEnvironment::default());
    let options = options(given);
    let before = options.clone();
    let url = config(&auth)
        .get_complete_url(Some(api_base), &options)
        .unwrap();
    assert_eq!(url, expected);
    assert_eq!(options, before);
}

#[test]
fn url_falls_back_to_environment_base() {
    let auth = auth(environment(&[("WATSONX_API_BASE", "https://env-host/")]));
    assert_eq!(
        config(&auth)
            .get_complete_url(None, &Object::new())
            .unwrap(),
        "https://env-host/ml/v1/audio/transcriptions?version=2024-03-13"
    );
}

#[test]
fn url_without_base_is_401() {
    let auth = auth(WatsonxEnvironment::default());
    let error = config(&auth)
        .get_complete_url(None, &Object::new())
        .unwrap_err();
    assert_eq!(error.status(), Some(401));
}

#[test]
fn url_with_non_text_version_names_the_field() {
    let auth = auth(WatsonxEnvironment::default());
    let error = config(&auth)
        .get_complete_url(Some("https://host"), &options(r#"{"api_version":5}"#))
        .unwrap_err();
    assert!(matches!(
        error,
        WatsonxAudioError::NotText {
            field: "api_version"
        }
    ));
}

#[rstest]
#[case::no_ids(r#"{}"#, r#"[["model","granite"]]"#)]
#[case::project_id(r#"{"project_id":"p"}"#, r#"[["model","granite"],["project_id","p"]]"#)]
#[case::watsonx_project(
    r#"{"watsonx_project":"wp"}"#,
    r#"[["model","granite"],["project_id","wp"]]"#
)]
#[case::project_wins(
    r#"{"project_id":"p","watsonx_project":"wp"}"#,
    r#"[["model","granite"],["project_id","p"]]"#
)]
#[case::null_project_falls_back(
    r#"{"project_id":null,"watsonx_project":"wp"}"#,
    r#"[["model","granite"],["project_id","wp"]]"#
)]
#[case::space_id(r#"{"space_id":"s"}"#, r#"[["model","granite"],["space_id","s"]]"#)]
#[case::project_beats_space(
    r#"{"project_id":"p","space_id":"s"}"#,
    r#"[["model","granite"],["project_id","p"]]"#
)]
#[case::null_space(r#"{"space_id":null}"#, r#"[["model","granite"]]"#)]
#[case::supported_options(r#"{"language":"en","prompt":"hi","temperature":0.2,"response_format":"json","timestamp_granularities":["word"],"model":"ignored","extra":1}"#, r#"[["model","granite"],["language","en"],["prompt","hi"],["response_format","json"],["temperature",0.2],["timestamp_granularities",["word"]]]"#)]
#[case::null_option_skipped(
    r#"{"language":null,"prompt":"hi"}"#,
    r#"[["model","granite"],["prompt","hi"]]"#
)]
fn request_fields_in_order(given: &str, expected: &str) {
    let auth = auth(WatsonxEnvironment::default());
    let request = config(&auth)
        .transform_audio_request("granite", file(), &options(given))
        .unwrap();
    assert_eq!(
        fields(&request),
        serde_json::from_str::<Value>(expected).unwrap()
    );
    assert_eq!(request.file, file());
}

#[rstest]
#[case::project_number(r#"{"project_id":5}"#, "project_id")]
#[case::watsonx_project_number(r#"{"project_id":null,"watsonx_project":5}"#, "watsonx_project")]
#[case::space_number(r#"{"space_id":5}"#, "space_id")]
fn request_rejects_non_text_ids(given: &str, field: &str) {
    let auth = auth(WatsonxEnvironment::default());
    let error = config(&auth)
        .transform_audio_request("granite", file(), &options(given))
        .unwrap_err();
    assert!(matches!(
        error,
        WatsonxAudioError::NotText { field: found } if found == field
    ));
    assert_eq!(error.status(), None);
}

#[test]
fn reply_keeps_text_and_other_keys_and_drops_model() {
    let auth = auth(WatsonxEnvironment::default());
    let response = config(&auth)
        .transform_audio_response(
            br#"{"text":"hello","model":"m","task":"transcribe","duration":2.5}"#,
        )
        .unwrap();
    assert_eq!(response.text, Some(String::from("hello")));
    assert_eq!(
        response.extra,
        options(r#"{"task":"transcribe","duration":2.5,"usage":null}"#)
    );
}

#[test]
fn reply_keeps_null_usage_as_null() {
    let auth = auth(WatsonxEnvironment::default());
    let response = config(&auth)
        .transform_audio_response(br#"{"text":"hello","usage":null}"#)
        .unwrap();
    assert_eq!(response.extra.get("usage"), Some(&Value::Null));
}

#[test]
fn reply_keeps_usage_beside_text() {
    let auth = auth(WatsonxEnvironment::default());
    let response = config(&auth)
        .transform_audio_response(br#"{"text":"hello","usage":{"total_tokens":3}}"#)
        .unwrap();
    assert_eq!(response.text, Some(String::from("hello")));
    assert_eq!(
        response.extra.get("usage"),
        Some(&json!({"total_tokens": 3}))
    );
}

#[test]
fn reply_keeps_usage_without_text() {
    let auth = auth(WatsonxEnvironment::default());
    let response = config(&auth)
        .transform_audio_response(br#"{"usage":{"total_tokens":3}}"#)
        .unwrap();
    assert_eq!(response.text, None);
    assert_eq!(
        response.extra.get("usage"),
        Some(&json!({"total_tokens": 3}))
    );
}

#[test]
fn reply_empty_text_is_kept() {
    let auth = auth(WatsonxEnvironment::default());
    let response = config(&auth)
        .transform_audio_response(br#"{"text":""}"#)
        .unwrap();
    assert_eq!(response.text, Some(String::new()));
}

#[rstest]
#[case::missing_both(br#"{"task":"transcribe"}"#)]
#[case::null_text_and_usage(br#"{"text":null,"usage":null}"#)]
#[case::only_model(br#"{"model":"m"}"#)]
fn reply_without_text_or_usage_fails(body: &[u8]) {
    let auth = auth(WatsonxEnvironment::default());
    let error = config(&auth).transform_audio_response(body).unwrap_err();
    assert!(matches!(error, WatsonxAudioError::MissingFields { .. }));
    assert_eq!(error.status(), None);
    assert!(error.to_string().contains("Invalid response format"));
    assert!(error.to_string().contains("task"));
}

#[rstest]
#[case::not_json(b"not json")]
#[case::not_an_object(b"[]")]
#[case::text_number(br#"{"text":5}"#)]
fn reply_rejects_malformed_bodies(body: &[u8]) {
    let auth = auth(WatsonxEnvironment::default());
    let error = config(&auth).transform_audio_response(body).unwrap_err();
    assert_eq!(error.status(), None);
}

#[test]
fn reply_text_number_names_the_field() {
    let auth = auth(WatsonxEnvironment::default());
    let error = config(&auth)
        .transform_audio_response(br#"{"text":5}"#)
        .unwrap_err();
    assert!(matches!(
        error,
        WatsonxAudioError::NotText { field: "text" }
    ));
}

#[test]
fn error_status_comes_from_the_shared_api_error() {
    let error = WatsonxAudioError::from(crate::watsonx_auth::error::WatsonxError::Api {
        status_code: 404,
        message: String::from("missing"),
        headers: None,
    });
    assert_eq!(error.status(), Some(404));
}

#[test]
fn replays_the_watsonx_transcription_reply_corpus() {
    let seen = std::fs::read_dir("fuzz/corpus/watsonx_transcription_reply")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .inspect(|path| {
            let bytes = std::fs::read(path).unwrap();
            let _reply = super::reply::decode_watsonx_transcription_reply(&bytes);
        })
        .count();
    assert!(seen > 0_usize);
}
