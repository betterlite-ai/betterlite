use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use super::cases::{SyncCall, run};
use super::fixture::{AudioFixture, ConnectionOptions, Fixture, Reply, authorization, connection};
use crate::transcription::client::{ClientKind, TranscriptionConnection, select_client};
use crate::transcription::error::TranscriptionError;
use crate::transcription::request::TranscriptionData;

const SECRET: &str = "sk-transcription-fixture-secret";

fn options() -> IndexMap<String, Json> {
    IndexMap::from([(String::from("language"), Json::Text(String::from("en")))])
}

fn connection_with(
    api_key: Option<&'static str>,
    environment: Option<&'static str>,
) -> TranscriptionConnection {
    connection(&ConnectionOptions {
        api_key,
        environment_api_key: environment,
        shared_session: false,
    })
}

fn secret_connection() -> TranscriptionConnection {
    connection_with(Some(SECRET), None)
}

#[tokio::test]
async fn openai_transcription_credential_identity_isolation() {
    assert_distinct_wire_credentials().await;
    assert_distinct_clients().await;
}

async fn assert_distinct_wire_credentials() {
    let audio = AudioFixture::new("a.wav", b"abc", "audio/wav");
    let first = Fixture::new(Reply::json(r#"{"text":"one"}"#));
    let second = Fixture::new(Reply::json(r#"{"text":"two"}"#));
    let first_run = run(
        &first,
        &audio,
        SyncCall::whisper(options(), connection_with(Some("sk-first"), None)),
    )
    .await;
    let second_run = run(
        &second,
        &audio,
        SyncCall::whisper(options(), connection_with(None, Some("sk-second"))),
    )
    .await;
    first_run.outcome.expect("the first reply is a success");
    second_run.outcome.expect("the second reply is a success");
    let first_auth = authorization(&first);
    let second_auth = authorization(&second);
    assert_eq!(first_auth.as_deref(), Some("Bearer sk-first"));
    assert_eq!(second_auth.as_deref(), Some("Bearer sk-second"));
    assert!(!second_auth.unwrap_or_default().contains("sk-first"));
}

async fn assert_distinct_clients() {
    let audio = AudioFixture::new("a.wav", b"abc", "audio/wav");
    let fixture = Fixture::new(Reply::json(r#"{"text":"one"}"#));
    // a second call builds its own client; nothing cached under one identity answers for the other
    let first = run(
        &fixture,
        &audio,
        SyncCall::whisper(IndexMap::new(), connection_with(Some("sk-first"), None)),
    )
    .await;
    let other = select_client(&connection_with(Some("sk-second"), None), ClientKind::Sync)
        .expect("the second client builds");
    assert_eq!(other.api_key.expose_secret(), "sk-second");
    assert_eq!(first.recorder.pre_calls.len(), 1);
    assert!(!format!("{:?}", first.outcome).contains("sk-first"));
}

#[tokio::test]
async fn openai_transcription_secret_safe_failure() {
    assert_debug_is_safe();
    assert_rejection_is_safe().await;
    assert_missing_key_is_safe();
    assert_async_is_safe().await;
}

fn assert_debug_is_safe() {
    let connection = secret_connection();
    assert!(!format!("{connection:?}").contains(SECRET));
    let client = select_client(&connection, ClientKind::Sync).expect("the client builds");
    assert!(!format!("{client:?}").contains(SECRET));
    assert_eq!(client.api_key.expose_secret(), SECRET);
}

fn rejected_reply() -> Reply {
    Reply {
        status: 401,
        headers: vec![(
            String::from("content-type"),
            String::from("application/json"),
        )],
        body: br#"{"error":{"message":"provider rejected the key","type":"invalid_request_error","param":"api_key","code":"invalid_api_key"}}"#.to_vec(),
    }
}

async fn assert_rejection_is_safe() {
    let audio = AudioFixture::new("a.wav", b"abc", "audio/wav");
    let fixture = Fixture::new(rejected_reply());
    let run = run(
        &fixture,
        &audio,
        SyncCall::whisper(IndexMap::new(), secret_connection()),
    )
    .await;
    let error = run.outcome.expect_err("the 401 is an error");
    assert!(!format!("{error:?}").contains(SECRET));
    assert!(!error.to_string().contains(SECRET));
}

fn assert_missing_key_is_safe() {
    let missing = select_client(&connection_with(None, None), ClientKind::Sync)
        .expect_err("a client needs a key");
    assert!(matches!(missing, TranscriptionError::MissingApiKey));
    assert!(!format!("{missing:?}").contains(SECRET));
    let bad_base = select_client(
        &TranscriptionConnection {
            api_base: Some(String::from("not a url")),
            ..secret_connection()
        },
        ClientKind::Sync,
    )
    .expect_err("a bad base is refused");
    assert!(!format!("{bad_base:?}").contains(SECRET));
}

async fn assert_async_is_safe() {
    let audio = AudioFixture::new("a.wav", b"abc", "audio/wav");
    let fixture = Fixture::new(Reply::json(r#"{"text":"hi"}"#));
    let data = TranscriptionData {
        model: Some(Json::Text(String::from("whisper-1"))),
        options: IndexMap::new(),
    };
    let run = run(
        &fixture,
        &audio,
        SyncCall::explicit(data, secret_connection()),
    )
    .await;
    assert!(!format!("{:?}", run.outcome).contains(SECRET));
    let expected = format!("Bearer {SECRET}");
    assert_eq!(authorization(&fixture).as_deref(), Some(expected.as_str()));
}
