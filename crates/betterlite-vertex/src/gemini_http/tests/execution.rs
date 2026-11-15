#![cfg(test)]

mod pool;

use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use betterlite_http::error::HttpError;
use rstest::rstest;
use secrecy::SecretString;
use tempfile::NamedTempFile;
use tracing::subscriber::set_default;
use uuid::Uuid;

use crate::gemini_http::call::{ClientKind, async_completion, async_streaming};
use crate::gemini_http::error::{GeminiHttpError, GeminiStreamError};
use crate::gemini_http::stream::{StreamCall, make_sync_call};
use crate::gemini_http::transform::{TransformRequestUnsupported, transform_request};
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::model_policy::capabilities::VertexCapabilityProvider;
use crate::vertex_embedding_call::post::{ClientTimeout, Timeout};

use super::support::{
    ANSWER, BODY, GEMINI_KEY, MODEL, NO_SECRET_HEADERS, OpenReply, Piece, Recorder, Reply,
    SCHEMA_ERROR, STREAM_CHUNK, TOKEN, auth, catalog, complete_reply, error_reply, line, llm,
    object, pooled, stream_reply, supplied, vertex_call,
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

#[tokio::test]
async fn execution_gemini_streaming_reads_no_credentials() {
    let body = object(BODY);
    let optional = object("{}");
    let key = SecretString::from(GEMINI_KEY.to_owned());
    let mut call = vertex_call(&body, &optional);
    call.provider = VertexCapabilityProvider::Gemini;
    call.gemini_api_key = Some(&key);
    let fixture = auth();
    let recorder = Arc::new(Recorder::scripted(
        Vec::new(),
        vec![stream_reply(200, vec![line(STREAM_CHUNK)])],
    ));
    let clients = pooled(&recorder);
    let mut stream = async_streaming(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap();
    stream.next_chunk().await.unwrap().unwrap();
    assert_eq!(fixture.executor.loads.load(Ordering::SeqCst), 0);
    let opens = recorder.opens();
    assert_eq!(
        opens[0].url,
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
    );
    assert_eq!(
        opens[0].headers.get("x-goog-api-key").map(String::as_str),
        Some(GEMINI_KEY)
    );
    assert_eq!(opens[0].headers.get("Authorization"), None);
}

#[rstest]
#[case::bad_request(400)]
#[case::unauthorized(401)]
#[case::rate_limited(429)]
#[case::server_error(500)]
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
    let clients = supplied(&caller, ClientKind::Async, &pooled_recorder);
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
    let error = async_streaming(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, GeminiHttpError::Flag { .. }));
    assert!(recorder.posts().is_empty());
    assert!(recorder.opens().is_empty());
}

