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
    AudioTranscriptionData, AudioTranscriptionRequestData,
};
use betterlite_openai::openai_base_config::Object;

use super::support::{
    Entry, Events, Fixture, ProxiedSource, RecordingConfig, call, object, record,
};
use crate::api::transcription_http::TranscriptionDeps;
use crate::api::transcription_http::reply::TranscriptionReply;

#[tokio::test]
async fn transcription_http_over_a_socket() {
    assert_json_post_reaches_the_server().await;
    assert_raw_bytes_post_reaches_the_server().await;
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
    let mut config = RecordingConfig::new(
        record(AudioTranscriptionData::Object(Object::new())),
        &events,
    );
    config.response =
        serde_json::from_str(r#"{"text":"live"}"#).expect("the response fixture is valid");
    let optional = object("{}");
    let litellm = object("{}");
    let mut request_call = call(&optional, &litellm);
    request_call.api_base = Some(&api_base);
    let deps = TranscriptionDeps {
        config: Some(&config),
        source: &source,
        supplied: None,
    };
    let mut fixture = Fixture::new("transcription");
    let answer = fixture
        .run(&events, &deps, &request_call, Entry::Audio)
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
