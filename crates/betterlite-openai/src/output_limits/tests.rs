#![cfg(test)]
#![cfg_attr(
    test,
    expect(
        clippy::shadow_unrelated,
        clippy::too_many_lines,
        reason = "each case carries several provider replies in one function and rebinds the reply names as it goes"
    )
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::mock_response::failure::MockError;
use betterlite_config::sections::mock_response::selection::MockPlan;
use betterlite_config::sections::mock_response::{MockResponse, UsageDefaults};
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use futures_util::StreamExt as _;
use http::{HeaderMap, HeaderValue};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use serde_json::{Map, Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::timeout;
use url::Url;
use uuid::Uuid;

use super::build::{RecoveredBody, Recovery, build_limit_response};
use super::tokens::{PromptCountError, PromptTokens};
use super::{
    ErrorBody, OUTPUT_TOKEN_LIMIT_MARKER, RejectedFieldsError, decode_rejected_fields,
    without_rejected_fields,
};
use crate::chat::deployment::{ChatDeployment, Provider};
use crate::chat::execution::{ChatError, ChatExecutor};
use crate::chat::response::ChatResponse;
use crate::chat::{ChatRequest, MockSettings};
use crate::embedding::EmbeddingResponse;
use crate::openai_base_config::CatalogCapabilities;

const COMPLETION: &str = r#"{"id":"c1","object":"chat.completion","created":7,"model":"upstream","choices":[{"index":0,"message":{"role":"assistant","content":"up"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
const USER_TURN: &str = r#"[{"role":"user","content":"hello world"}]"#;

fn fixed_clock() -> u64 {
    1_700_000_000
}

fn fixed_id() -> Uuid {
    Uuid::from_u128(7)
}

fn fields(entries: &[(&str, Value)]) -> Map<String, Value> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn header_map(entries: &[(&'static str, &'static str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in entries {
        map.insert(*name, HeaderValue::from_static(value));
    }
    map
}

// the prompt count is injected in most cases; the production tokenizer is asserted where the count itself matters
struct Recorder {
    calls: Mutex<Vec<(String, Vec<Value>)>>,
    tokens: usize,
}

impl Recorder {
    fn new(tokens: usize) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            tokens,
        }
    }

    fn calls(&self) -> Vec<(String, Vec<Value>)> {
        self.calls.lock().expect("calls").clone()
    }
}

#[async_trait]
impl PromptTokens for Recorder {
    async fn count(&self, model: &str, messages: &[Value]) -> Result<usize, PromptCountError> {
        self.calls
            .lock()
            .expect("calls")
            .push((model.to_owned(), messages.to_vec()));
        Ok(self.tokens)
    }
}

fn recovery(tokens: &dyn PromptTokens) -> Recovery<'_> {
    Recovery {
        clock: fixed_clock,
        next_id: fixed_id,
        tokens,
    }
}

fn deployment(base_url: &str) -> ChatDeployment {
    let api_base = format!("{base_url}/v1");
    let endpoint = Url::parse(&format!("{base_url}/v1/chat/completions")).expect("endpoint");
    let provider = Provider::OpenAi {
        model: String::from("gpt-3.5-turbo"),
    };
    ChatDeployment {
        model_group: String::from("g"),
        litellm_model: String::from("openai/gpt-3.5-turbo"),
        provider,
        api_base,
        endpoint,
        authorization: None,
        api_key: Some(secrecy::SecretString::from(String::from("k"))),
        organization: None,
        timeout: Duration::from_secs(30),
        timeout_seconds: 30.0,
        model_id: String::from("g"),
        weight: 0,
        rpm: 0,
        tpm: 0,
    }
}

fn unused_chat(
    _plan: &MockPlan<'_>,
    _usage: UsageDefaults,
    _model: &str,
    _id: &str,
    _created: u64,
) -> Result<ChatResponse, MockError> {
    Err(MockError::WrongType {
        field: "mock_response",
    })
}

fn unused_embedding(
    _response: Option<&MockResponse>,
    _model: &str,
) -> Result<EmbeddingResponse, MockError> {
    Err(MockError::EmbeddingFailure)
}

