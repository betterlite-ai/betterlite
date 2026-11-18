#![cfg(test)]

use std::collections::BTreeMap;
use std::str;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

use super::support;
use crate::openai_dispatch::call::{ChatCall, OpenAiOperation};
use crate::openai_dispatch::compose::chat_plan;
use crate::openai_dispatch::dispatch::dispatch;
use crate::openai_dispatch::error::OpenAiProviderError;
use crate::openai_dispatch::fixtures::answer::RecordedRequest;
use crate::openai_dispatch::fixtures::logger::FixtureLogger;
use crate::openai_dispatch::fixtures::text_transport::{FixtureTextSource, TextReply};
use crate::openai_dispatch::transport::{ClientAnswer, Deps};
use crate::openai_dispatch::{OpenAi, OpenAiRequest};
use crate::operation::{ChatCompletion, OperationError, OperationRequest, OperationResult};
use crate::prepared::deployment::Deployment;
use crate::provider::Adapter as _;

const REPLY: &str = r#"{"id":"chatcmpl-http","object":"chat.completion","created":1700000000,"model":"gpt-http-upstream-2024","choices":[{"index":0,"message":{"role":"assistant","content":"hello over http"},"logprobs":null,"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;
const ERROR_BODY: &str = r#"{"error":{"message":"temperature is not supported","type":"invalid_request_error","param":"temperature","code":"unsupported_value"}}"#;

// Reads one JSON POST from a fresh connection and answers it, returning the raw request text.
async fn serve_once(listener: TcpListener, body: &[u8], reply: &str) -> String {
    let (mut stream, _peer) = listener.accept().await.expect("the connection");
    let mut received = Vec::new();
    let mut chunk = [0_u8; 4096];
    while !received.ends_with(body) {
        let read = stream.read(&mut chunk).await.expect("the request bytes");
        assert_ne!(read, 0, "the client closed before sending the body");
        received.extend_from_slice(chunk.get(..read).expect("the bytes read"));
    }
    stream.write_all(reply.as_bytes()).await.expect("the reply");
    String::from_utf8(received).expect("the request text")
}

fn client() -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("the policy"),
    );
    ProxiedClient::outbound(&policy).expect("the client")
}

fn call(api_base: &str) -> ChatCall {
    ChatCall {
        model: String::from("gpt-http-upstream"),
        request: support::chat_request(
            r#"{"model":"gpt-http-upstream","messages":[{"role":"user","content":"hi"}]}"#,
        ),
        credentials: support::credentials(Some(api_base), Some("oracle-key"), None),
        headers: None,
        extra_headers: None,
        timeout: Some(30.0),
        metadata: None,
    }
}

// The composition a deployment builds for this call, so the production executor runs against the loopback server.
fn deployment(api_base: &str) -> Deployment {
    let mut params = IndexMap::new();
    params.insert(
        String::from("model"),
        support::text_value("gpt-http-upstream"),
    );
    params.insert(String::from("api_base"), support::text_value(api_base));
    params.insert(String::from("api_key"), support::text_value("oracle-key"));
    Deployment::from_params(params).expect("the deployment")
}

async fn loopback() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener");
    let address = listener.local_addr().expect("the address");
    (listener, format!("http://{address}/v1"))
}

// Sends the composed call to a one-shot loopback server and returns the request text, its body and the decoded reply.
async fn round_trip(reply: &str) -> (String, Vec<u8>, Result<OperationResult, OperationError>) {
    let (listener, api_base) = loopback().await;
    let composition = support::composition();
    let call = call(&api_base);
    let plan = chat_plan(&composition, &call).expect("the chat plan");
    let expected = RecordedRequest::of(&plan.prepared)
        .expect("a prepared request")
        .body;
    let (result, request) = tokio::join!(
        async {
            let client = client();
            let deployment = deployment(&api_base);
            let upstream = OpenAi
                .execute(&client, &plan.prepared, &deployment)
                .await
                .expect("the upstream answer");
            OpenAi.decode(&plan.prepared, upstream)
        },
        serve_once(listener, &expected, reply)
    );
    (request, expected, result)
}

