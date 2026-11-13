#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::{LiteLlmParams, ProxyTrust};
use betterlite_http::proxy::ProxiedClient;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Map, Value};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::timeout;

use super::counter::{CountTokensCall, OpenAiTokenCounter, TokenCountResponse, TokenCounter as _};
use super::handler::CountTokensHandler;
use super::messages::ChatMessage;
use super::python_text::PyValue;

pub(super) const KEY: &str = "sk-fixture-key";
pub(super) const OK: &str = r#"{"input_tokens":7}"#;

#[derive(Deserialize)]
pub(super) struct Scenario {
    model: String,
    messages: Vec<ChatMessage>,
    tools: Option<Vec<Map<String, Value>>>,
    system: Option<PyValue>,
    #[serde(default = "deployment_key")]
    key: String,
    #[serde(default)]
    no_key: bool,
    env_key: Option<String>,
    provider: Option<String>,
}

fn deployment_key() -> String {
    String::from("sk-deployment")
}

async fn provider(listener: TcpListener, status: u16, answer: &str) -> (String, Value) {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut raw = Vec::new();
    while !raw.ends_with(b"\r\n\r\n") {
        raw.push(stream.read_u8().await.unwrap());
    }
    let head = String::from_utf8(raw).unwrap();
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .map_or(0, |value| value.parse().unwrap());
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).await.unwrap();
    let reply = format!(
        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
        answer.len()
    );
    let _written = stream.write_all(reply.as_bytes()).await;
    assert!(head.starts_with("POST /v1/responses/input_tokens HTTP/1.1\r\n"));
    assert!(head.contains("content-type: application/json\r\n"));
    let authorization = head
        .lines()
        .find_map(|line| line.strip_prefix("authorization: "))
        .unwrap()
        .to_owned();
    (authorization, serde_json::from_slice(&body).unwrap())
}

pub(super) fn counter(env_key: Option<String>) -> OpenAiTokenCounter {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    OpenAiTokenCounter {
        handler: CountTokensHandler {
            client: ProxiedClient::outbound(&policy).unwrap(),
            request_timeout: Duration::from_mins(100),
        },
        environment_api_key: env_key.map(SecretString::from),
    }
}

pub(super) fn params(key: Option<String>, base: String) -> LiteLlmParams {
    LiteLlmParams {
        ssl_verify: None,
        model: None,
        api_base: Some(base),
        api_key: key.map(SecretString::from),
        api_version: None,
        timeout: None,
        stream_timeout: None,
        max_retries: None,
        rpm: None,
        tpm: None,
        weight: None,
    }
}

pub(super) fn render(result: Option<&TokenCountResponse>) -> String {
    match result {
        None => String::from("none"),
        Some(found) if found.error => format!(
            "error {}: {}",
            found.status_code.unwrap(),
            found.error_message.as_deref().unwrap()
        ),
        Some(found) => format!(
            "tokens={} original={}",
            found.total_tokens,
            serde_json::to_string(&found.original_response).unwrap()
        ),
    }
}

// The fixture server listens once, so a run that never reaches HTTP shows as no request
pub(super) async fn run(
    source: &str,
    status: u16,
    answer: &str,
) -> (String, Option<(String, Value)>) {
    let scenario: Scenario = serde_json::from_str(source).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1/", listener.local_addr().unwrap());
    let deployment = params((!scenario.no_key).then_some(scenario.key), base);
    let call = CountTokensCall {
        model_to_use: &scenario.model,
        messages: &scenario.messages,
        deployment: Some(&deployment),
        request_model: "requested",
        tools: scenario.tools.as_deref(),
        system: scenario.system.as_ref(),
    };
    let counter = counter(scenario.env_key);
    let (result, sent) = tokio::join!(
        counter.count_tokens(&call),
        timeout(
            Duration::from_millis(300),
            provider(listener, status, answer)
        )
    );
    if let Some(found) = &result {
        assert_eq!(found.request_model, "requested");
        assert_eq!(found.model_used, scenario.model);
        assert_eq!(found.tokenizer_type, "openai_api");
    }
    let mut rendered = render(result.as_ref());
    if let Some(name) = &scenario.provider {
        let selected = counter.should_use_token_counting_api(Some(name));
        rendered = format!("native={selected} {rendered}");
    }
    (rendered, sent.ok())
}

mod behavior;
mod cases;
mod invariants;
