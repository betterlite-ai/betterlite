use std::str::FromStr as _;
use std::time::Duration;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde_json::Number;

use super::fixture::{Fixture, Recorded, Recorder, Reply, connection, header, sent_body};
use crate::speech::call::{SpeechCall, audio_speech};
use crate::speech::client::{
    ClientKind, SharedSession, SpeechConnection, SuppliedClient, normalize_base, select_client,
};
use crate::speech::error::SpeechError;
use crate::speech::logging::PrePostLogger;
use crate::speech::request::{SpeechFormat, SpeechOptions, SpeechRequest};
use crate::speech::result::SpeechResponse;

#[derive(Debug, Clone, Copy)]
enum Case {
    VoiceAndFormat,
    SpeedAndInstructions,
    BinaryBody,
    ChunkedBinaryBody,
    SpeechError,
    SpeechClientOptions,
    SpeechLoggingProjection,
}

#[rstest]
#[case::case_1_voice_and_format(Case::VoiceAndFormat)]
#[case::case_2_speed_and_instructions(Case::SpeedAndInstructions)]
#[case::case_3_binary_body(Case::BinaryBody)]
#[case::case_4_chunked_binary_body(Case::ChunkedBinaryBody)]
#[case::case_5_speech_error(Case::SpeechError)]
#[case::case_6_speech_client_options(Case::SpeechClientOptions)]
#[case::case_7_speech_logging_projection(Case::SpeechLoggingProjection)]
#[tokio::test]
async fn speech_cases(#[case] case: Case) {
    match case {
        Case::VoiceAndFormat => voice_and_format().await,
        Case::SpeedAndInstructions => speed_and_instructions().await,
        Case::BinaryBody => binary_body().await,
        Case::ChunkedBinaryBody => chunked_binary_body().await,
        Case::SpeechError => speech_error().await,
        Case::SpeechClientOptions => speech_client_options().await,
        Case::SpeechLoggingProjection => speech_logging_projection().await,
    }
}

async fn call(
    fixture: &Fixture,
    options: &SpeechOptions,
    connection: SpeechConnection,
    aspeech: Option<bool>,
    input: &str,
) -> (Result<SpeechResponse, SpeechError>, Recorder) {
    let mut recorder = Recorder::default();
    let prepared = audio_speech(
        SpeechCall {
            request: SpeechRequest {
                model: "tts-1",
                input,
                voice: "alloy",
                options,
            },
            connection,
            aspeech,
        },
        &mut recorder,
        fixture,
    );
    let outcome = match prepared {
        Ok(pending) => pending.await,
        Err(error) => Err(error),
    };
    (outcome, recorder)
}

fn audio_ok(chunks: Vec<Vec<u8>>) -> Fixture {
    Fixture::new(Reply::ok("audio/mpeg", chunks))
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_slice(text.as_bytes()).expect("the expected body is JSON")
}

fn assert_wire_request(fixture: &Fixture, deadline: Duration) {
    assert_eq!(fixture.request().method, http::Method::POST);
    assert_eq!(
        fixture.request().url.as_str(),
        "https://speech.test/v1/audio/speech"
    );
    assert_eq!(
        header(fixture, "accept").as_deref(),
        Some("application/octet-stream")
    );
    assert_eq!(
        header(fixture, "authorization").as_deref(),
        Some("Bearer sk-speech")
    );
    assert_eq!(
        header(fixture, "content-type").as_deref(),
        Some("application/json")
    );
    assert_eq!(fixture.request().deadline, deadline);
}

