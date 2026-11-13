#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::str;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::embedding::EmbeddingInput;
use rstest::rstest;
use secrecy::SecretString;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::model_policy::capabilities::VertexCapabilityProvider;
use crate::vertex_embedding_call::decode::{RESPONSE_BODY_LIMIT, decode_vertex_embedding};
use crate::vertex_embedding_call::error::VertexEmbeddingCallError;
use crate::vertex_embedding_call::multimodal::{MultimodalClients, multimodal_embedding};
use crate::vertex_embedding_call::text::{TextClients, async_text_embedding, text_embedding};

use super::support::{
    GEMINI_KEY, MULTIMODAL_ANSWER, MULTIMODAL_MODEL, NO_HEADERS, PendingPost, PostRecorder, Reply,
    TEXT_ANSWER, TEXT_MODEL, TOKEN, auth, catalog, extra, json, multimodal_call, reply, text_auth,
    text_call,
};

#[rstest]
#[case::bad_request(400)]
#[case::unauthorized(401)]
#[case::rate_limited(429)]
#[case::server_error(500)]
#[case::redirect(302)]
#[tokio::test]
async fn execution_text_preserves_status(#[case] status: u16) {
    let optional = json("{}");
    let input = EmbeddingInput::Text(String::from("hello"));
    let call = text_call(&input, &optional);
    let recorder = Arc::new(PostRecorder::scripted(vec![reply(
        status,
        r#"{"error":"upstream"}"#,
    )]));
    let fixture = auth();
    let clients = TextClients {
        pooled: recorder.as_ref(),
        caller: None,
    };
    let error = text_embedding(&call, &catalog(), &text_auth(&fixture), &clients)
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), status);
    let VertexEmbeddingCallError::Status { status: sent, body } = &error else {
        panic!("a {status} answer is not a status error: {error}");
    };
    assert_eq!(*sent, status);
    assert_eq!(body.as_slice(), br#"{"error":"upstream"}"#);
}

#[rstest]
#[case::ok(200)]
#[case::created(201)]
#[tokio::test]
async fn execution_text_2xx_succeeds(#[case] status: u16) {
    let optional = json("{}");
    let input = EmbeddingInput::Text(String::from("hello"));
    let call = text_call(&input, &optional);
    let recorder = Arc::new(PostRecorder::scripted(vec![reply(status, TEXT_ANSWER)]));
    let fixture = auth();
    let clients = TextClients {
        pooled: recorder.as_ref(),
        caller: None,
    };
    let response = text_embedding(&call, &catalog(), &text_auth(&fixture), &clients)
        .await
        .unwrap();
    assert_eq!(response.model, TEXT_MODEL);
    assert_eq!(response.usage.prompt_tokens, 2);
    assert_eq!(response.usage.total_tokens, 2);
    assert_eq!(response.data.len(), 1);
}

#[tokio::test]
async fn execution_text_posts_the_transformed_request() {
    let optional = json("{}");
    let input = EmbeddingInput::Text(String::from("hello"));
    let call = text_call(&input, &optional);
    let recorder = Arc::new(PostRecorder::scripted(vec![reply(200, TEXT_ANSWER)]));
    let fixture = auth();
    let clients = TextClients {
        pooled: recorder.as_ref(),
        caller: None,
    };
    text_embedding(&call, &catalog(), &text_auth(&fixture), &clients)
        .await
        .unwrap();
    let calls = recorder.recorded();
    assert_eq!(
        str::from_utf8(&calls[0].body).unwrap(),
        r#"{"instances":[{"content":"hello"}],"parameters":{}}"#
    );
    assert_eq!(
        calls[0].url,
        "https://us-central1-aiplatform.googleapis.com/v1/projects/project-p/locations/us-central1/publishers/google/models/text-embedding-005:predict"
    );
    let expected = BTreeMap::from([
        (String::from("Authorization"), format!("Bearer {TOKEN}")),
        (
            String::from("Content-Type"),
            String::from("application/json"),
        ),
    ]);
    assert_eq!(calls[0].headers, expected);
}

