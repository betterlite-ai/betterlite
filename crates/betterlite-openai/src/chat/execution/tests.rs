#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::iter;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::sleep;

use super::{ChatError, ChatExecutor};
use crate::chat::ChatRequest;
use crate::chat::deployment::{ChatDeployment, OpenAiDefaults};
use crate::openai_base_config::CatalogCapabilities;

const ANSWER: &str = r#"{"id":"c1","object":"chat.completion","created":7,"model":"upstream","choices":[{"index":0,"message":{"role":"assistant","content":null,"refusal":"no"},"finish_reason":"stop"}]}"#;

fn executor(entries: &str, seed: u64) -> ChatExecutor {
    let loader = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    };
    let config = loader.load(&format!("model_list: {entries}\n")).unwrap();
    let defaults = OpenAiDefaults {
        request_timeout: None,
        api_bases: Vec::new(),
        api_keys: Vec::new(),
        organization: Some("org-7".to_owned()),
    };
    let deployments = config
        .model_list
        .iter()
        .map(|deployment| ChatDeployment::resolve(deployment, &defaults).unwrap())
        .collect();
    let policy = Arc::new(ProxyPolicy::resolve(
        &ProxyTrust::default(),
        &BTreeMap::new(),
    ));
    ChatExecutor {
        client: ProxiedClient::outbound(&policy).unwrap(),
        deployments,
        catalog: CatalogCapabilities {
            chat_models: BTreeSet::new(),
            text_models: BTreeSet::new(),
        },
        random: Mutex::new(StdRng::seed_from_u64(seed)),
    }
}

async fn provider(listener: TcpListener, status: u16, body: &str, delay: Duration) -> String {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        received.push(stream.read_u8().await.unwrap());
    }
    let head = String::from_utf8(received).unwrap();
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .map_or(0, |value| value.parse().unwrap());
    let mut sent = vec![0_u8; length];
    stream.read_exact(&mut sent).await.unwrap();
    sleep(delay).await;
    let reply = format!(
        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\nx-request-id: r1\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _written = stream.write_all(reply.as_bytes()).await;
    format!("{head}{}", String::from_utf8(sent).unwrap())
}

async fn listen() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    (listener, base)
}

#[test]
fn picks_every_deployment_and_repeats_a_seeded_sequence() {
    let entries = "[{model_name: g, litellm_params: {model: openai/a}}, {model_name: other, litellm_params: {model: openai/x}}, {model_name: g, litellm_params: {model: openai/b}}, {model_name: g, litellm_params: {model: openai/c}}]";
    let sequence = |seed| {
        let picker = executor(entries, seed);
        iter::repeat_with(|| picker.pick("g").unwrap().litellm_model.clone())
            .take(40)
            .collect::<Vec<_>>()
    };
    let first = sequence(7);
    assert_eq!(first, sequence(7));
    let picked: BTreeSet<&str> = first.iter().map(String::as_str).collect();
    assert_eq!(picked, BTreeSet::from(["openai/a", "openai/b", "openai/c"]));
    let error = executor(entries, 7).pick("missing").unwrap_err();
    assert!(matches!(error, ChatError::UnknownModel { model } if model == "missing"));
}

