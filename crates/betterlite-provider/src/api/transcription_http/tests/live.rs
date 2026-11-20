use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::request::Request;
use betterlite_http_fixture::server::serve;
use betterlite_openai::api::transcription_contract::{
    AudioTranscriptionData, AudioTranscriptionFileValue, AudioTranscriptionFiles,
    AudioTranscriptionRequestData,
};

use super::support::{
    Entry, Events, Fixture, ProxiedSource, RecordingConfig, call, object, record,
};
use crate::api::transcription_http::TranscriptionDeps;
use crate::api::transcription_http::error::TranscriptionHttpError;
use crate::api::transcription_http::reply::TranscriptionReply;

// The declared Content-Type header a redirected multipart post must keep on the second request
const DECLARED_CONTENT_TYPE: &str = "content-type: multipart/form-data; boundary=fixture-boundary";

#[tokio::test]
async fn transcription_http_over_a_socket() {
    assert_json_post_reaches_the_server().await;
    assert_raw_bytes_post_reaches_the_server().await;
}

// The recorded deviation: betterlite-http has no raw-byte body that adds no Content-Type, so this path cannot follow
#[tokio::test]
async fn transcription_http_raw_bytes_report_a_redirect() {
    let redirecting = serve(
        reply(
            "307 Temporary Redirect",
            &[("location", "http://127.0.0.1:1/v1/audio/transcriptions")],
            "",
        ),
        Duration::ZERO,
    )
    .expect("the redirect serves");
    let events = Events::new();
    let source = proxied_source();
    let mut config = RecordingConfig::new(
        record(AudioTranscriptionData::Bytes(b"abc".to_vec())),
        &events,
    );
    config.response =
        serde_json::from_str(r#"{"text":"live"}"#).expect("the response fixture is valid");
    let api_base = format!("http://{}/v1", redirecting.address);
    let error = send(&events, &source, &config, &api_base)
        .await
        .expect_err("a raw-byte post does not follow the redirect");
    let TranscriptionHttpError::Provider(provider) = error else {
        panic!("the redirect answer is a provider error");
    };
    assert_eq!(provider.status_code, 307);
    let request = redirecting.finish().expect("the redirect answers");
    assert!(request.line.starts_with("POST /v1/audio/transcriptions "));
    assert_eq!(request.body, "abc");
}

#[tokio::test]
async fn transcription_http_follows_a_redirect() {
    let target = serve(
        reply(
            "200 OK",
            &[("content-type", "application/json")],
            r#"{"text":"live"}"#,
        ),
        Duration::ZERO,
    )
    .expect("the target serves");
    let location = format!("http://{}/v1/audio/transcriptions", target.address);
    let redirecting = serve(
        reply(
            "307 Temporary Redirect",
            &[("location", location.as_str())],
            "",
        ),
        Duration::ZERO,
    )
    .expect("the redirect serves");
    let api_base = format!("http://{}/v1", redirecting.address);
    let events = Events::new();
    let source = proxied_source();
    let mut config = RecordingConfig::new(declared_multipart(), &events);
    config.response =
        serde_json::from_str(r#"{"text":"live"}"#).expect("the response fixture is valid");
    let answer = send(&events, &source, &config, &api_base)
        .await
        .expect("the redirected post reaches the transcription");
    assert_eq!(answer.response.text.as_deref(), Some("live"));
    let first = redirecting.finish().expect("the redirect answers");
    let second = target.finish().expect("the target answers");
    assert!(first.line.starts_with("POST /v1/audio/transcriptions "));
    assert!(second.line.starts_with("POST /v1/audio/transcriptions "));
    assert_eq!(first.body, second.body);
    assert!(second.head.contains(DECLARED_CONTENT_TYPE));
}

fn declared_multipart() -> AudioTranscriptionRequestData {
    let mut data = record(AudioTranscriptionData::Object(object(
        r#"{"query":"hello"}"#,
    )));
    data.files = Some(AudioTranscriptionFiles::from([(
        String::from("a.wav"),
        AudioTranscriptionFileValue::Text(String::from("abc")),
    )]));
    data.content_type = Some(String::from(
        "multipart/form-data; boundary=fixture-boundary",
    ));
    data
}

async fn send(
    events: &Events,
    source: &ProxiedSource,
    config: &RecordingConfig,
    api_base: &str,
) -> Result<TranscriptionReply, TranscriptionHttpError> {
    let optional = object("{}");
    let litellm = object("{}");
    let mut request_call = call(&optional, &litellm);
    request_call.api_base = Some(api_base);
    let deps = TranscriptionDeps {
        config: Some(config),
        source,
        supplied: None,
    };
    let mut fixture = Fixture::new("transcription");
    fixture
        .run(events, &deps, &request_call, Entry::Audio)
        .await
}

fn proxied_source() -> ProxiedSource {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    ProxiedSource {
        client: ProxiedClient::outbound(&policy).expect("the proxied client builds"),
    }
}

async fn exchange(data: AudioTranscriptionRequestData) -> (Request, TranscriptionReply) {
    let serving = serve(
        reply(
            "200 OK",
            &[
                ("content-type", "application/json"),
                ("x-request-id", "transcribe_1"),
            ],
            r#"{"text":"live"}"#,
        ),
        Duration::ZERO,
    )
    .expect("the fixture serves");
    let api_base = format!("http://{}/v1", serving.address);
    let events = Events::new();
    let source = proxied_source();
    let mut config = RecordingConfig::new(data, &events);
    config.response =
        serde_json::from_str(r#"{"text":"live"}"#).expect("the response fixture is valid");
    let answer = send(&events, &source, &config, &api_base)
        .await
        .expect("the live call succeeds");
    let request = serving.finish().expect("the fixture answers");
    (request, answer)
}

async fn assert_json_post_reaches_the_server() {
    let (request, answer) = exchange(record(AudioTranscriptionData::Object(object(
        r#"{"query":"hello"}"#,
    ))))
    .await;
    assert!(request.line.starts_with("POST /v1/audio/transcriptions "));
    assert!(request.head.contains("content-type: application/json"));
    assert!(request.head.contains("authorization: Bearer sk-provider"));
    assert_eq!(request.body, r#"{"query":"hello"}"#);
    assert_eq!(answer.response.text.as_deref(), Some("live"));
    assert_eq!(
        answer
            .provider_headers
            .headers
            .get("x-request-id")
            .map(String::as_str),
        Some("transcribe_1")
    );
}

async fn assert_raw_bytes_post_reaches_the_server() {
    let (request, answer) = exchange(record(AudioTranscriptionData::Bytes(b"abc".to_vec()))).await;
    assert!(request.line.starts_with("POST /v1/audio/transcriptions "));
    assert!(!request.head.to_ascii_lowercase().contains("content-type:"));
    assert_eq!(request.body, "abc");
    assert_eq!(answer.response.text.as_deref(), Some("live"));
}
