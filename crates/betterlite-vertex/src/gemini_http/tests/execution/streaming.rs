#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use betterlite_http::error::HttpError;
use rstest::rstest;
use secrecy::SecretString;

use crate::gemini_http::call::async_streaming;
use crate::gemini_http::error::{GeminiHttpError, GeminiStreamError};
use crate::gemini_http::stream::{StreamCall, make_sync_call};
use crate::gemini_http::tests::support::{
    BODY, GEMINI_KEY, MODEL, NO_SECRET_HEADERS, OpenReply, Piece, Recorder, SCHEMA_ERROR,
    STREAM_CHUNK, auth, catalog, line, llm, object, pooled, stream_reply, supplied, vertex_call,
};
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::model_policy::capabilities::VertexCapabilityProvider;
use crate::vertex_embedding_call::post::ClientTimeout;
use betterlite_core::api::vector_store_http_write::service::ClientKind;

use super::{VERTEX_URL, fixed_id};

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
    let clients = supplied(&caller, ClientKind::Asynchronous, &pooled_recorder);
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
    let GeminiStreamError::Http(http) = error else {
        panic!("a transport failure is an HTTP error");
    };
    assert!(matches!(http, GeminiHttpError::Transport { .. }));
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
    let GeminiStreamError::Http(http) = error else {
        panic!("an open timeout is an HTTP error");
    };
    let provider = http.provider().expect("a timeout is a provider error");
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
async fn execution_streaming_delivers_queued_lines_before_a_failure() {
    let body = object(BODY);
    let optional = object("{}");
    let call = vertex_call(&body, &optional);
    let fixture = auth();
    let recorder = Arc::new(Recorder::scripted(
        Vec::new(),
        vec![stream_reply(
            200,
            vec![line(STREAM_CHUNK), Piece::Failed(HttpError::Timeout)],
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
    let chunk = stream.next_chunk().await.unwrap().unwrap();
    assert_eq!(chunk.id, "r1");
    let error = stream.next_chunk().await.unwrap().unwrap_err();
    assert!(matches!(error, GeminiStreamError::Timeout));
    assert!(stream.next_chunk().await.is_none());
}

// a value spliced onto the same data line stays in the decoder until the stream's final flush
#[tokio::test]
async fn execution_streaming_finishes_a_split_payload() {
    let body = object(BODY);
    let optional = object("{}");
    let call = vertex_call(&body, &optional);
    let fixture = auth();
    let second = r#"{"responseId":"r2","candidates":[{"content":{"parts":[{"text":"llo"}]}}]}"#;
    let spliced = format!("{STREAM_CHUNK}{second}\n\n");
    let recorder = Arc::new(Recorder::scripted(
        Vec::new(),
        vec![stream_reply(200, vec![Piece::Bytes(spliced.into_bytes())])],
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
    let first = stream.next_chunk().await.unwrap().unwrap();
    assert_eq!(first.id, "r1");
    let chunk = stream.next_chunk().await.unwrap().unwrap();
    assert_eq!(chunk.id, "r2");
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

#[tokio::test]
async fn execution_streaming_refuses_an_oversized_error_body() {
    let body = object(BODY);
    let optional = object("{}");
    let call = vertex_call(&body, &optional);
    let fixture = auth();
    let oversized = vec![b'e'; MAX_RESPONSE_BYTES.saturating_add(1)];
    let recorder = Arc::new(Recorder::scripted(
        Vec::new(),
        vec![stream_reply(500, vec![Piece::Bytes(oversized)])],
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
    assert!(matches!(
        error,
        GeminiStreamError::TooLarge { limit } if limit == MAX_RESPONSE_BYTES
    ));
}

#[tokio::test]
async fn execution_streaming_keeps_an_error_body_at_the_limit() {
    let body = object(BODY);
    let optional = object("{}");
    let call = vertex_call(&body, &optional);
    let fixture = auth();
    let at_limit = vec![b'e'; MAX_RESPONSE_BYTES];
    let recorder = Arc::new(Recorder::scripted(
        Vec::new(),
        vec![stream_reply(500, vec![Piece::Bytes(at_limit)])],
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
        panic!("an error body at the limit is the provider's message");
    };
    assert_eq!(provider.message.len(), MAX_RESPONSE_BYTES);
}

fn stream_call(url: &str) -> StreamCall<'_> {
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
    let outcome = make_sync_call(None, recorder.as_ref(), &stream_call(VERTEX_URL)).await;
    let Err(error) = outcome else {
        panic!("a 500 is a provider rejection, not an opened stream");
    };
    let GeminiStreamError::Http(GeminiHttpError::Provider(provider)) = error else {
        panic!("a 500 is a provider rejection");
    };
    assert_eq!(provider.status_code, 500);
    assert_eq!(provider.message, SCHEMA_ERROR);
}