#[tokio::test]
async fn sends_the_mapped_body_and_answers_with_the_requested_model() {
    let (listener, base) = listen().await;
    let entries = format!(
        "[{{model_name: g, litellm_params: {{model: openai/up, api_base: '{base}', api_key: k}}}}]"
    );
    let chat = executor(&entries, 1);
    let body = br#"{"model":"g","stream":false,"temperature":1,"user":"u","vendor_flag":{"a":1},"messages":[{"role":"assistant","content":null,"tool_calls":[{"id":"t","type":"function","function":{"name":"f","arguments":"{}"}}]}]}"#;
    let request = ChatRequest::parse(body).unwrap();
    let deployment = chat.pick("g").unwrap();
    let (sent, answered) = tokio::join!(
        provider(listener, 200, ANSWER, Duration::ZERO),
        chat.send(deployment, &request)
    );
    assert!(sent.starts_with("POST /v1/chat/completions HTTP/1.1\r\n"));
    assert!(sent.contains("authorization: Bearer k"));
    assert!(sent.contains("openai-organization: org-7"));
    assert!(sent.contains("content-type: application/json"));
    assert!(sent.contains("accept: application/json"));
    let upstream: Value = serde_json::from_str(sent.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    let expected = r#"{"model":"up","temperature":1,"vendor_flag":{"a":1},"messages":[{"role":"assistant","tool_calls":[{"id":"t","type":"function","function":{"name":"f","arguments":"{}"}}]}]}"#;
    assert_eq!(upstream, serde_json::from_str::<Value>(expected).unwrap());
    let outcome = answered.unwrap();
    assert_eq!(outcome.upstream.status, 200);
    assert_eq!(outcome.upstream.headers.get("x-request-id").unwrap(), "r1");
    let client_body = serde_json::to_value(&outcome.response).unwrap();
    let client_expected = r#"{"id":"c1","object":"chat.completion","created":7,"model":"g","usage":{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0},"choices":[{"index":0,"finish_reason":"stop","provider_specific_fields":{},"message":{"role":"assistant","content":null,"provider_specific_fields":{"refusal":"no"}}}]}"#;
    assert_eq!(
        client_body,
        serde_json::from_str::<Value>(client_expected).unwrap()
    );
}

#[rstest]
#[case::provider_error(400, ANSWER, Duration::ZERO, "status 400")]
#[case::not_a_completion(200, "[]", Duration::ZERO, "not a chat completion")]
#[case::slower_than_the_timeout(
    200,
    ANSWER,
    Duration::from_secs(2),
    "within the deployment timeout"
)]
#[tokio::test]
async fn reports_what_went_wrong_upstream(
    #[case] status: u16,
    #[case] answer: &str,
    #[case] delay: Duration,
    #[case] message: &str,
) {
    let (listener, base) = listen().await;
    let entries = format!(
        "[{{model_name: g, litellm_params: {{model: openai/up, api_base: '{base}', api_key: k, timeout: 0.3}}}}]"
    );
    let chat = executor(&entries, 1);
    let request = ChatRequest::parse(br#"{"messages":[]}"#).unwrap();
    let deployment = chat.pick("g").unwrap();
    let (_sent, answered) = tokio::join!(
        provider(listener, status, answer, delay),
        chat.send(deployment, &request)
    );
    let error = answered.err().unwrap();
    assert!(error.to_string().contains(message), "{error}");
    if delay > Duration::ZERO {
        assert_eq!(error.failure().status, 408);
    }
}

#[rstest]
#[case::streaming(
    "{model: openai/m, api_key: k}",
    r#"{"stream":true,"messages":[]}"#,
    "streaming"
)]
#[case::other_provider("{model: azure/m, api_key: k}", r#"{"messages":[]}"#, "azure/m")]
#[case::no_api_key("{model: openai/m}", r#"{"messages":[]}"#, "no API key")]
#[case::unreachable(
    "{model: openai/m, api_key: k, api_base: 'http://127.0.0.1:1'}",
    r#"{"messages":[]}"#,
    "could not be reached"
)]
#[tokio::test]
async fn refuses_calls_it_cannot_make(
    #[case] params: &str,
    #[case] body: &str,
    #[case] message: &str,
) {
    let chat = executor(&format!("[{{model_name: g, litellm_params: {params}}}]"), 1);
    let request = ChatRequest::parse(body.as_bytes()).unwrap();
    let error = chat
        .send(chat.pick("g").unwrap(), &request)
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains(message), "{error}");
}

#[rstest]
#[case::default(r#"{"messages":[],"stream":true}"#, Some(true))]
#[case::enabled(
    r#"{"messages":[],"stream":true,"stream_options":{"include_usage":true}}"#,
    Some(true)
)]
#[case::disabled(
    r#"{"messages":[],"stream":true,"stream_options":{"include_usage":false}}"#,
    Some(false)
)]
#[case::buffered(r#"{"messages":[]}"#, None)]
fn stream_options_preserve_caller(#[case] input: &str, #[case] usage: Option<bool>) {
    let chat = executor("[]", 1);
    let request = ChatRequest::parse(input.as_bytes()).unwrap();
    let body = chat.body(&request, "up").unwrap();
    assert_eq!(
        body.get("stream"),
        request.stream.then_some(&Value::Bool(true))
    );
    let actual = body
        .get("stream_options")
        .and_then(|options| options.get("include_usage"))
        .and_then(Value::as_bool);
    assert_eq!(actual, usage);
}