// The composed plan names the endpoint with the resolved auth and content type, and a stream adds its body flag.
pub(super) fn check_plans() {
    let composition = support::composition();
    let plan = chat_plan(&composition, &call("http://127.0.0.1:9/v1")).expect("the chat plan");
    let sent = RecordedRequest::of(&plan.prepared).expect("a prepared HTTP request");
    assert_eq!(sent.url, "http://127.0.0.1:9/v1/chat/completions");
    assert_eq!(sent.header("authorization"), Some("Bearer oracle-key"));
    assert_eq!(sent.header("content-type"), Some("application/json"));
    let stream_call = ChatCall {
        request: support::chat_request(
            r#"{"model":"gpt-http-upstream","messages":[{"role":"user","content":"hi"}],"stream":true}"#,
        ),
        ..call("http://127.0.0.1:9/v1")
    };
    let stream_plan = chat_plan(&composition, &stream_call).expect("the chat plan");
    let stream_request = RecordedRequest::of(&stream_plan.prepared).expect("a prepared stream");
    assert_eq!(stream_request.url, "http://127.0.0.1:9/v1/chat/completions");
    assert_eq!(stream_request.header("accept"), Some("application/json"));
    let body = support::body_fields(&stream_request.body);
    assert_eq!(support::field(&body, "stream"), &JsonValue::Bool(true));
}

// The composed call reaches the upstream with its auth, headers and body, and the reply decodes.
pub(super) async fn check_round_trip() {
    let reply = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{REPLY}",
        REPLY.len()
    );
    let (request, expected, result) = round_trip(&reply).await;
    assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1\r\n"));
    let lowercase = request.to_ascii_lowercase();
    assert!(lowercase.contains("authorization: bearer oracle-key"));
    assert!(lowercase.contains("content-type: application/json"));
    assert!(request.ends_with(str::from_utf8(&expected).expect("the prepared body")));
    let OperationResult::Chat(ChatCompletion::OpenAi(completion)) =
        result.expect("the chat completion")
    else {
        panic!("a chat completion");
    };
    assert_eq!(completion.id, "chatcmpl-http");
    assert_eq!(completion.model, "gpt-http-upstream-2024");
    assert_eq!(
        completion.choices[0].message.content.as_deref(),
        Some("hello over http")
    );
    assert_eq!(completion.usage.total_tokens, 5);
}

// An upstream refusal keeps the provider's status and body, so the route renders its type, param and code.
pub(super) async fn check_failure() {
    let reply = format!(
        "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{ERROR_BODY}",
        ERROR_BODY.len()
    );
    let (request, _expected, result) = round_trip(&reply).await;
    assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1\r\n"));
    let Err(error) = result else {
        panic!("the provider refusal");
    };
    let OperationError::Provider { response } = error else {
        panic!("the provider's own error");
    };
    assert_eq!(response.status, 400);
    assert_eq!(response.body, ERROR_BODY.as_bytes());
}

// A successful streamed chat call logs the opened stream itself, with a null original response.
pub(super) async fn check_stream_post_call() {
    let (result, request, logger) = streamed_chat().await;
    assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1\r\n"));
    assert!(matches!(result, Ok(OperationResult::Stream(_))));
    assert_eq!(logger.calls.len(), 1);
    assert!(matches!(&logger.calls[0].original_response, Json::Null));
}

async fn streamed_chat() -> (
    Result<OperationResult, OpenAiProviderError>,
    String,
    FixtureLogger,
) {
    let (listener, api_base) = loopback().await;
    let composition = support::composition();
    let call = ChatCall {
        request: support::chat_request(
            r#"{"model":"gpt-http-upstream","messages":[{"role":"user","content":"hi"}],"stream":true}"#,
        ),
        ..call(&api_base)
    };
    let plan = chat_plan(&composition, &call).expect("the chat plan");
    let expected = RecordedRequest::of(&plan.prepared)
        .expect("a prepared stream")
        .body;
    let answer = ClientAnswer { client: client() };
    let text = FixtureTextSource::new(TextReply::new(200, Vec::new()));
    let mut logger = FixtureLogger::default();
    let reply = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\ndata: {\"id\":\"chatcmpl-http\"}\n\n";
    let (result, request) = tokio::join!(
        dispatch(
            OperationRequest::OpenAi(OpenAiRequest),
            OpenAiOperation::Chat(Box::new(call)),
            &composition,
            Deps {
                answer: &answer,
                text: &text,
                now: support::fixed_now,
                next_id: support::fixed_id,
            },
            &mut logger,
        ),
        serve_once(listener, &expected, reply),
    );
    (result, request, logger)
}

pub(super) async fn check_all() {
    check_plans();
    check_round_trip().await;
    check_failure().await;
    check_stream_post_call().await;
}
