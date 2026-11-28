#![cfg(test)]

mod pool;
mod streaming;

use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use rstest::rstest;
use secrecy::SecretString;
use tempfile::NamedTempFile;
use tracing::subscriber::set_default;
use uuid::Uuid;

use crate::gemini_http::call::{async_completion, async_streaming};
use crate::gemini_http::error::GeminiHttpError;
use crate::gemini_http::transform::{TransformRequestUnsupported, transform_request};
use crate::model_policy::capabilities::VertexCapabilityProvider;
use crate::vertex_embedding_call::post::{ClientTimeout, Timeout};
use betterlite_core::api::vector_store_http_write::service::ClientKind;

use super::support::{
    ANSWER, BODY, GEMINI_KEY, MODEL, Recorder, Reply, SCHEMA_ERROR, TOKEN, auth, catalog,
    complete_reply, error_reply, llm, object, pooled, supplied, vertex_call,
};

const VERTEX_URL: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/project-p/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent";

fn fixed_id() -> Uuid {
    Uuid::nil()
}

#[tokio::test]
async fn execution_posts_the_transformed_request_once() {
    let body = object(BODY);
    let optional = object("{}");
    let call = vertex_call(&body, &optional);
    let fixture = auth();
    let recorder = Arc::new(Recorder::scripted(
        vec![complete_reply(200, ANSWER)],
        Vec::new(),
    ));
    let clients = pooled(&recorder);
    let response = async_completion(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(Uuid::new_v4),
    )
    .await
    .unwrap();
    assert_eq!(response.model, MODEL);
    assert_eq!(response.usage.total_tokens, 5);
    assert_eq!(fixture.executor.loads.load(Ordering::SeqCst), 1);
    let posts = recorder.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].url, VERTEX_URL);
    assert_eq!(posts[0].body.as_slice(), BODY.as_bytes());
    let expected = BTreeMap::from([
        (String::from("Authorization"), format!("Bearer {TOKEN}")),
        (
            String::from("Content-Type"),
            String::from("application/json"),
        ),
    ]);
    assert_eq!(posts[0].headers, expected);
    assert_eq!(posts[0].timeout, ClientTimeout::HandlerDefault);
}