#[tokio::test]
async fn execution_streaming_opens_lazily() {
    let body = object(BODY);
    let optional = object("{}");
    let call = vertex_call(&body, &optional);
    let fixture = auth();
    let recorder = Arc::new(Recorder::scripted(
        Vec::new(),
        vec![stream_reply(200, vec![line(STREAM_CHUNK)])],
    ));
    let clients = pooled(&recorder);
    let mut stream = async_streaming(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap();
    assert!(recorder.opens().is_empty());
    assert!(recorder.posts().is_empty());
    let chunk = stream.next_chunk().await.unwrap().unwrap();
    assert_eq!(chunk.id, "r1");
    assert_eq!(recorder.opens().len(), 1);
    assert!(recorder.posts().is_empty());
    assert!(
        recorder.opens()[0]
            .url
            .ends_with(":streamGenerateContent?alt=sse")
    );
    assert!(stream.next_chunk().await.is_none());
}

#[rstest]
#[case::ok(200)]
#[case::created(201)]
#[tokio::test]
async fn execution_streaming_accepts_200_and_201(#[case] status: u16) {
    let body = object(BODY);
    let optional = object("{}");
    let call = vertex_call(&body, &optional);
    let fixture = auth();
    let recorder = Arc::new(Recorder::scripted(
        Vec::new(),
        vec![stream_reply(status, vec![line(STREAM_CHUNK)])],
    ));
    let clients = pooled(&recorder);
    let mut stream = async_streaming(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap();
    let chunk = stream.next_chunk().await.unwrap().unwrap();
    assert_eq!(chunk.id, "r1");
}

#[tokio::test]
async fn execution_streaming_rejects_202_after_the_status_check() {
    let body = object(BODY);
    let optional = object("{}");
    let call = vertex_call(&body, &optional);
    let fixture = auth();
    let recorder = Arc::new(Recorder::scripted(
        Vec::new(),
        vec![stream_reply(
            202,
            vec![Piece::Bytes(br#"{"error":"accepted"}"#.to_vec())],
        )],
    ));
    let clients = pooled(&recorder);
    let mut stream = async_streaming(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap();
    let error = stream.next_chunk().await.unwrap().unwrap_err();
    let GeminiStreamError::Http(GeminiHttpError::Provider(provider)) = error else {
        panic!("a 202 is a provider rejection");
    };
    assert_eq!(provider.status_code, 202);
    assert_eq!(provider.message, r#"{"error":"accepted"}"#);
    assert!(stream.next_chunk().await.is_none());
}

#[tokio::test]
async fn execution_streaming_translates_the_function_schema_error() {
    let body = object(BODY);
    let optional = object("{}");
    let call = vertex_call(&body, &optional);
    let fixture = auth();
    let recorder = Arc::new(Recorder::scripted(
        Vec::new(),
        vec![stream_reply(
            400,
            vec![Piece::Bytes(SCHEMA_ERROR.as_bytes().to_vec())],
        )],
    ));
    let clients = pooled(&recorder);
    let mut stream = async_streaming(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap();
    let error = stream.next_chunk().await.unwrap().unwrap_err();
    let GeminiStreamError::Http(GeminiHttpError::Provider(provider)) = error else {
        panic!("a 400 is a provider rejection");
    };
    assert_eq!(provider.status_code, 400);
    assert_eq!(
        provider.message,
        format!(
            "'properties' field in tools[0]['function']['parameters'] cannot be empty if 'type' == 'object'. Received error from provider - {SCHEMA_ERROR}"
        )
    );
}

#[tokio::test]
async fn execution_streaming_keeps_a_body_it_cannot_translate() {
    let body = object(BODY);
    let optional = object("{}");
    let call = vertex_call(&body, &optional);
    let fixture = auth();
    let recorder = Arc::new(Recorder::scripted(
        Vec::new(),
        vec![stream_reply(
            400,
            vec![Piece::Bytes(br#"{"error":"bad tool"}"#.to_vec())],
        )],
    ));
    let clients = pooled(&recorder);
    let mut stream = async_streaming(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap();
    let error = stream.next_chunk().await.unwrap().unwrap_err();
    let GeminiStreamError::Http(GeminiHttpError::Provider(provider)) = error else {
        panic!("a 400 is a provider rejection");
    };
    assert_eq!(provider.message, r#"{"error":"bad tool"}"#);
}

#[tokio::test]
async fn execution_streaming_uses_the_supplied_async_client() {
    let body = object(BODY);
    let optional = object("{}");
    let call = vertex_call(&body, &optional);
    let fixture = auth();
    let caller = Arc::new(Recorder::scripted(
        Vec::new(),
        vec![stream_reply(200, vec![line(STREAM_CHUNK)])],
    ));
    let pooled_recorder = Arc::new(Recorder::scripted(Vec::new(), Vec::new()));
    let clients = supplied(&caller, ClientKind::Async, &pooled_recorder);
    let mut stream = async_streaming(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap();
    stream.next_chunk().await.unwrap().unwrap();
    assert_eq!(caller.opens().len(), 1);
    assert!(pooled_recorder.opens().is_empty());
}

#[tokio::test]
async fn execution_streaming_ignores_a_supplied_sync_client() {
    let body = object(BODY);
    let optional = object("{}");
    let call = vertex_call(&body, &optional);
    let fixture = auth();
    let caller = Arc::new(Recorder::scripted(Vec::new(), Vec::new()));
    let pooled_recorder = Arc::new(Recorder::scripted(
        Vec::new(),
        vec![stream_reply(200, vec![line(STREAM_CHUNK)])],
    ));
    let clients = supplied(&caller, ClientKind::Sync, &pooled_recorder);
    let mut stream = async_streaming(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap();
    stream.next_chunk().await.unwrap().unwrap();
    assert_eq!(pooled_recorder.opens().len(), 1);
    assert!(caller.opens().is_empty());
}

#[tokio::test]
async fn execution_streaming_never_forwards_the_timeout() {
    let body = object(BODY);
    let optional = object("{}");
    let mut call = vertex_call(&body, &optional);
    call.timeout = Some(Duration::from_secs(30));
    let fixture = auth();
    let recorder = Arc::new(Recorder::scripted(
        Vec::new(),
        vec![stream_reply(200, vec![line(STREAM_CHUNK)])],
    ));
    let clients = pooled(&recorder);
    let mut stream = async_streaming(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap();
    stream.next_chunk().await.unwrap().unwrap();
    assert_eq!(recorder.opens()[0].timeout, ClientTimeout::HandlerDefault);
}

#[tokio::test]
async fn execution_streaming_transport_failure_ends_the_stream() {
    let body = object(BODY);
    let optional = object("{}");
    let call = vertex_call(&body, &optional);
    let fixture = auth();
    let recorder = Arc::new(Recorder::scripted(Vec::new(), vec![OpenReply::Failed]));
    let clients = pooled(&recorder);
    let mut stream = async_streaming(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap();
    let error = stream.next_chunk().await.unwrap().unwrap_err();
    let GeminiStreamError::Http(error) = error else {
        panic!("a transport failure is an HTTP error");
    };
    assert!(matches!(error, GeminiHttpError::Transport { .. }));
    assert!(stream.next_chunk().await.is_none());
}

#[tokio::test]
async fn execution_streaming_open_timeout_is_408() {
    let body = object(BODY);
    let optional = object("{}");
    let call = vertex_call(&body, &optional);
    let fixture = auth();
    let recorder = Arc::new(Recorder::scripted(Vec::new(), vec![OpenReply::Timeout]));
    let clients = pooled(&recorder);
    let mut stream = async_streaming(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap();
    let error = stream.next_chunk().await.unwrap().unwrap_err();
    let GeminiStreamError::Http(error) = error else {
        panic!("an open timeout is an HTTP error");
    };
    let provider = error.provider().expect("a timeout is a provider error");
    assert_eq!(provider.status_code, 408);
    assert_eq!(provider.message, "Timeout error occurred.");
}

#[rstest]
#[case::timeout(HttpError::Timeout, true)]
#[case::too_large(HttpError::TooLarge { limit: 8 }, false)]
#[tokio::test]
async fn execution_streaming_reads_the_body_failure(
    #[case] failure: HttpError,
    #[case] timed_out: bool,
) {
    let body = object(BODY);
    let optional = object("{}");
    let call = vertex_call(&body, &optional);
    let fixture = auth();
    let recorder = Arc::new(Recorder::scripted(
        Vec::new(),
        vec![stream_reply(200, vec![Piece::Failed(failure)])],
    ));
    let clients = pooled(&recorder);
    let mut stream = async_streaming(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap();
    let error = stream.next_chunk().await.unwrap().unwrap_err();
    if timed_out {
        assert!(matches!(error, GeminiStreamError::Timeout));
    } else {
        assert!(matches!(error, GeminiStreamError::Body { .. }));
    }
    assert!(stream.next_chunk().await.is_none());
}

#[tokio::test]
async fn execution_streaming_refuses_an_overlong_line() {
    let body = object(BODY);
    let optional = object("{}");
    let call = vertex_call(&body, &optional);
    let fixture = auth();
    let oversized = vec![b'a'; MAX_RESPONSE_BYTES.saturating_add(1)];
    let recorder = Arc::new(Recorder::scripted(
        Vec::new(),
        vec![stream_reply(200, vec![Piece::Bytes(oversized)])],
    ));
    let clients = pooled(&recorder);
    let mut stream = async_streaming(
        &call,
        &catalog(),
        &llm(&fixture),
        &clients,
        Box::new(fixed_id),
    )
    .await
    .unwrap();
    let error = stream.next_chunk().await.unwrap().unwrap_err();
    assert!(matches!(error, GeminiStreamError::Line { .. }));
    assert!(stream.next_chunk().await.is_none());
}

fn stream_call<'call>(url: &'call str) -> StreamCall<'call> {
    StreamCall {
        url,
        headers: &NO_SECRET_HEADERS,
        body: BODY.as_bytes(),
        model: MODEL,
    }
}

#[rstest]
#[case::ok(200, true)]
#[case::created(201, true)]
#[case::accepted(202, false)]
#[case::no_content(204, false)]
#[case::redirect(302, false)]
#[tokio::test]
async fn execution_sync_call_gates_on_200_and_201(#[case] status: u16, #[case] accepted: bool) {
    let pieces = if accepted {
        vec![line(STREAM_CHUNK)]
    } else {
        vec![Piece::Bytes(b"denied".to_vec())]
    };
    let recorder = Arc::new(Recorder::scripted(
        Vec::new(),
        vec![stream_reply(status, pieces)],
    ));
    let outcome = make_sync_call(None, recorder.as_ref(), &stream_call(VERTEX_URL)).await;
    assert_eq!(outcome.is_ok(), accepted);
    if let Err(GeminiStreamError::Http(GeminiHttpError::Provider(provider))) = outcome {
        assert_eq!(provider.status_code, status);
        assert_eq!(provider.message, "denied");
    }
}

#[tokio::test]
async fn execution_sync_call_keeps_the_raw_error_body() {
    let recorder = Arc::new(Recorder::scripted(
        Vec::new(),
        vec![stream_reply(
            500,
            vec![Piece::Bytes(SCHEMA_ERROR.as_bytes().to_vec())],
        )],
    ));
    let error = make_sync_call(None, recorder.as_ref(), &stream_call(VERTEX_URL))
        .await
        .unwrap_err();
    let GeminiStreamError::Http(GeminiHttpError::Provider(provider)) = error else {
        panic!("a 500 is a provider rejection");
    };
    assert_eq!(provider.status_code, 500);
    assert_eq!(provider.message, SCHEMA_ERROR);
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