fn executor(base_url: &str) -> ChatExecutor {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("proxy");
    let client = ProxiedClient::outbound(&Arc::new(policy)).expect("client");
    ChatExecutor {
        client,
        deployments: vec![deployment(base_url)],
        catalog: CatalogCapabilities {
            chat_models: BTreeSet::new(),
            text_models: BTreeSet::new(),
        },
        random: Mutex::new(StdRng::seed_from_u64(1)),
        clock: fixed_clock,
        next_id: fixed_id,
        mock: MockSettings::default(),
        mock_chat: unused_chat,
        mock_embedding: unused_embedding,
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("redactor"),
    }
}

struct Reply {
    status: u16,
    body: String,
}

async fn serve(listener: TcpListener, replies: Vec<Reply>, seen: Arc<Mutex<Vec<String>>>) {
    for reply in replies {
        let (mut stream, _) = timeout(Duration::from_secs(5), listener.accept())
            .await
            .expect("no request reached the provider")
            .expect("accepted request");
        let mut received = Vec::new();
        while !received.ends_with(b"\r\n\r\n") {
            received.push(stream.read_u8().await.expect("request head"));
        }
        let head = String::from_utf8(received).expect("request head text");
        let length: usize = head
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .map_or(0, |value| value.parse().expect("content length"));
        let mut sent = vec![0_u8; length];
        stream.read_exact(&mut sent).await.expect("request body");
        seen.lock()
            .expect("seen")
            .push(String::from_utf8(sent).expect("request text"));
        let response = format!(
            "HTTP/1.1 {} X\r\ncontent-type: application/json\r\nx-request-id: r1\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            reply.status,
            reply.body.len(),
            reply.body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("provider reply");
    }
}

async fn listen() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    (listener, base)
}

fn bodies(seen: &Mutex<Vec<String>>) -> Vec<Value> {
    seen.lock()
        .expect("seen")
        .iter()
        .map(|request| {
            let body = request.split("\r\n\r\n").nth(1).expect("request body");
            serde_json::from_str(body).expect("request json")
        })
        .collect()
}

fn marker_error() -> String {
    format!(
        r#"{{"error":{{"message":"{OUTPUT_TOKEN_LIMIT_MARKER}","type":"invalid_request_error","param":null,"code":"max_tokens"}}}}"#
    )
}

fn two_element_loc() {
    let body = json!({"message": "{\"detail\": [{\"loc\": [\"body\", \"temperature\"]}, {\"loc\": [\"body\", \"top_p\"]}, {\"loc\": [\"body\"]}, {\"loc\": []}, {\"loc\": [\"body\", \"seed\", \"extra\"]}, {\"loc\": \"body\"}, {\"loc\": null}, {}]}"});
    let data = fields(&[
        ("temperature", json!(1)),
        ("top_p", json!(2)),
        ("seed", json!(3)),
    ]);
    let dropped = without_rejected_fields(ErrorBody::Sdk(&body), &data).expect("sdk body");
    assert_eq!(dropped, fields(&[("seed", json!(3))]));
    assert_eq!(data.len(), 3, "the caller's map is untouched");
    let http = json!({"error": {"message": "{\"detail\": [{\"loc\": [\"body\", \"seed\"]}]}"}});
    let text = serde_json::to_vec(&http).expect("http body");
    let dropped = without_rejected_fields(ErrorBody::Http(&text), &data).expect("http");
    assert_eq!(
        dropped,
        fields(&[("temperature", json!(1)), ("top_p", json!(2))])
    );
    let direct = json!({"message": {"detail": [{"loc": ["body", "top_p"]}]}});
    let dropped = without_rejected_fields(ErrorBody::Sdk(&direct), &data).expect("direct");
    assert_eq!(
        dropped,
        fields(&[("temperature", json!(1)), ("seed", json!(3))])
    );
    let unparsable = json!({"message": "not json at all"});
    let dropped =
        without_rejected_fields(ErrorBody::Sdk(&unparsable), &data).expect("unparsable message");
    assert_eq!(dropped, data);
}