#[tokio::test]
async fn execution_gemini_completion_reads_no_credentials() {
    let body = object(BODY);
    let optional = object("{}");
    let key = SecretString::from(GEMINI_KEY.to_owned());
    let mut call = vertex_call(&body, &optional);
    call.provider = VertexCapabilityProvider::Gemini;
    call.gemini_api_key = Some(&key);
    let fixture = auth();
    let recorder = Arc::new(Recorder::scripted(
        vec![complete_reply(200, ANSWER)],
        Vec::new(),
    ));
    let clients = pooled(&recorder);
    let response = async_completion(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap();
    assert_eq!(response.id, "chatcmpl-00000000-0000-0000-0000-000000000000");
    assert_eq!(fixture.executor.loads.load(Ordering::SeqCst), 0);
    let posts = recorder.posts();
    assert_eq!(
        posts[0].url,
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent"
    );
    assert_eq!(
        posts[0].headers.get("x-goog-api-key").map(String::as_str),
        Some(GEMINI_KEY)
    );
    assert_eq!(posts[0].headers.get("Authorization"), None);
}

#[rstest]
#[case::bad_request(400)]
#[case::unauthorized(401)]
#[case::rate_limited(429)]
#[case::server_error(500)]
#[case::redirect_without_a_location(304)]
#[tokio::test]
async fn execution_complete_preserves_status_and_headers(#[case] status: u16) {
    let body = object(BODY);
    let optional = object("{}");
    let call = vertex_call(&body, &optional);
    let fixture = auth();
    let recorder = Arc::new(Recorder::scripted(
        vec![error_reply(
            status,
            &[("x-request-id", "req-7")],
            r#"{"error":"upstream"}"#,
        )],
        Vec::new(),
    ));
    let clients = pooled(&recorder);
    let error = async_completion(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap_err();
    let provider = error.provider().expect("a provider rejection");
    assert_eq!(provider.status_code, status);
    assert_eq!(provider.message, r#"{"error":"upstream"}"#);
    assert_eq!(
        provider.headers,
        Some(BTreeMap::from([(
            String::from("x-request-id"),
            String::from("req-7")
        )]))
    );
}

#[tokio::test]
async fn execution_complete_keeps_the_raw_function_schema_error() {
    let body = object(BODY);
    let optional = object("{}");
    let call = vertex_call(&body, &optional);
    let fixture = auth();
    let recorder = Arc::new(Recorder::scripted(
        vec![complete_reply(400, SCHEMA_ERROR)],
        Vec::new(),
    ));
    let clients = pooled(&recorder);
    let error = async_completion(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap_err();
    let provider = error.provider().expect("a provider rejection");
    assert_eq!(provider.status_code, 400);
    assert_eq!(provider.message, SCHEMA_ERROR);
}

#[tokio::test]
async fn execution_complete_timeout_is_408() {
    let body = object(BODY);
    let optional = object("{}");
    let call = vertex_call(&body, &optional);
    let fixture = auth();
    let recorder = Arc::new(Recorder::scripted(vec![Reply::Timeout], Vec::new()));
    let clients = pooled(&recorder);
    let error = async_completion(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, GeminiHttpError::Timeout(_)));
    let provider = error.provider().expect("a timeout is a provider error");
    assert_eq!(provider.status_code, 408);
    assert_eq!(provider.message, "Timeout error occurred.");
    assert_eq!(provider.headers, None);
}

#[tokio::test]
async fn execution_complete_transport_failure_is_not_a_provider_error() {
    let body = object(BODY);
    let optional = object("{}");
    let call = vertex_call(&body, &optional);
    let fixture = auth();
    let recorder = Arc::new(Recorder::scripted(vec![Reply::Failed], Vec::new()));
    let clients = pooled(&recorder);
    let error = async_completion(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, GeminiHttpError::Transport { .. }));
    assert!(error.provider().is_none());
}

#[tokio::test]
async fn execution_complete_uses_the_supplied_async_client() {
    let body = object(BODY);
    let optional = object("{}");
    let mut call = vertex_call(&body, &optional);
    call.timeout = Some(Duration::from_secs(30));
    let fixture = auth();
    let caller = Arc::new(Recorder::scripted(
        vec![complete_reply(200, ANSWER)],
        Vec::new(),
    ));
    let pooled_recorder = Arc::new(Recorder::scripted(Vec::new(), Vec::new()));
    let clients = supplied(&caller, ClientKind::Asynchronous, &pooled_recorder);
    async_completion(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap();
    let posts = caller.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].timeout, ClientTimeout::HandlerDefault);
    assert!(pooled_recorder.posts().is_empty());
}

#[tokio::test]
async fn execution_complete_ignores_a_supplied_sync_client() {
    let body = object(BODY);
    let optional = object("{}");
    let mut call = vertex_call(&body, &optional);
    call.timeout = Some(Duration::from_secs(30));
    let fixture = auth();
    let caller = Arc::new(Recorder::scripted(Vec::new(), Vec::new()));
    let pooled_recorder = Arc::new(Recorder::scripted(
        vec![complete_reply(200, ANSWER)],
        Vec::new(),
    ));
    let clients = supplied(&caller, ClientKind::Sync, &pooled_recorder);
    async_completion(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap();
    let posts = pooled_recorder.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(
        posts[0].timeout,
        ClientTimeout::Explicit(Timeout {
            total: Duration::from_secs(30),
            connect: Duration::from_secs(30),
        })
    );
    assert!(caller.posts().is_empty());
}

#[tokio::test]
async fn execution_complete_the_caller_timeout_reaches_the_pooled_client() {
    let body = object(BODY);
    let optional = object("{}");
    let mut call = vertex_call(&body, &optional);
    call.timeout = Some(Duration::from_secs(30));
    let fixture = auth();
    let recorder = Arc::new(Recorder::scripted(
        vec![complete_reply(200, ANSWER)],
        Vec::new(),
    ));
    let clients = pooled(&recorder);
    async_completion(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap();
    assert_eq!(
        recorder.posts()[0].timeout,
        ClientTimeout::Explicit(Timeout {
            total: Duration::from_secs(30),
            connect: Duration::from_secs(30),
        })
    );
}

#[rstest]
#[case::unset(None)]
#[case::zero(Some(Duration::ZERO))]
#[tokio::test]
async fn execution_complete_a_falsy_timeout_keeps_the_handler_default(
    #[case] timeout: Option<Duration>,
) {
    let body = object(BODY);
    let optional = object("{}");
    let mut call = vertex_call(&body, &optional);
    call.timeout = timeout;
    let fixture = auth();
    let recorder = Arc::new(Recorder::scripted(
        vec![complete_reply(200, ANSWER)],
        Vec::new(),
    ));
    let clients = pooled(&recorder);
    async_completion(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap();
    assert_eq!(recorder.posts()[0].timeout, ClientTimeout::HandlerDefault);
}

#[tokio::test]
async fn execution_psc_flag_reaches_the_endpoint() {
    let body = object(BODY);
    let optional = object(r#"{"use_psc_endpoint_format":true}"#);
    let mut call = vertex_call(&body, &optional);
    call.api_base = Some("https://psc.example.test/");
    let fixture = auth();
    let recorder = Arc::new(Recorder::scripted(
        vec![complete_reply(200, ANSWER)],
        Vec::new(),
    ));
    let clients = pooled(&recorder);
    async_completion(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap();
    assert_eq!(
        recorder.posts()[0].url,
        "https://psc.example.test/v1/projects/project-p/locations/us-central1/endpoints/gemini-2.5-flash:generateContent"
    );
}

#[tokio::test]
async fn execution_a_wrong_psc_flag_fails_before_any_request() {
    let body = object(BODY);
    let optional = object(r#"{"use_psc_endpoint_format":"yes"}"#);
    let call = vertex_call(&body, &optional);
    let fixture = auth();
    let recorder = Arc::new(Recorder::scripted(Vec::new(), Vec::new()));
    let clients = pooled(&recorder);
    let error = async_completion(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        GeminiHttpError::Flag {
            field: "use_psc_endpoint_format"
        }
    ));
    let stream = async_streaming(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await;
    let Err(stream_error) = stream else {
        panic!("a wrong psc flag fails before the stream is built");
    };
    assert!(matches!(stream_error, GeminiHttpError::Flag { .. }));
    assert!(recorder.posts().is_empty());
    assert!(recorder.opens().is_empty());
}

#[test]
fn execution_transform_request_stays_unsupported() {
    let error = transform_request().unwrap_err();
    assert!(matches!(error, TransformRequestUnsupported));
    assert_eq!(
        error.to_string(),
        "Vertex AI has a custom implementation of transform_request. Needs sync + async."
    );
}

#[tokio::test]
async fn execution_logs_no_credentials() {
    let body = object(BODY);
    let optional = object("{}");
    let mut call = vertex_call(&body, &optional);
    call.api_base = Some("https://user:super-secret@private.example.com/");
    let fixture = auth();
    let recorder = Arc::new(Recorder::scripted(
        vec![complete_reply(200, ANSWER)],
        Vec::new(),
    ));
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let clients = pooled(&recorder);
    let guard = set_default(subscriber);
    async_completion(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap();
    drop(guard);
    let logged = fs::read_to_string(log.path()).unwrap();
    assert!(logged.contains("sending the Gemini request"));
    assert!(logged.contains("received the Gemini answer"));
    assert!(logged.contains(MODEL));
    assert!(logged.contains("private.example.com"));
    assert!(!logged.contains(TOKEN));
    assert!(!logged.contains("super-secret"));
}
