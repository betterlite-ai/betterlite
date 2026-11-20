use betterlite_core::api::vector_store_http_write::service::{ClientKind, SuppliedClient};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::api::transcription_contract::AudioTranscriptionData;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::transcription::client::SharedSession;

use super::support::{
    ClientCall, Entry, Events, Fixture, RecordingConfig, Reply, Source, StageFailure, Wire, call,
    deps, object, record,
};
use crate::api::transcription_contract::TranscriptionContractError;
use crate::api::transcription_http::TranscriptionDeps;
use crate::api::transcription_http::config::ConfigFailure;
use crate::api::transcription_http::error::TranscriptionHttpError;
use crate::api::transcription_http::reply::TranscriptionReply;

#[tokio::test]
async fn transcription_http_client_policy() {
    assert_only_a_matching_supplied_client_is_reused().await;
    assert_fallback_client_reads_verification().await;
    assert_prepare_failures_bypass_the_error_converter().await;
    assert_logging_failure_bypasses_the_error_converter().await;
    assert_response_transform_failure_bypasses_the_error_converter().await;
    assert_send_failure_stops_transformation().await;
}

struct Harness {
    events: Events,
    config: RecordingConfig,
    source: Source,
    source_wire: Wire,
}

impl Harness {
    fn new() -> Self {
        let events = Events::new();
        let source_wire = Wire::replying(Reply::json(200, r#"{"text":"hi"}"#), &events);
        let source = Source::new(source_wire.clone());
        let config = RecordingConfig::new(
            record(AudioTranscriptionData::Object(Object::new())),
            &events,
        );
        Self {
            events,
            config,
            source,
            source_wire,
        }
    }
}

async fn run(
    harness: &Harness,
    fixture: &mut Fixture,
    reused: Option<(&Wire, ClientKind)>,
    entry: Entry,
) -> Result<TranscriptionReply, TranscriptionHttpError> {
    let optional = object("{}");
    let litellm = object(r#"{"ssl_verify":false}"#);
    let session = SharedSession;
    let mut request_call = call(&optional, &litellm);
    request_call.shared_session = Some(&session);
    let deps: TranscriptionDeps<'_, RecordingConfig, Source> = match reused {
        Some((transport, kind)) => TranscriptionDeps {
            supplied: Some(SuppliedClient { kind, transport }),
            ..deps(&harness.config, &harness.source)
        },
        None => deps(&harness.config, &harness.source),
    };
    fixture
        .run(&harness.events, &deps, &request_call, entry)
        .await
}

async fn assert_only_a_matching_supplied_client_is_reused() {
    let harness = Harness::new();
    let matching = Wire::replying(Reply::json(200, r#"{"text":"hi"}"#), &harness.events);
    let mut fixture = Fixture::new("transcription");
    run(
        &harness,
        &mut fixture,
        Some((&matching, ClientKind::Sync)),
        Entry::Audio,
    )
    .await
    .expect("the supplied client sends");
    assert_eq!(matching.requests().len(), 1);
    assert!(harness.source_wire.requests().is_empty());
    assert!(harness.source.calls().is_empty());
    let wrong_kind = Wire::replying(Reply::json(200, r#"{"text":"hi"}"#), &harness.events);
    let mut fixture = Fixture::new("transcription");
    run(
        &harness,
        &mut fixture,
        Some((&wrong_kind, ClientKind::Asynchronous)),
        Entry::Audio,
    )
    .await
    .expect("the fallback sends");
    assert!(wrong_kind.requests().is_empty());
    assert_eq!(harness.source_wire.requests().len(), 1);
    assert_eq!(
        harness.source.calls(),
        vec![ClientCall {
            ssl_verify: None,
            shared_session: false,
        }]
    );
}

async fn fallback_calls(entry: Entry) -> Vec<ClientCall> {
    let harness = Harness::new();
    let mut fixture = Fixture::new("transcription");
    run(&harness, &mut fixture, None, entry)
        .await
        .expect("the fallback sends");
    harness.source.calls()
}

async fn assert_fallback_client_reads_verification() {
    assert_eq!(
        fallback_calls(Entry::Async).await,
        vec![ClientCall {
            ssl_verify: Some(false),
            shared_session: true,
        }]
    );
    assert_eq!(
        fallback_calls(Entry::Audio).await,
        vec![ClientCall {
            ssl_verify: None,
            shared_session: false,
        }]
    );
}

async fn prepare_failure(failure: StageFailure) -> (TranscriptionHttpError, Harness) {
    let mut harness = Harness::new();
    harness.config.failure = failure;
    let mut fixture = Fixture::new("transcription");
    let error = run(&harness, &mut fixture, None, Entry::Audio)
        .await
        .expect_err("the prepare stage fails");
    (error, harness)
}

async fn assert_prepare_failures_bypass_the_error_converter() {
    let (validate, validate_harness) = prepare_failure(StageFailure::Validate).await;
    assert_eq!(validate.to_string(), "the fixture config refused the call");
    assert!(matches!(
        validate,
        TranscriptionHttpError::Config(ConfigFailure { .. })
    ));
    assert!(
        !validate_harness
            .events
            .recorded()
            .iter()
            .any(|event| event == "send")
    );
    let (url, url_harness) = prepare_failure(StageFailure::Url).await;
    assert_eq!(url.to_string(), "the fixture config has no URL");
    assert!(url_harness.source_wire.requests().is_empty());
    let (transform, transform_harness) = prepare_failure(StageFailure::Transform).await;
    assert!(matches!(
        transform,
        TranscriptionHttpError::Contract(TranscriptionContractError::MissingRequestTransform)
    ));
    assert!(transform_harness.source_wire.requests().is_empty());
}

async fn assert_logging_failure_bypasses_the_error_converter() {
    let harness = Harness::new();
    let mut fixture = Fixture::new("transcription");
    fixture.state.request_logging.raw_request_only = true;
    let error = run(&harness, &mut fixture, None, Entry::Audio)
        .await
        .expect_err("the logging hook fails");
    assert!(matches!(error, TranscriptionHttpError::Logging(_)));
    assert!(harness.source.calls().is_empty());
    assert!(harness.source_wire.requests().is_empty());
}

async fn assert_response_transform_failure_bypasses_the_error_converter() {
    let mut harness = Harness::new();
    harness.config.failure = StageFailure::Response;
    let mut fixture = Fixture::new("transcription");
    let error = run(&harness, &mut fixture, None, Entry::Audio)
        .await
        .expect_err("the response transform fails");
    assert!(matches!(
        error,
        TranscriptionHttpError::Contract(TranscriptionContractError::UnexpectedResponseTransform)
    ));
    assert_eq!(harness.source_wire.requests().len(), 1);
    assert_eq!(harness.config.calls().responses, 1);
    assert!(
        harness
            .events
            .recorded()
            .iter()
            .any(|event| event == "send")
    );
}

async fn assert_send_failure_stops_transformation() {
    let events = Events::new();
    let wire = Wire::failing(TransportError::Closed, &events);
    let source = Source::new(wire.clone());
    let mut config = RecordingConfig::new(
        record(AudioTranscriptionData::Object(Object::new())),
        &events,
    );
    config.subtitles = true;
    config.response = serde_json::from_str(r#"{"text":"old","words":[]}"#)
        .expect("the response fixture is a transcription response");
    let optional = object(r#"{"response_format":"srt"}"#);
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
        .expect_err("the send fails");
    let TranscriptionHttpError::Provider(error) = error else {
        panic!("a send failure is converted");
    };
    assert_eq!(error.status_code, 500);
    assert!(error.status_code_is_synthesized);
    assert_eq!(
        error.message,
        "the client is shutting down and takes no new requests"
    );
    assert!(error.headers.is_empty());
    assert_eq!(config.calls().responses, 0);
    assert!(
        !events
            .recorded()
            .iter()
            .any(|event| event == "transform-response")
    );
    assert_eq!(wire.requests().len(), 1);
}
