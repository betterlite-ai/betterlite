#![cfg(test)]

use std::collections::BTreeMap;
use std::str;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::controls::json_value::JsonValue;
use indexmap::IndexMap;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

use super::support;
use crate::azure_dispatch::Azure;
use crate::azure_dispatch::call::Credentials;
use crate::azure_dispatch::compose::{chat_request, prepare_chat_http};
use crate::operation::{ChatCompletion, OperationError, OperationResult};
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

fn chat(api_base: String) -> crate::azure_dispatch::call::ChatCall {
    let credentials = Credentials {
        api_base: Some(api_base),
        api_version: Some(String::from("2024-10-21")),
        api_key: Some(support::key("http-key")),
        ..Credentials::default()
    };
    support::chat(
        "gpt-http-upstream",
        support::object(r#"{"temperature":1}"#),
        credentials,
    )
}

// The composed request reaches the loopback server with its deployment URL, auth and body, and the
// answer decodes through the production adapter.
pub(super) async fn check_round_trip() {
    let (listener, api_base) = loopback().await;
    let composition = support::composition(&[]);
    let (http, _json_mode) =
        prepare_chat_http(&composition, chat(api_base.clone())).expect("the chat request");
    let expected = http.body.clone();
    let reply = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nx-request-id: upstream-http-1\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{REPLY}",
        REPLY.len()
    );
    let deployment = deployment(&api_base);
    let prepared = chat_request(http, false);
    let (result, request) = tokio::join!(
        async {
            let client = client();
            let upstream = Azure
                .execute(&client, &prepared, &deployment)
                .await
                .expect("the upstream answer");
            Azure.decode(&prepared, upstream)
        },
        serve_once(listener, &expected, &reply),
    );
    assert!(request.starts_with(
        "POST /openai/deployments/gpt-http-upstream/chat/completions?api-version=2024-10-21 HTTP/1.1\r\n"
    ));
    let lowercase = request.to_ascii_lowercase();
    assert!(lowercase.contains("api-key: http-key"));
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

// An upstream refusal keeps the provider's status, body and error fields for the route to render.
pub(super) async fn check_failure() {
    let (listener, api_base) = loopback().await;
    let composition = support::composition(&[]);
    let (http, _json_mode) =
        prepare_chat_http(&composition, chat(api_base.clone())).expect("the chat request");
    let expected = http.body.clone();
    let reply = format!(
        "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{ERROR_BODY}",
        ERROR_BODY.len()
    );
    let deployment = deployment(&api_base);
    let prepared = chat_request(http, false);
    let (result, request) = tokio::join!(
        async {
            let client = client();
            let upstream = Azure
                .execute(&client, &prepared, &deployment)
                .await
                .expect("the upstream answer");
            Azure.decode(&prepared, upstream)
        },
        serve_once(listener, &expected, &reply),
    );
    assert!(request.starts_with(
        "POST /openai/deployments/gpt-http-upstream/chat/completions?api-version=2024-10-21 HTTP/1.1\r\n"
    ));
    let error = result.expect_err("the provider refusal");
    let OperationError::Provider { response } = error else {
        panic!("the provider's own error");
    };
    assert_eq!(response.status, 400);
    assert_eq!(response.body, ERROR_BODY.as_bytes());
    let body: serde_json::Value = serde_json::from_slice(&response.body).expect("the error body");
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(body["error"]["param"], "temperature");
    assert_eq!(body["error"]["code"], "unsupported_value");
}

pub(super) async fn check_all() {
    check_round_trip().await;
    check_failure().await;
}