#[tokio::test]
async fn execution_multimodal_posts_the_transformed_request() {
    let input = json(r#""hello""#);
    let optional = extra("{}");
    let recorder = Arc::new(PostRecorder::scripted(vec![
        reply(200, MULTIMODAL_ANSWER),
        reply(200, MULTIMODAL_ANSWER),
    ]));
    let fixture = auth();
    let clients = MultimodalClients {
        pooled: recorder.as_ref(),
        caller: None,
    };
    let mut call = multimodal_call(MULTIMODAL_MODEL, &input, &optional, &NO_HEADERS);
    multimodal_embedding(&call, &catalog(), &fixture.cache, &clients)
        .await
        .unwrap();
    call.aembedding = true;
    multimodal_embedding(&call, &catalog(), &fixture.cache, &clients)
        .await
        .unwrap();
    let calls = recorder.recorded();
    let sync_body = str::from_utf8(&calls[0].body).unwrap();
    assert_eq!(sync_body, r#"{"instances":[{"text":"hello"}]}"#);
    assert_eq!(str::from_utf8(&calls[1].body).unwrap(), sync_body);
    assert_eq!(
        calls[0].url,
        "https://us-central1-aiplatform.googleapis.com/v1/projects/project-p/locations/us-central1/publishers/google/models/multimodalembedding@001:predict"
    );
    assert_eq!(calls[1].url, calls[0].url);
    assert_eq!(
        calls[0].headers.get("Content-Type").map(String::as_str),
        Some("application/json; charset=utf-8")
    );
}

#[derive(Clone, Copy, Debug)]
enum Branch {
    Sync,
    Async,
}

#[rstest]
#[case::sync_client_error(Branch::Sync, 400, 500)]
#[case::async_client_error(Branch::Async, 400, 400)]
#[case::sync_server_error(Branch::Sync, 500, 500)]
#[case::async_server_error(Branch::Async, 500, 500)]
#[case::sync_created(Branch::Sync, 201, 500)]
#[case::async_created(Branch::Async, 201, 500)]
#[tokio::test]
async fn execution_multimodal_status_boundaries(
    #[case] branch: Branch,
    #[case] status: u16,
    #[case] expected: u16,
) {
    let input = json(r#""hello""#);
    let optional = extra("{}");
    let mut call = multimodal_call(MULTIMODAL_MODEL, &input, &optional, &NO_HEADERS);
    call.aembedding = matches!(branch, Branch::Async);
    let recorder = Arc::new(PostRecorder::scripted(vec![reply(
        status,
        MULTIMODAL_ANSWER,
    )]));
    let fixture = auth();
    let clients = MultimodalClients {
        pooled: recorder.as_ref(),
        caller: None,
    };
    let error = multimodal_embedding(&call, &catalog(), &fixture.cache, &clients)
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), expected);
}

#[rstest]
#[case::sync(false)]
#[case::async_handoff(true)]
#[tokio::test]
async fn execution_text_acquires_one_token(#[case] aembedding: bool) {
    let optional = json("{}");
    let input = EmbeddingInput::Text(String::from("hello"));
    let mut call = text_call(&input, &optional);
    call.aembedding = aembedding;
    let recorder = Arc::new(PostRecorder::scripted(vec![reply(200, TEXT_ANSWER)]));
    let fixture = auth();
    let clients = TextClients {
        pooled: recorder.as_ref(),
        caller: None,
    };
    text_embedding(&call, &catalog(), &text_auth(&fixture), &clients)
        .await
        .unwrap();
    assert_eq!(fixture.executor.loads.load(Ordering::SeqCst), 1);
    assert_eq!(recorder.recorded().len(), 1);
}

#[rstest]
#[case::sync(false)]
#[case::async_handoff(true)]
#[tokio::test]
async fn execution_multimodal_acquires_one_token(#[case] aembedding: bool) {
    let input = json(r#""hello""#);
    let optional = extra("{}");
    let mut call = multimodal_call(MULTIMODAL_MODEL, &input, &optional, &NO_HEADERS);
    call.aembedding = aembedding;
    let recorder = Arc::new(PostRecorder::scripted(vec![reply(200, MULTIMODAL_ANSWER)]));
    let fixture = auth();
    let clients = MultimodalClients {
        pooled: recorder.as_ref(),
        caller: None,
    };
    multimodal_embedding(&call, &catalog(), &fixture.cache, &clients)
        .await
        .unwrap();
    assert_eq!(fixture.executor.loads.load(Ordering::SeqCst), 1);
    assert_eq!(recorder.recorded().len(), 1);
}

#[tokio::test]
async fn execution_gemini_reads_no_credentials() {
    let optional = json("{}");
    let input = EmbeddingInput::Text(String::from("hello"));
    let key = SecretString::from(GEMINI_KEY.to_owned());
    let mut call = text_call(&input, &optional);
    call.provider = VertexCapabilityProvider::Gemini;
    call.gemini_api_key = Some(&key);
    let recorder = Arc::new(PostRecorder::scripted(vec![reply(200, TEXT_ANSWER)]));
    let fixture = auth();
    let clients = TextClients {
        pooled: recorder.as_ref(),
        caller: None,
    };
    text_embedding(&call, &catalog(), &text_auth(&fixture), &clients)
        .await
        .unwrap();
    assert_eq!(fixture.executor.loads.load(Ordering::SeqCst), 0);
    let calls = recorder.recorded();
    assert_eq!(
        calls[0].url,
        "https://generativelanguage.googleapis.com/v1beta/models/text-embedding-005:embedContent"
    );
    assert_eq!(
        calls[0].headers.get("x-goog-api-key").map(String::as_str),
        Some(GEMINI_KEY)
    );
    assert_eq!(calls[0].headers.get("Authorization"), None);
}

#[rstest]
#[case::sync(false)]
#[case::async_handoff(true)]
#[tokio::test]
async fn execution_text_timeout_is_a_408(#[case] aembedding: bool) {
    let optional = json("{}");
    let input = EmbeddingInput::Text(String::from("hello"));
    let mut call = text_call(&input, &optional);
    call.aembedding = aembedding;
    let recorder = Arc::new(PostRecorder::scripted(vec![Reply::Timeout]));
    let fixture = auth();
    let clients = TextClients {
        pooled: recorder.as_ref(),
        caller: None,
    };
    let error = text_embedding(&call, &catalog(), &text_auth(&fixture), &clients)
        .await
        .unwrap_err();
    assert!(matches!(error, VertexEmbeddingCallError::Timeout));
    assert_eq!(error.status_code(), 408);
    assert_eq!(error.to_string(), "Timeout error occurred.");
}

#[rstest]
#[case::sync(false, 500)]
#[case::async_handoff(true, 408)]
#[tokio::test]
async fn execution_multimodal_timeout_boundaries(#[case] aembedding: bool, #[case] expected: u16) {
    let input = json(r#""hello""#);
    let optional = extra("{}");
    let mut call = multimodal_call(MULTIMODAL_MODEL, &input, &optional, &NO_HEADERS);
    call.aembedding = aembedding;
    let recorder = Arc::new(PostRecorder::scripted(vec![Reply::Timeout]));
    let fixture = auth();
    let clients = MultimodalClients {
        pooled: recorder.as_ref(),
        caller: None,
    };
    let error = multimodal_embedding(&call, &catalog(), &fixture.cache, &clients)
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), expected);
}

#[tokio::test]
async fn execution_text_transport_failure_is_a_500() {
    let optional = json("{}");
    let input = EmbeddingInput::Text(String::from("hello"));
    let call = text_call(&input, &optional);
    let recorder = Arc::new(PostRecorder::scripted(vec![Reply::Failed]));
    let fixture = auth();
    let clients = TextClients {
        pooled: recorder.as_ref(),
        caller: None,
    };
    let error = text_embedding(&call, &catalog(), &text_auth(&fixture), &clients)
        .await
        .unwrap_err();
    assert!(matches!(error, VertexEmbeddingCallError::Transport { .. }));
    assert_eq!(error.status_code(), 500);
}

#[tokio::test]
async fn execution_text_sync_client_used() {
    let optional = json("{}");
    let input = EmbeddingInput::Text(String::from("hello"));
    let call = text_call(&input, &optional);
    let caller = Arc::new(PostRecorder::scripted(vec![reply(200, TEXT_ANSWER)]));
    let pooled = Arc::new(PostRecorder::scripted(vec![reply(200, TEXT_ANSWER)]));
    let fixture = auth();
    let clients = TextClients {
        pooled: pooled.as_ref(),
        caller: Some(caller.as_ref()),
    };
    text_embedding(&call, &catalog(), &text_auth(&fixture), &clients)
        .await
        .unwrap();
    assert_eq!(caller.recorded().len(), 1);
    assert!(pooled.recorded().is_empty());
}

#[tokio::test]
async fn execution_direct_async_client_used() {
    let optional = json("{}");
    let input = EmbeddingInput::Text(String::from("hello"));
    let call = text_call(&input, &optional);
    let caller = Arc::new(PostRecorder::scripted(vec![reply(200, TEXT_ANSWER)]));
    let pooled = Arc::new(PostRecorder::scripted(vec![reply(200, TEXT_ANSWER)]));
    let fixture = auth();
    async_text_embedding(
        &call,
        &catalog(),
        &fixture.refresh,
        pooled.as_ref(),
        Some(caller.as_ref()),
    )
    .await
    .unwrap();
    assert_eq!(caller.recorded().len(), 1);
    assert!(pooled.recorded().is_empty());
}

#[tokio::test]
async fn execution_cancelled_call_drops_the_post() {
    let optional = json("{}");
    let input = EmbeddingInput::Text(String::from("hello"));
    let call = text_call(&input, &optional);
    let dropped = Arc::new(AtomicBool::new(false));
    let pending = PendingPost {
        dropped: Arc::clone(&dropped),
    };
    let fixture = auth();
    let clients = TextClients {
        pooled: &pending,
        caller: None,
    };
    let elapsed = timeout(
        Duration::from_millis(10),
        text_embedding(&call, &catalog(), &text_auth(&fixture), &clients),
    )
    .await;
    assert!(elapsed.is_err());
    assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn execution_logs_no_credentials() {
    let optional = json("{}");
    let input = EmbeddingInput::Text(String::from("hello"));
    let mut call = text_call(&input, &optional);
    call.api_base = Some("https://user:super-secret@private.example.com/");
    let recorder = Arc::new(PostRecorder::scripted(vec![reply(200, TEXT_ANSWER)]));
    let fixture = auth();
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let clients = TextClients {
        pooled: recorder.as_ref(),
        caller: None,
    };
    let guard = tracing::subscriber::set_default(subscriber);
    text_embedding(&call, &catalog(), &text_auth(&fixture), &clients)
        .await
        .unwrap();
    drop(guard);
    let logged = fs::read_to_string(log.path()).unwrap();
    assert!(logged.contains("sending the Vertex embedding request"));
    assert!(logged.contains("received the Vertex embedding answer"));
    assert!(logged.contains(TEXT_MODEL));
    assert!(logged.contains("private.example.com"));
    assert!(!logged.contains(TOKEN));
    assert!(!logged.contains("super-secret"));
}

#[tokio::test]
async fn execution_posts_through_the_pooled_client() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let api_base = format!("http://{address}/");
    let optional = json("{}");
    let input = EmbeddingInput::Text(String::from("hello"));
    let mut call = text_call(&input, &optional);
    call.api_base = Some(&api_base);
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let client = ProxiedClient::outbound(&policy).unwrap();
    let fixture = auth();
    let clients = TextClients {
        pooled: &client,
        caller: None,
    };
    let mut server = JoinSet::new();
    server.spawn(answer_embedding(listener));
    let response = text_embedding(&call, &catalog(), &text_auth(&fixture), &clients)
        .await
        .unwrap();
    assert_eq!(response.model, TEXT_MODEL);
    let received = timeout(Duration::from_secs(5), server.join_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let request = String::from_utf8(received).unwrap();
    assert!(request.starts_with(
        "POST /v1/projects/project-p/locations/us-central1/publishers/google/models/text-embedding-005:predict HTTP/1.1"
    ));
    assert!(request.contains(&format!("authorization: Bearer {TOKEN}")));
    assert!(request.contains("content-type: application/json"));
    assert!(request.ends_with(r#"{"instances":[{"content":"hello"}],"parameters":{}}"#));
}

async fn answer_embedding(listener: TcpListener) -> Vec<u8> {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut received = Vec::new();
    let mut buffer = [0_u8; 4096];
    while !request_is_complete(&received) {
        let read = stream.read(&mut buffer).await.unwrap();
        assert_ne!(
            read, 0,
            "the client closed before sending its whole request"
        );
        received.extend_from_slice(&buffer[..read]);
    }
    let body = TEXT_ANSWER.as_bytes();
    let head = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    received
}

fn request_is_complete(received: &[u8]) -> bool {
    let Some(head_end) = received.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let head = str::from_utf8(&received[..head_end]).unwrap();
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(": ")?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or_default();
    received.len() >= head_end.saturating_add(4).saturating_add(length)
}

#[test]
fn execution_decodes_a_vertex_answer_at_the_limit() {
    let at_limit = valid_string_body(RESPONSE_BODY_LIMIT);
    assert!(decode_vertex_embedding(&at_limit).is_ok());
}

#[test]
fn execution_refuses_an_oversized_vertex_answer() {
    let oversized = valid_string_body(RESPONSE_BODY_LIMIT.saturating_add(1));
    let error = decode_vertex_embedding(&oversized).unwrap_err();
    assert!(matches!(
        error,
        VertexEmbeddingCallError::ResponseTooLarge { size, limit }
            if size == RESPONSE_BODY_LIMIT.saturating_add(1) && limit == RESPONSE_BODY_LIMIT
    ));
    assert_eq!(error.status_code(), 500);
}

#[test]
fn execution_refuses_malformed_vertex_json() {
    let error = decode_vertex_embedding(b"{\"predictions\":").unwrap_err();
    assert!(matches!(error, VertexEmbeddingCallError::Decode { .. }));
    assert_eq!(error.status_code(), 500);
}

fn valid_string_body(len: usize) -> Vec<u8> {
    let mut body = Vec::with_capacity(len);
    body.push(b'"');
    body.resize(len.saturating_sub(1), b'a');
    body.push(b'"');
    body
}

#[rstest]
fn replays_the_vertex_embedding_corpus(#[files("fuzz/corpus/vertex_embedding/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    drop(decode_vertex_embedding(&bytes));
}
