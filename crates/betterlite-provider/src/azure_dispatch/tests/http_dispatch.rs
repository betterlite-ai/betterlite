#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::str;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use indexmap::IndexMap;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

use super::support;
use crate::azure_dispatch::Azure;
use crate::azure_dispatch::call::{ChatCall, Credentials, Request};
use crate::azure_dispatch::compose::{prepare, prepared_request};
use crate::operation::{
    ChatCompletion, ChatOperation, OperationError, OperationRequest, OperationResult,
};
use crate::prepared::PreparedRequest;
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

async fn loopback() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener");
    let address = listener.local_addr().expect("the address");
    (listener, format!("http://{address}"))
}

fn deployment(api_base: &str) -> Deployment {
    let mut params = IndexMap::new();
    params.insert(
        String::from("model"),
        JsonValue::Text(String::from("gpt-http-upstream")),
    );
    params.insert(
        String::from("api_base"),
        JsonValue::Text(String::from(api_base)),
    );
    params.insert(
        String::from("api_key"),
        JsonValue::Text(String::from("http-key")),
    );
    Deployment::from_params(params).expect("the deployment")
}

fn chat(api_base: String) -> ChatCall {
    let credentials = Credentials {
        api_base: Some(api_base),
        api_version: Some(String::from("2024-10-21")),
        api_key: Some(support::key("http-key")),
        ..support::credentials()
    };
    let mut call = support::chat(
        "gpt-http-upstream",
        support::object(r#"{"temperature":1}"#),
        credentials,
    );
    call.client_model = Some(String::from("http-group"));
    call
}

// One composed chat request against a loopback server: the adapter executes it and decodes the answer.
async fn exchange(
    status: &str,
    headers: &str,
    body: &str,
) -> (Result<OperationResult, OperationError>, String) {
    let (listener, api_base) = loopback().await;
    let composition = support::composition(&[]);
    let composed = prepare(
        &composition,
        Request::Chat(Box::new(chat(api_base.clone()))),
    )
    .expect("the chat request");
    let expected = composed.http.body.clone();
    let response = format!(
        "HTTP/1.1 {status}\r\n{headers}content-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let deployment = deployment(&api_base);
    let request = prepared_request(composed);
    let client = client();
    let adapter = Azure { composition };
    let executed = async {
        let upstream = adapter
            .execute(&client, &request, &deployment)
            .await
            .expect("the upstream answer");
        adapter.decode(&request, upstream)
    };
    let (result, raw) = tokio::join!(executed, serve_once(listener, &expected, &response));
    assert!(raw.ends_with(str::from_utf8(&expected).expect("the prepared body")));
    (result, raw)
}

// The composed request reaches the loopback server with its deployment URL, auth and body, and the answer decodes through the adapter.
pub(super) async fn check_round_trip() {
    let headers = "content-type: application/json\r\nx-request-id: upstream-http-1\r\n";
    let (result, request) = exchange("200 OK", headers, REPLY).await;
    assert!(request.starts_with(
        "POST /openai/deployments/gpt-http-upstream/chat/completions?api-version=2024-10-21 HTTP/1.1\r\n"
    ));
    let lowercase = request.to_ascii_lowercase();
    assert!(lowercase.contains("api-key: http-key"));
    assert!(lowercase.contains("content-type: application/json"));
    let OperationResult::Chat(ChatCompletion::OpenAi(completion)) =
        result.expect("the chat completion")
    else {
        panic!("a chat completion");
    };
    assert_eq!(completion.id, "chatcmpl-http");
    assert_eq!(completion.model, "http-group");
    assert_eq!(
        completion.choices[0].message.content.as_deref(),
        Some("hello over http")
    );
    assert_eq!(completion.usage.total_tokens, 5);
}

// An upstream refusal keeps the provider's status, body and error fields for the route to render.
pub(super) async fn check_failure() {
    let headers = "content-type: application/json\r\n";
    let (result, request) = exchange("400 Bad Request", headers, ERROR_BODY).await;
    assert!(request.starts_with(
        "POST /openai/deployments/gpt-http-upstream/chat/completions?api-version=2024-10-21 HTTP/1.1\r\n"
    ));
    let Err(error) = result else {
        panic!("the provider refusal");
    };
    let OperationError::Provider { response } = error else {
        panic!("the provider's own error");
    };
    assert_eq!(response.status, 400);
    assert_eq!(response.body, ERROR_BODY.as_bytes());
    let body = support::parse(&response.body);
    let failure = support::at(&body, "error").expect("the error object");
    assert_eq!(
        support::text_at(failure, "type"),
        Some("invalid_request_error")
    );
    assert_eq!(support::text_at(failure, "param"), Some("temperature"));
    assert_eq!(support::text_at(failure, "code"), Some("unsupported_value"));
}

fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn adapter_request() -> OperationRequest {
    OperationRequest::Chat(Box::new(ChatOperation {
        model: String::from("gpt-http-upstream"),
        request: ChatRequest::parse(
            br#"{"model":"gpt-http-upstream","messages":[{"role":"user","content":"hi"}]}"#,
        )
        .expect("the chat request"),
        control: ControlOptions::default(),
    }))
}

// The adapter's own prepare composes the deployment's base and key through the production path.
fn check_adapter_prepare() {
    let target = deployment("https://deployment.test");
    let adapter = Azure {
        composition: support::composition(&[]),
    };
    let prepared = adapter
        .prepare(&adapter_request(), &target, &catalog())
        .expect("the adapter request");
    let PreparedRequest::Chat(chat) = prepared else {
        panic!("a chat request");
    };
    assert_eq!(chat.http.url.host_str(), Some("deployment.test"));
    assert_eq!(
        chat.http.url.path(),
        "/openai/deployments/gpt-http-upstream/chat/completions"
    );
    assert_eq!(
        chat.http
            .headers
            .get("api-key")
            .and_then(|value| value.to_str().ok()),
        Some("http-key")
    );
}

pub(super) async fn check_all() {
    check_round_trip().await;
    check_failure().await;
    check_adapter_prepare();
}