fn malformed_detail_keeps_fields() {
    let data = fields(&[("temperature", json!(1)), ("seed", json!(3))]);
    let kept = [
        json!(null),
        json!("a string body"),
        json!({"message": ""}),
        json!({"message": 0}),
        json!({"message": []}),
        json!({"message": {}}),
        json!({"message": "{}"}),
        json!({"message": "{\"detail\": []}"}),
        json!({"message": "{\"detail\": {\"loc\": [\"body\", \"temperature\"]}}"}),
        json!({"message": "{\"detail\": [5, {\"loc\": [\"body\", \"temperature\"]}]}"}),
    ];
    for body in kept {
        let dropped = without_rejected_fields(ErrorBody::Sdk(&body), &data).expect("kept fields");
        assert_eq!(dropped, data, "{body}");
    }
    let dropped = without_rejected_fields(ErrorBody::Http(b"{}"), &data).expect("no error key");
    assert_eq!(dropped, data);
}

// the truthiness the message and loc checks read; an empty value is absent even though the field is there
#[test]
fn present_matches_python_truthiness() {
    assert!(!super::present(&json!(null)));
    assert!(!super::present(&json!(false)));
    assert!(!super::present(&json!(0)));
    assert!(!super::present(&json!("")));
    assert!(!super::present(&json!([])));
    assert!(!super::present(&json!({})));
    assert!(super::present(&json!(true)));
    assert!(super::present(&json!(1)));
    assert!(super::present(&json!(-1)));
    assert!(super::present(&json!("x")));
    assert!(super::present(&json!([0])));
    assert!(super::present(&json!({"a": 1})));
}

fn output_limit_case_insensitive() {
    let marker = OUTPUT_TOKEN_LIMIT_MARKER.to_uppercase();
    let rendered = format!("Error code: 400 - {{\"error\": {{\"message\": \"{marker}\"}}}}");
    assert!(ErrorBody::Http(rendered.as_bytes()).reaches_output_limit());
    let rendered = format!("Azure rejected gpt-5.1 with: {marker} for this request");
    assert!(ErrorBody::Http(rendered.as_bytes()).reaches_output_limit());
    for shorter in [
        "could not finish the message",
        "max_tokens or model output limit was reached",
        "Error code: 400 - invalid max_tokens value",
        "",
    ] {
        assert!(
            !ErrorBody::Http(shorter.as_bytes()).reaches_output_limit(),
            "{shorter}"
        );
    }
}

async fn empty_length_completion() {
    let recorder = Recorder::new(7);
    let data = fields(&[
        ("model", json!("gpt-3.5-turbo")),
        (
            "messages",
            json!([{"role": "user", "content": "hello world"}]),
        ),
    ]);
    let answer = build_limit_response(
        &data,
        header_map(&[("x-request-id", "r1")]),
        &recovery(&recorder),
    )
    .await
    .expect("recovered reply");
    assert_eq!(answer.headers, header_map(&[("x-request-id", "r1")]));
    let RecoveredBody::Completion(completion) = answer.body else {
        panic!("a buffered request recovers a completion");
    };
    assert_eq!(
        serde_json::to_value(&completion).expect("completion json"),
        json!({
            "id": "chatcmpl-00000000-0000-0000-0000-000000000007",
            "object": "chat.completion",
            "created": 1_700_000_000,
            "model": "gpt-3.5-turbo",
            "choices": [{
                "index": 0,
                "finish_reason": "length",
                "provider_specific_fields": {},
                "message": {
                    "role": "assistant",
                    "content": "",
                    "provider_specific_fields": {"refusal": null},
                },
            }],
            "usage": {"prompt_tokens": 7, "completion_tokens": 0, "total_tokens": 7},
        })
    );
    assert_eq!(
        recorder.calls(),
        vec![(
            String::from("gpt-3.5-turbo"),
            vec![json!({"role": "user", "content": "hello world"})],
        )]
    );
}

async fn single_length_stream_chunk() {
    let recorder = Recorder::new(5);
    let data = fields(&[
        ("model", json!("gpt-3.5-turbo")),
        ("messages", json!([{"role": "user", "content": "hi"}])),
        ("stream", json!(true)),
    ]);
    let answer = build_limit_response(
        &data,
        header_map(&[("x-request-id", "r1")]),
        &recovery(&recorder),
    )
    .await
    .expect("recovered chunk");
    assert_eq!(answer.headers, header_map(&[("x-request-id", "r1")]));
    let RecoveredBody::Chunk(chunk) = answer.body else {
        panic!("a stream request recovers one chunk");
    };
    let serialized = serde_json::to_value(&chunk).expect("chunk json");
    assert_eq!(
        serialized,
        json!({
            "id": "chatcmpl-00000000-0000-0000-0000-000000000007",
            "object": "chat.completion.chunk",
            "created": 1_700_000_000,
            "model": "gpt-3.5-turbo",
            "choices": [{
                "index": 0,
                "finish_reason": "length",
                "delta": {"role": "assistant", "content": ""},
            }],
        })
    );
    assert!(serialized.get("usage").is_none());
    assert_eq!(recorder.calls().len(), 1, "the prompt is counted anyway");
}

