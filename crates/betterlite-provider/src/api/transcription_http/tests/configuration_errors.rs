use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::api::transcription_contract::AudioTranscriptionData;
use betterlite_openai::containers::container_file_http::request::SslVerify;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::transcription::client::SharedSession;
use serde_json::Number;

use super::support::{
    ClientCall, Entry, Events, Fixture, RecordingConfig, Reply, Source, Wire, call, deps, json,
    object, record,
};
use crate::api::transcription_http::TranscriptionDeps;
use crate::api::transcription_http::error::TranscriptionHttpError;

#[tokio::test]
async fn transcription_http_configuration_errors() {
    assert_missing_config_is_a_value_error().await;
    assert_boolean_atranscription_selects_async().await;
    assert_integer_atranscription_stays_sync().await;
    assert_max_retries_sends_once().await;
    assert_status_failure_keeps_status_body_and_headers().await;
    assert_status_failure_with_an_empty_body_has_no_length().await;
    assert_timeouts_report_the_timeout().await;
    assert_whole_second_timeouts_print_exactly().await;
}

async fn assert_missing_config_is_a_value_error() {
    for entry in [Entry::Audio, Entry::Async] {
        let events = Events::new();
        let wire = Wire::replying(Reply::json(200, "{}"), &events);
        let source = Source::new(wire.clone());
        let optional = object("{}");
        let litellm = object("{}");
        let request_call = call(&optional, &litellm);
        let deps: TranscriptionDeps<'_, RecordingConfig, Source> = TranscriptionDeps {
            config: None,
            source: &source,
            supplied: None,
        };
        let mut fixture = Fixture::new("transcription");
        let error = fixture
            .run(&events, &deps, &request_call, entry)
            .await
            .expect_err("a missing config fails");
        assert!(matches!(
            error,
            TranscriptionHttpError::MissingProviderConfig { .. }
        ));
        assert_eq!(
            error.to_string(),
            "No provider config found for model: m and provider: openai"
        );
        assert_no_http(&source, &wire, &events);
    }
}

fn assert_no_http(source: &Source, wire: &Wire, events: &Events) {
    assert!(source.calls().is_empty());
    assert!(wire.requests().is_empty());
    assert!(!events.recorded().iter().any(|event| event == "send"));
}