async fn voice_and_format() {
    let fixture = audio_ok(vec![vec![0xFF, 0xFB, 0x10]]);
    let options = SpeechOptions {
        response_format: Some(SpeechFormat::Wav),
        speed: None,
        instructions: None,
        extra_headers: IndexMap::from([(String::from("x-trace"), String::from("trace-1"))]),
    };
    let (first, recorder) = call(&fixture, &options, connection(), None, "Hello speech").await;
    let response = first.expect("the 200 reply is a success");
    assert_eq!(response.content_type.as_deref(), Some("audio/mpeg"));
    assert_eq!(
        response.content(1024).await.expect("the body reads"),
        vec![0xFF, 0xFB, 0x10]
    );
    assert_wire_request(&fixture, Duration::from_secs(30));
    assert_eq!(header(&fixture, "x-trace").as_deref(), Some("trace-1"));
    assert_eq!(
        sent_body(&fixture),
        json(r#"{"input":"Hello speech","model":"tts-1","voice":"alloy","response_format":"wav"}"#)
    );
    assert_eq!(
        recorder.records[0].body.get("response_format"),
        Some(&Json::Text(String::from("wav")))
    );

    // an omitted response_format stays omitted: the SDK default is not written by this wrapper
    let plain = audio_ok(vec![vec![1]]);
    let (second, _) = call(
        &plain,
        &SpeechOptions::default(),
        connection(),
        Some(false),
        "Hi",
    )
    .await;
    second.expect("the 200 reply is a success");
    assert_eq!(
        sent_body(&plain),
        json(r#"{"input":"Hi","model":"tts-1","voice":"alloy"}"#)
    );
}

async fn speed_and_instructions() {
    let fixture = audio_ok(vec![vec![1]]);
    let options = SpeechOptions {
        response_format: None,
        speed: Some(Decimal::from_str("1.234567890123456789").expect("the speed is a decimal")),
        instructions: Some(String::from("calm and clear")),
        extra_headers: IndexMap::new(),
    };
    let (first, recorder) = call(&fixture, &options, connection(), None, "Hi").await;
    first.expect("the 200 reply is a success");
    let body = sent_body(&fixture);
    assert_eq!(body["speed"].to_string(), "1.234567890123456789");
    assert_eq!(
        body["instructions"],
        serde_json::Value::String(String::from("calm and clear"))
    );
    assert_eq!(
        recorder.records[0].body.get("speed"),
        Some(&Json::Number(
            Number::from_str("1.234567890123456789").expect("the speed is a JSON number")
        ))
    );
    assert_eq!(
        recorder.records[0].body.get("instructions"),
        Some(&Json::Text(String::from("calm and clear")))
    );
}

async fn binary_body() {
    let payload = b"{\"not\":\"json\"}\n\x00\xff".to_vec();
    let fixture = Fixture::new(Reply::ok("audio/ogg", vec![payload.clone()]));
    let (first, _) = call(
        &fixture,
        &SpeechOptions::default(),
        connection(),
        None,
        "Hi",
    )
    .await;
    let response = first.expect("the 200 reply is a success");
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type.as_deref(), Some("audio/ogg"));
    assert_eq!(
        response.headers.get("x-request-id").map(String::as_str),
        Some("req-1")
    );
    assert_eq!(
        response.additional_headers.get("llm_provider-content-type"),
        Some(&Json::Text(String::from("audio/ogg")))
    );
    assert_eq!(
        response.additional_headers.get("llm_provider-x-request-id"),
        Some(&Json::Text(String::from("req-1")))
    );
    assert!(!response.additional_headers.contains_key("content-type"));
    assert_eq!(
        response.content(1024).await.expect("the body reads"),
        payload
    );
}

async fn chunked_binary_body() {
    let chunks = vec![vec![1, 2], vec![3], vec![4, 5, 6]];
    let fixture = Fixture::new(Reply::ok("audio/wav", chunks.clone()));
    let (first, _) = call(
        &fixture,
        &SpeechOptions::default(),
        connection(),
        None,
        "Hi",
    )
    .await;
    let whole = first.expect("the 200 reply is a success");
    assert_eq!(
        whole.content(6).await.expect("the body reads"),
        vec![1, 2, 3, 4, 5, 6]
    );

    let over = Fixture::new(Reply::ok("audio/wav", chunks));
    let (second, _) = call(&over, &SpeechOptions::default(), connection(), None, "Hi").await;
    let limited = second.expect("the 200 reply is a success");
    assert!(matches!(
        limited.content(5).await,
        Err(SpeechError::TooLarge)
    ));
}

async fn speech_error() {
    let error_body = br#"{"error":{"message":"The voice does not exist","type":"invalid_request_error","param":"voice","code":"voice_not_found"}}"#;
    let fixture = Fixture::new(Reply {
        status: 404,
        headers: vec![(
            String::from("content-type"),
            String::from("application/json"),
        )],
        chunks: vec![error_body.to_vec()],
    });
    let (first, recorder) = call(
        &fixture,
        &SpeechOptions::default(),
        connection(),
        None,
        "Hi",
    )
    .await;
    let Err(SpeechError::Status(status)) = first else {
        panic!("the 404 is the SDK's status error");
    };
    assert_eq!(status.status, 404);
    assert_eq!(status.error_type.as_deref(), Some("invalid_request_error"));
    assert_eq!(status.param.as_deref(), Some("voice"));
    assert_eq!(status.code.as_deref(), Some("voice_not_found"));
    assert_eq!(fixture.sends(), 1);
    assert_eq!(recorder.records.len(), 1);

    // a transport failure reaches the caller untouched
    let broken = Fixture::broken();
    let (second, _) = call(&broken, &SpeechOptions::default(), connection(), None, "Hi").await;
    assert!(matches!(second, Err(SpeechError::Transport { .. })));
}

async fn speech_client_options() {
    assert_eq!(ClientKind::Sync.label(), "sync");
    assert_eq!(ClientKind::Async.label(), "async");
    assert_built_client_options();
    assert_supplied_client_options().await;
    assert_async_supplied_client().await;
}

fn assert_built_client_options() {
    let settings = connection();
    let sync = select_client(&settings, ClientKind::Sync).expect("the sync client resolves");
    assert_eq!(sync.kind, ClientKind::Sync);
    assert_eq!(sync.base_url.as_str(), "https://speech.test/v1/");
    assert_eq!(sync.timeout, Duration::from_secs(30));
    assert_eq!(sync.max_retries, 4);
    assert_eq!(sync.organization, None);
    assert!(sync.shared_session.is_none());
    let async_client =
        select_client(&settings, ClientKind::Async).expect("the async client resolves");
    assert_eq!(async_client.kind, ClientKind::Async);
    assert_eq!(async_client.organization, None);
    assert!(async_client.shared_session.is_none());

    let mut shared = connection();
    shared.shared_session = Some(SharedSession);
    let async_shared =
        select_client(&shared, ClientKind::Async).expect("the async client resolves");
    assert!(async_shared.shared_session.is_some());
    let sync_shared = select_client(&shared, ClientKind::Sync).expect("the sync client resolves");
    assert!(sync_shared.shared_session.is_none());
}

fn supplied_settings(kind: ClientKind) -> SpeechConnection {
    let mut settings = connection();
    settings.client = Some(SuppliedClient {
        kind,
        api_key: Some(SecretString::from("sk-supplied")),
        base_url: normalize_base("https://supplied.test/v1").expect("the supplied base is a URL"),
        timeout: Duration::from_secs(45),
        max_retries: 1,
        organization: Some(String::from("org-supplied")),
    });
    settings
}

async fn assert_supplied_client_options() {
    let settings = supplied_settings(ClientKind::Sync);
    let kept = select_client(&settings, ClientKind::Sync).expect("the supplied client wins");
    assert_eq!(kept.base_url.as_str(), "https://supplied.test/v1/");
    assert_eq!(kept.timeout, Duration::from_secs(45));
    assert_eq!(kept.max_retries, 4);
    assert_eq!(kept.organization.as_deref(), Some("org-supplied"));
    assert!(matches!(
        select_client(&settings, ClientKind::Async),
        Err(SpeechError::ClientKind { .. })
    ));

    let fixture = audio_ok(vec![vec![1]]);
    let (first, _) = call(&fixture, &SpeechOptions::default(), settings, None, "Hi").await;
    first.expect("the supplied client serves the call");
    assert_eq!(
        fixture.request().url.as_str(),
        "https://supplied.test/v1/audio/speech"
    );
    assert_eq!(fixture.request().deadline, Duration::from_secs(45));
    assert_eq!(
        header(&fixture, "authorization").as_deref(),
        Some("Bearer sk-supplied")
    );
}

async fn assert_async_supplied_client() {
    let fixture = audio_ok(vec![vec![1]]);
    let (first, _) = call(
        &fixture,
        &SpeechOptions::default(),
        supplied_settings(ClientKind::Async),
        Some(true),
        "Hi",
    )
    .await;
    first.expect("aspeech true picks the async client");
}

async fn speech_logging_projection() {
    let fixture = audio_ok(vec![vec![1]]);
    let options = SpeechOptions {
        response_format: None,
        speed: Some(Decimal::from_str("0.5").expect("the speed is a decimal")),
        instructions: None,
        extra_headers: IndexMap::from([(String::from("x-trace"), String::from("trace-1"))]),
    };
    let (first, mut recorder) = call(&fixture, &options, connection(), None, "Hello speech").await;
    first.expect("the 200 reply is a success");
    let record = recorder.records.remove(0);
    assert_logged_projection(&record);
    assert_wired_logging(&record, &options).await;
}

fn assert_logged_projection(record: &Recorded) {
    assert_eq!(record.input, "Hello speech");
    assert_eq!(record.api_base, "https://speech.test/v1/");
    assert_eq!(
        record.body.get("model"),
        Some(&Json::Text(String::from("tts-1")))
    );
    assert_eq!(
        record.body.get("voice"),
        Some(&Json::Text(String::from("alloy")))
    );
    assert_eq!(
        record.body.get("speed"),
        Some(&Json::Number(
            Number::from_str("0.5").expect("the speed is a JSON number")
        ))
    );
    assert!(!record.body.contains_key("extra_headers"));
    assert!(!record.body.contains_key("input"));
    assert!(
        !record
            .body
            .keys()
            .any(|name| name.contains("key") || name.contains("authorization"))
    );
}

async fn assert_wired_logging(record: &Recorded, options: &SpeechOptions) {
    let (additional, input) = wired_logging(options).await;
    let Json::Object(fields) = additional else {
        panic!("the additional args are an object");
    };
    assert_eq!(
        fields.get("complete_input_dict"),
        Some(&Json::Object(record.body.clone()))
    );
    assert_eq!(
        fields.get("api_base"),
        Some(&Json::Text(String::from("https://speech.test/v1/")))
    );
    assert_eq!(input, Json::Text(String::from("Hello speech")));
}

async fn wired_logging(options: &SpeechOptions) -> (Json, Json) {
    let mut state = super::logging::state();
    let redactor = super::logging::redactor();
    let sink = super::logging::Sink::default();
    let fixture = audio_ok(vec![vec![1]]);
    let prepared = {
        let env = super::logging::env(state.start_time, &redactor, &sink);
        audio_speech(
            SpeechCall {
                request: SpeechRequest {
                    model: "tts-1",
                    input: "Hello speech",
                    voice: "alloy",
                    options,
                },
                connection: connection(),
                aspeech: None,
            },
            &mut PrePostLogger {
                state: &mut state,
                env: &env,
            },
            &fixture,
        )
    };
    let outcome = prepared.expect("the logging hook accepts the call").await;
    outcome.expect("the 200 reply is a success");
    let additional = state
        .model_call_details
        .get("additional_args")
        .expect("pre_call stored the additional args")
        .clone();
    let input = state
        .model_call_details
        .get("input")
        .expect("pre_call stored the input")
        .clone();
    (additional, input)
}