async fn prompt_token_count() {
    let recorder = Recorder::new(7);
    let messages = json!([{"role": "user", "content": "hello world"}]);
    let data = fields(&[
        ("model", json!("gpt-3.5-turbo")),
        ("messages", messages.clone()),
    ]);
    let answer = build_limit_response(
        &data,
        header_map(&[("x-request-id", "r1")]),
        &recovery(&recorder),
    )
    .await
    .expect("counted reply");
    assert_eq!(answer.headers, header_map(&[("x-request-id", "r1")]));
    assert_eq!(
        recorder.calls(),
        vec![(String::from("gpt-3.5-turbo"), messages.to_vec())]
    );
    let unmodelled = fields(&[("messages", json!([]))]);
    build_limit_response(&unmodelled, HeaderMap::new(), &recovery(&recorder))
        .await
        .expect("empty model");
    assert_eq!(
        recorder.calls().last().map(|call| call.0.as_str()),
        Some("")
    );
    let uncounted = Recorder::new(7);
    for data in [
        fields(&[("model", json!("gpt-3.5-turbo"))]),
        fields(&[
            ("model", json!("gpt-3.5-turbo")),
            ("messages", json!("not a list")),
        ]),
    ] {
        let answer = build_limit_response(&data, HeaderMap::new(), &recovery(&uncounted))
            .await
            .expect("uncounted reply");
        let RecoveredBody::Completion(completion) = answer.body else {
            panic!("a buffered request recovers a completion");
        };
        assert_eq!(completion.usage.prompt_tokens, 0);
        assert_eq!(completion.usage.total_tokens, 0);
    }
    assert!(uncounted.calls().is_empty());
    let counted = build_limit_response(
        &fields(&[
            ("model", json!("gpt-3.5-turbo")),
            (
                "messages",
                json!([{"role": "user", "content": "hello world"}]),
            ),
        ]),
        HeaderMap::new(),
        &Recovery {
            clock: fixed_clock,
            next_id: fixed_id,
            tokens: &super::tokens::TokenizerPromptTokens,
        },
    )
    .await;
    let RecoveredBody::Completion(completion) = counted.expect("tokenized reply").body else {
        panic!("a buffered request recovers a completion");
    };
    assert_eq!(completion.usage.prompt_tokens, 9);
    assert_eq!(completion.usage.total_tokens, 9);
}

async fn unrelated_error() {
    let (listener, base) = listen().await;
    let chat = executor(&base);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let request = ChatRequest::parse(
        br#"{"model":"g","max_tokens":1,"messages":[{"role":"user","content":"hi"}]}"#,
    )
    .expect("request");
    let replies = vec![Reply {
        status: 400,
        body: String::from(
            r#"{"error":{"message":"could not finish the message","type":"invalid_request_error","param":"max_tokens"}}"#,
        ),
    }];
    let ((), answered) = tokio::join!(
        serve(listener, replies, Arc::clone(&seen)),
        chat.send(chat.deployments.first().expect("deployment"), &request)
    );
    let error = answered.expect_err("an unrelated 400 is not repaired");
    assert!(matches!(&error, ChatError::Provider { .. }), "{error}");
    let failure = error.failure(&chat.redactor);
    assert_eq!(failure.status, 400);
    assert_eq!(failure.error_type, "invalid_request_error");
    assert_eq!(failure.param.as_deref(), Some("max_tokens"));
    assert_eq!(
        bodies(&seen),
        vec![json!({
            "model": "gpt-3.5-turbo",
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "hi"}],
        })]
    );
}