async fn run_switching_atranscription(atranscription: Json) -> Vec<ClientCall> {
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, r#"{"text":"hi"}"#), &events);
    let source = Source::new(wire);
    let config = RecordingConfig::new(
        record(AudioTranscriptionData::Object(Object::new())),
        &events,
    );
    let optional = object("{}");
    let litellm = object(r#"{"ssl_verify":false}"#);
    let session = SharedSession;
    let mut request_call = call(&optional, &litellm);
    request_call.atranscription = atranscription;
    request_call.shared_session = Some(&session);
    let mut fixture = Fixture::new("transcription");
    fixture
        .run(
            &events,
            &deps(&config, &source),
            &request_call,
            Entry::Audio,
        )
        .await
        .expect("the fixture call succeeds");
    source.calls()
}

async fn assert_boolean_atranscription_selects_async() {
    let calls = run_switching_atranscription(Json::Bool(true)).await;
    assert_eq!(
        calls,
        vec![ClientCall {
            ssl_verify: Some(SslVerify::Enabled(false)),
            shared_session: true,
        }]
    );
}

async fn assert_integer_atranscription_stays_sync() {
    let calls = run_switching_atranscription(Json::Number(Number::from(1_i32))).await;
    assert_eq!(
        calls,
        vec![ClientCall {
            ssl_verify: None,
            shared_session: false,
        }]
    );
}

async fn assert_max_retries_sends_once() {
    let events = Events::new();
    let wire = Wire::replying(Reply::json(429, "slow down"), &events);
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new(
        record(AudioTranscriptionData::Object(Object::new())),
        &events,
    );
    let optional = object("{}");
    let litellm = object("{}");
    let mut request_call = call(&optional, &litellm);
    request_call.max_retries = 5;
    let mut fixture = Fixture::new("transcription");
    let error = fixture
        .run(
            &events,
            &deps(&config, &source),
            &request_call,
            Entry::Audio,
        )
        .await
        .expect_err("the status fails the call");
    assert_eq!(wire.requests().len(), 1);
    assert_eq!(source.calls().len(), 1);
    let TranscriptionHttpError::Provider(provider) = error else {
        panic!("the status error is a provider error");
    };
    assert_eq!(provider.status_code, 429);
    assert_eq!(provider.message, "slow down");
}

async fn assert_status_failure_keeps_status_body_and_headers() {
    let events = Events::new();
    let reply = Reply::json(429, "slow down")
        .with_header("content-type", "text/plain")
        .with_header("retry-after", "3");
    let wire = Wire::replying(reply, &events);
    let source = Source::new(wire.clone());
    let data = record(AudioTranscriptionData::Object(Object::new()));
    let config = RecordingConfig::new(data, &events);
    let optional = object("{}");
    let litellm = object("{}");
    let request_call = call(&optional, &litellm);
    let mut fixture = Fixture::new("transcription");
    let dependencies = deps(&config, &source);
    let error = fixture
        .run(&events, &dependencies, &request_call, Entry::Audio)
        .await
        .expect_err("the status fails the call");
    let TranscriptionHttpError::Provider(provider) = error else {
        panic!("the status error is a provider error");
    };
    assert_eq!(provider.status_code, 429);
    assert_eq!(provider.message, "slow down");
    assert!(!provider.status_code_is_synthesized);
    let headers = &provider.headers;
    assert_eq!(headers.get("content-type"), Some(&json(r#""text/plain""#)));
    assert_eq!(headers.get("retry-after"), Some(&json(r#""3""#)));
    assert_eq!(headers.get("content-length"), Some(&json(r#""9""#)));
    assert_eq!(headers.get("content-encoding"), None);
}

async fn assert_status_failure_with_an_empty_body_has_no_length() {
    let events = Events::new();
    let wire = Wire::replying(Reply::json(429, ""), &events);
    let source = Source::new(wire);
    let config = RecordingConfig::new(
        record(AudioTranscriptionData::Object(Object::new())),
        &events,
    );
    let optional = object("{}");
    let litellm = object("{}");
    let request_call = call(&optional, &litellm);
    let mut fixture = Fixture::new("transcription");
    let error = fixture
        .run(
            &events,
            &deps(&config, &source),
            &request_call,
            Entry::Audio,
        )
        .await
        .expect_err("the status fails the call");
    let TranscriptionHttpError::Provider(provider) = error else {
        panic!("the status error is a provider error");
    };
    assert_eq!(provider.status_code, 429);
    assert_eq!(provider.message, "");
    assert_eq!(provider.headers.get("content-length"), None);
}

async fn failing_call(
    error: TransportError,
    timeout: Option<Duration>,
    entry: Entry,
) -> TranscriptionHttpError {
    let events = Events::new();
    let wire = Wire::failing(error, &events);
    let source = Source::new(wire);
    let config = RecordingConfig::new(
        record(AudioTranscriptionData::Object(Object::new())),
        &events,
    );
    let optional = object("{}");
    let litellm = object("{}");
    let mut request_call = call(&optional, &litellm);
    request_call.timeout = timeout;
    let mut fixture = Fixture::new("transcription");
    fixture
        .run(&events, &deps(&config, &source), &request_call, entry)
        .await
        .expect_err("the send fails")
}

async fn assert_timeouts_report_the_timeout() {
    let sync = failing_call(
        TransportError::Rejected(HttpError::Timeout),
        Some(Duration::from_secs(30)),
        Entry::Audio,
    )
    .await;
    let TranscriptionHttpError::Provider(sync_timeout) = sync else {
        panic!("a timeout is converted");
    };
    assert_eq!(sync_timeout.status_code, 408);
    assert!(!sync_timeout.status_code_is_synthesized);
    assert_eq!(
        sync_timeout.message,
        "litellm.Timeout: Connection timed out after 30 seconds."
    );
    let asynchronous = failing_call(
        TransportError::Rejected(HttpError::Timeout),
        Some(Duration::from_millis(1500)),
        Entry::Async,
    )
    .await;
    let TranscriptionHttpError::Provider(async_timeout) = asynchronous else {
        panic!("a timeout is converted");
    };
    assert_eq!(async_timeout.status_code, 408);
    assert!(
        async_timeout
            .message
            .starts_with("litellm.Timeout: Connection timed out. Timeout passed=1.5, time taken=")
    );
}

async fn assert_whole_second_timeouts_print_exactly() {
    let error = failing_call(
        TransportError::Rejected(HttpError::Timeout),
        Some(Duration::new(u64::MAX, 0)),
        Entry::Audio,
    )
    .await;
    let TranscriptionHttpError::Provider(provider) = error else {
        panic!("a timeout is converted");
    };
    assert_eq!(
        provider.message,
        "litellm.Timeout: Connection timed out after 18446744073709551615 seconds."
    );
}