async fn openai_output_limit_buffered() {
    let (listener, base) = listen().await;
    let chat = executor(&base);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let request =
        ChatRequest::parse(format!(r#"{{"model":"g","messages":{USER_TURN}}}"#).as_bytes())
            .expect("request");
    let replies = vec![Reply {
        status: 400,
        body: marker_error(),
    }];
    let ((), answered) = tokio::join!(
        serve(listener, replies, Arc::clone(&seen)),
        chat.send(chat.deployments.first().expect("deployment"), &request)
    );
    let outcome = answered.expect("the output limit becomes a reply");
    assert_eq!(outcome.upstream.status, 200);
    assert_eq!(
        outcome
            .upstream
            .headers
            .get("x-request-id")
            .expect("header"),
        "r1"
    );
    assert_eq!(
        serde_json::to_value(&outcome.response).expect("response json"),
        json!({
            "id": "chatcmpl-00000000-0000-0000-0000-000000000007",
            "object": "chat.completion",
            "created": 1_700_000_000,
            "model": "g",
            "choices": [{
                "index": 0,
                "finish_reason": "length",
                "provider_specific_fields": {},
                "message": {
                    "role": "assistant",
                    "content": "",
                    "provider_specific_fields": {"refusal": null},
                },
            }],
            "usage": {"prompt_tokens": 9, "completion_tokens": 0, "total_tokens": 9},
        })
    );
    let sent = bodies(&seen);
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent.first().and_then(|body| body.get("messages")),
        Some(&json!([{"role": "user", "content": "hello world"}]))
    );
    assert!(sent.first().and_then(|body| body.get("stream")).is_none());
}

async fn openai_output_limit_stream() {
    let (listener, base) = listen().await;
    let chat = executor(&base);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let request = ChatRequest::parse(
        format!(r#"{{"model":"g","stream":true,"messages":{USER_TURN}}}"#).as_bytes(),
    )
    .expect("request");
    let replies = vec![Reply {
        status: 400,
        body: marker_error(),
    }];
    let ((), answered) = tokio::join!(
        serve(listener, replies, Arc::clone(&seen)),
        chat.stream(chat.deployments.first().expect("deployment"), &request)
    );
    let stream = answered.expect("the output limit becomes a chunk");
    assert_eq!(stream.headers.get("x-request-id").expect("header"), "r1");
    let mut body = stream.body;
    let mut frames = Vec::new();
    while let Some(frame) = body.next().await {
        frames.push(frame.expect("frame"));
    }
    assert_eq!(frames.len(), 1);
    let frame = String::from_utf8(frames.remove(0)).expect("frame text");
    let chunk = frame
        .strip_prefix("data: ")
        .and_then(|rest| rest.strip_suffix("\n\n"))
        .expect("one sse event");
    assert_eq!(
        serde_json::from_str::<Value>(chunk).expect("chunk json"),
        json!({
            "id": "chatcmpl-00000000-0000-0000-0000-000000000007",
            "object": "chat.completion.chunk",
            "created": 1_700_000_000,
            "model": "gpt-3.5-turbo",
            "choices": [{
                "index": 0,
                "finish_reason": "length",
                "delta": {"role": "assistant", "content": ""},
            }],
        })
    );
    let sent = bodies(&seen);
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent.first().and_then(|body| body.get("stream")),
        Some(&json!(true))
    );
    let (listener, base) = listen().await;
    let chat = executor(&base);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let request = ChatRequest::parse(
        format!(r#"{{"model":"g","stream":true,"messages":{USER_TURN}}}"#).as_bytes(),
    )
    .expect("request");
    let replies = vec![Reply {
        status: 400,
        body: String::from(
            r#"{"error":{"message":"invalid max_tokens value","type":"invalid_request_error","param":"max_tokens"}}"#,
        ),
    }];
    let ((), answered) = tokio::join!(
        serve(listener, replies, Arc::clone(&seen)),
        chat.stream(chat.deployments.first().expect("deployment"), &request)
    );
    let Err(error) = answered else {
        panic!("an unrelated 400 is not repaired on the stream path");
    };
    assert!(matches!(&error, ChatError::Provider { .. }), "{error}");
    assert_eq!(error.failure(&chat.redactor).status, 400);
    assert_eq!(bodies(&seen).len(), 1);
}

async fn openai_422_drop_retry() {
    let (listener, base) = listen().await;
    let chat = executor(&base);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let request = ChatRequest::parse(
        br#"{"model":"g","temperature":2,"drop_params":true,"messages":[{"role":"user","content":"hi"}]}"#,
    )
    .expect("request");
    let replies = vec![
        Reply {
            status: 422,
            body: String::from(
                r#"{"error":{"message":"{\"detail\":[{\"loc\":[\"body\",\"temperature\"],\"msg\":\"unsupported value\"}]}","type":"invalid_request_error"}}"#,
            ),
        },
        Reply {
            status: 200,
            body: String::from(COMPLETION),
        },
    ];
    let ((), answered) = tokio::join!(
        serve(listener, replies, Arc::clone(&seen)),
        chat.send(chat.deployments.first().expect("deployment"), &request)
    );
    let outcome = answered.expect("the rejected field is dropped");
    assert_eq!(outcome.response.id, "c1");
    assert!(
        request.temperature.is_some(),
        "the caller's request keeps it"
    );
    let sent = bodies(&seen);
    assert_eq!(sent.len(), 2);
    assert_eq!(
        sent.first().and_then(|body| body.get("temperature")),
        Some(&json!(2))
    );
    assert!(
        sent.get(1)
            .and_then(|body| body.get("temperature"))
            .is_none()
    );
    for body in &sent {
        assert_eq!(body.get("model"), Some(&json!("gpt-3.5-turbo")));
        assert!(body.get("drop_params").is_none());
    }
    let (listener, base) = listen().await;
    let chat = executor(&base);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let request = ChatRequest::parse(
        br#"{"model":"g","temperature":2,"messages":[{"role":"user","content":"hi"}]}"#,
    )
    .expect("request");
    let rejected = String::from(
        r#"{"error":{"message":"{\"detail\":[{\"loc\":[\"body\",\"temperature\"],\"msg\":\"unsupported value\"}]}","type":"invalid_request_error"}}"#,
    );
    let replies = vec![Reply {
        status: 422,
        body: rejected,
    }];
    let ((), answered) = tokio::join!(
        serve(listener, replies, Arc::clone(&seen)),
        chat.send(chat.deployments.first().expect("deployment"), &request)
    );
    let Err(error) = answered else {
        panic!("a 422 without drop_params is not retried");
    };
    assert!(matches!(&error, ChatError::Provider { .. }), "{error}");
    assert_eq!(error.failure(&chat.redactor).status, 422);
    let sent = bodies(&seen);
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent.first().and_then(|body| body.get("temperature")),
        Some(&json!(2))
    );
    let (listener, base) = listen().await;
    let chat = executor(&base);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let request = ChatRequest::parse(
        br#"{"model":"g","temperature":2,"drop_params":true,"messages":[{"role":"user","content":"hi"}]}"#,
    )
    .expect("request");
    let rejected = String::from(
        r#"{"error":{"message":"{\"detail\":[{\"loc\":[\"body\",\"temperature\"],\"msg\":\"unsupported value\"}]}","type":"invalid_request_error"}}"#,
    );
    let replies = vec![
        Reply {
            status: 422,
            body: rejected.clone(),
        },
        Reply {
            status: 422,
            body: rejected,
        },
    ];
    let ((), answered) = tokio::join!(
        serve(listener, replies, Arc::clone(&seen)),
        chat.send(chat.deployments.first().expect("deployment"), &request)
    );
    let Err(error) = answered else {
        panic!("a second 422 becomes the provider's error");
    };
    assert!(matches!(&error, ChatError::Provider { .. }), "{error}");
    assert_eq!(error.failure(&chat.redactor).status, 422);
    let sent = bodies(&seen);
    assert_eq!(sent.len(), 2);
    assert!(
        sent.get(1)
            .and_then(|body| body.get("temperature"))
            .is_none()
    );
}

fn malformed_nested_error_failure() {
    let number = json!({"message": "42"});
    assert!(matches!(
        decode_rejected_fields(ErrorBody::Sdk(&number)),
        Err(RejectedFieldsError::MessageNotObject)
    ));
    let later_scalar = json!({"message": "{\"detail\": [{\"loc\": [\"body\", \"a\"]}, 5]}"});
    assert!(matches!(
        decode_rejected_fields(ErrorBody::Sdk(&later_scalar)),
        Err(RejectedFieldsError::DetailEntryNotObject { index: 1 })
    ));
    let listed = json!({"error": {"message": "[1]"}});
    let listed = serde_json::to_vec(&listed).expect("http body");
    assert!(matches!(
        decode_rejected_fields(ErrorBody::Http(&listed)),
        Err(RejectedFieldsError::MessageNotObject)
    ));
    assert!(matches!(
        decode_rejected_fields(ErrorBody::Http(b"not json")),
        Err(RejectedFieldsError::Json { .. })
    ));
    assert!(matches!(
        decode_rejected_fields(ErrorBody::Http(b"[1, 2]")),
        Err(RejectedFieldsError::ResponseNotObject)
    ));
}

mod raw_send;

#[derive(Clone, Copy)]
enum Scenario {
    TwoElementLoc,
    MalformedDetailKeepsFields,
    OutputLimitCaseInsensitive,
    EmptyLengthCompletion,
    SingleLengthStreamChunk,
    PromptTokenCount,
    UnrelatedError,
    OpenAiOutputLimitBuffered,
    OpenAiOutputLimitStream,
    OpenAi422DropRetry,
    MalformedNestedErrorFailure,
    AzureRawSendHeaders,
    AzureOutputLimitRecovery,
    AzureAsyncTimeout,
}

#[rstest]
#[case::two_element_loc(Scenario::TwoElementLoc)]
#[case::malformed_detail_keeps_fields(Scenario::MalformedDetailKeepsFields)]
#[case::output_limit_case_insensitive(Scenario::OutputLimitCaseInsensitive)]
#[case::empty_length_completion(Scenario::EmptyLengthCompletion)]
#[case::single_length_stream_chunk(Scenario::SingleLengthStreamChunk)]
#[case::prompt_token_count(Scenario::PromptTokenCount)]
#[case::unrelated_error(Scenario::UnrelatedError)]
#[case::openai_output_limit_buffered(Scenario::OpenAiOutputLimitBuffered)]
#[case::openai_output_limit_stream(Scenario::OpenAiOutputLimitStream)]
#[case::openai_422_drop_retry(Scenario::OpenAi422DropRetry)]
#[case::malformed_nested_error_failure(Scenario::MalformedNestedErrorFailure)]
#[case::azure_raw_send_headers(Scenario::AzureRawSendHeaders)]
#[case::azure_output_limit_recovery(Scenario::AzureOutputLimitRecovery)]
#[case::azure_async_timeout(Scenario::AzureAsyncTimeout)]
#[tokio::test]
async fn output_limits_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::TwoElementLoc => two_element_loc(),
        Scenario::MalformedDetailKeepsFields => malformed_detail_keeps_fields(),
        Scenario::OutputLimitCaseInsensitive => output_limit_case_insensitive(),
        Scenario::EmptyLengthCompletion => empty_length_completion().await,
        Scenario::SingleLengthStreamChunk => single_length_stream_chunk().await,
        Scenario::PromptTokenCount => prompt_token_count().await,
        Scenario::UnrelatedError => unrelated_error().await,
        Scenario::OpenAiOutputLimitBuffered => openai_output_limit_buffered().await,
        Scenario::OpenAiOutputLimitStream => openai_output_limit_stream().await,
        Scenario::OpenAi422DropRetry => openai_422_drop_retry().await,
        Scenario::MalformedNestedErrorFailure => malformed_nested_error_failure(),
        Scenario::AzureRawSendHeaders => raw_send::azure_raw_send_headers().await,
        Scenario::AzureOutputLimitRecovery => raw_send::azure_output_limit_recovery().await,
        Scenario::AzureAsyncTimeout => raw_send::azure_async_timeout().await,
    }
}

fn replays(path: &Path) -> bool {
    let bytes = std::fs::read(path).expect("corpus seed");
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("seed name")
        .to_owned();
    let decoded = decode_rejected_fields(ErrorBody::Http(&bytes));
    match name.split('-').next().expect("seed category") {
        "valid" | "boundary" => decoded.is_ok(),
        "malformed" | "truncated" => decoded.is_err(),
        other => panic!("unknown corpus category {other}"),
    }
}

#[test]
fn replays_the_openai_output_limits_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/openai_output_limits");
    let seeds: Vec<PathBuf> = std::fs::read_dir(corpus)
        .expect("corpus directory")
        .map(|entry| entry.expect("seed").path())
        .filter(|path| path.is_file())
        .collect();
    assert!(!seeds.is_empty());
    assert_eq!(seeds.iter().find(|path| !replays(path)), None);
}
