use std::str::from_utf8;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::params::ParameterError;
use http::HeaderMap;
use serde::Deserialize;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use crate::anthropic_dispatch::Anthropic;
use crate::anthropic_dispatch::AnthropicResult;
use crate::anthropic_dispatch::base::Surface;
use crate::anthropic_dispatch::native::tests::support::wire;
use crate::anthropic_dispatch::tests::gateway::support::{self, Harness, MODEL};
use crate::operation::{OperationError, OperationRequest, OperationResult};
use crate::provider::Provider;

const CHAT_REPLY: &str = r#"{"id":"msg_gateway","type":"message","role":"assistant","model":"claude-3-haiku-20240307","content":[{"type":"text","text":"native answer"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":3,"output_tokens":2}}"#;
const ERROR_REPLY: &str =
    r#"{"type":"error","error":{"type":"invalid_request_error","message":"model is required"}}"#;
const USER: &str = r#"[{"role":"user","content":"hi"}]"#;
const OPUS: &str = "claude-opus-4-5";
// Long enough for the loopback to accept the call, short enough that the hanging read ends it.
const HANG: Duration = Duration::from_millis(300);

#[derive(Deserialize)]
struct Sent {
    model: String,
    messages: Vec<SentMessage>,
    output_config: Option<SentOutputConfig>,
    logprobs: Option<bool>,
}

#[derive(Deserialize)]
struct SentOutputConfig {
    effort: Option<String>,
}

#[derive(Deserialize)]
struct SentMessage {
    content: Vec<SentPart>,
}

#[derive(Deserialize)]
struct SentPart {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    source: Option<SentSource>,
}

#[derive(Deserialize)]
struct SentSource {
    #[serde(rename = "type")]
    kind: String,
    media_type: String,
    data: String,
}

fn buffered() -> wire::Answer {
    wire::Answer {
        status: 200,
        content_type: "application/json",
        body: String::from(CHAT_REPLY),
    }
}

fn chat_body(extra: &str) -> String {
    format!(r#"{{"model":"{MODEL}","messages":{USER}{extra}}}"#)
}

fn file_body(url: &str) -> String {
    format!(
        r#"{{"model":"{MODEL}","messages":[{{"role":"user","content":[{{"type":"file","file":{{"file_data":"{url}","format":"application/pdf"}}}}]}}]}}"#
    )
}

fn sent_body(request: &wire::Recorded) -> Sent {
    serde_json::from_slice(&request.body).expect("the upstream request body is a Messages request")
}

fn failure(status: u16, content_type: &'static str, body: &str) -> wire::Answer {
    wire::Answer {
        status,
        content_type,
        body: String::from(body),
    }
}

// The provider error both upstream-failure cases read: the status, the message and the headers it carries.
async fn provider_error(harness: &Harness, request: OperationRequest) -> (u16, String, HeaderMap) {
    let result = support::complete(harness, request).await;
    let Err(OperationError::Provider { response }) = result else {
        panic!("an upstream failure keeps the status the provider gave it");
    };
    let UpstreamResponse {
        status,
        headers,
        body,
    } = *response;
    let message = String::from_utf8(body).expect("the failure body is text");
    (status, message, headers)
}

pub(super) async fn http() {
    let mut harness = Harness::start(vec![buffered()]).await;
    let result = support::complete(&harness, support::chat(&chat_body("")))
        .await
        .expect("the buffered call answers");
    let OperationResult::Anthropic(AnthropicResult::Reply(reply)) = result else {
        panic!("the buffered call is a Messages reply");
    };
    assert_eq!(reply.model, MODEL);
    assert!(reply.id.starts_with("chatcmpl-"));
    assert_eq!(
        reply.choices[0].message.content.as_deref(),
        Some("native answer")
    );
    let shown = serde_json::to_string(&*reply).expect("the client view serializes");
    assert!(!shown.contains("_hidden_params"));
    let requests = harness.requests().await;
    assert_eq!(requests.len(), 1);
    assert_chat_request(&requests[0]);
}

// The request a plain chat call sends: the Messages endpoint, its auth, its version and its text block.
fn assert_chat_request(sent: &wire::Recorded) {
    assert_eq!(sent.method, "POST");
    assert_eq!(sent.path, "/v1/messages");
    assert_eq!(sent.header("x-api-key"), Some("sk-test"));
    assert_eq!(sent.header("anthropic-version"), Some("2023-06-01"));
    let effort_beta = sent
        .header("anthropic-beta")
        .is_some_and(|value| value.contains("effort-2025-11-24"));
    assert!(
        !effort_beta,
        "a call without effort asks for no effort beta"
    );
    let body = sent_body(sent);
    assert_eq!(body.model, MODEL);
    let part = &body.messages[0].content[0];
    assert_eq!(part.kind, "text");
    assert_eq!(part.text.as_deref(), Some("hi"));
}

pub(super) async fn error() {
    let mut harness = Harness::start(vec![failure(400, "application/json", ERROR_REPLY)]).await;
    let (status, body, headers) = provider_error(&harness, support::chat(&chat_body(""))).await;
    assert_eq!(status, 400);
    assert!(body.contains("model is required"));
    let content_type = headers
        .get("content-type")
        .and_then(|value| value.to_str().ok());
    assert_eq!(
        content_type,
        Some("application/json"),
        "the failure keeps the provider's headers"
    );
    assert_eq!(harness.requests().await.len(), 1);
}

pub(super) async fn unsupported_param() {
    let mut harness = Harness::start(Vec::new()).await;
    let result = support::complete(
        &harness,
        support::chat(&chat_body(r#","logit_bias":{"1":2}"#)),
    )
    .await;
    let Err(OperationError::Parameters {
        source: ParameterError::Unsupported { names, .. },
    }) = result
    else {
        panic!("an unsupported parameter is a Parameters error");
    };
    assert_eq!(names, vec![String::from("logit_bias")]);
    assert!(harness.requests().await.is_empty());
}

pub(super) async fn dropped_param() {
    let mut harness = Harness::start(vec![buffered()]).await;
    let result = support::complete(
        &harness,
        support::chat(&chat_body(r#","logit_bias":{"1":2},"drop_params":true"#)),
    )
    .await
    .expect("drop_params sends the call without the parameter");
    assert!(matches!(
        result,
        OperationResult::Anthropic(AnthropicResult::Reply(_))
    ));
    let requests = harness.requests().await;
    assert_eq!(requests.len(), 1);
    let text = from_utf8(&requests[0].body).expect("the request body is text");
    assert!(!text.contains("logit_bias"));
    assert!(!text.contains("drop_params"));
}

pub(super) async fn remote_file() {
    let mut harness = Harness::start(vec![
        wire::Answer {
            status: 200,
            content_type: "application/pdf",
            body: String::from("%PDF-1.4\n"),
        },
        buffered(),
    ])
    .await;
    let url = format!("{}/file.pdf", harness.base);
    let result = support::complete(&harness, support::chat(&file_body(&url)))
        .await
        .expect("the fetched file is a document block");
    assert!(matches!(
        result,
        OperationResult::Anthropic(AnthropicResult::Reply(_))
    ));
    let requests = harness.requests().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, "/file.pdf");
    assert_eq!(requests[1].method, "POST");
    let body = sent_body(&requests[1]);
    let part = &body.messages[0].content[0];
    assert_eq!(part.kind, "document");
    let source = part.source.as_ref().expect("the document source");
    assert_eq!(source.kind, "base64");
    assert_eq!(source.media_type, "application/pdf");
    assert_eq!(source.data, "JVBERi0xLjQK");
}

pub(super) async fn remote_file_error() {
    let mut harness = Harness::start(vec![failure(404, "text/plain", "missing")]).await;
    let url = format!("{}/missing.pdf", harness.base);
    let (status, body, _headers) = provider_error(&harness, support::chat(&file_body(&url))).await;
    assert_eq!(status, 400);
    assert!(body.contains(&url));
    let requests = harness.requests().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
}

// A buffered reply hands over the provider headers LiteLLM projects into hidden_params.
#[tokio::test]
async fn the_provider_headers_carry_the_reply_headers() {
    let harness = Harness::start(vec![buffered()]).await;
    let (_, upstream) = support::upstream(&harness, support::chat(&chat_body("")))
        .await
        .expect("the buffered call answers");
    let content_type = upstream
        .headers()
        .get("llm_provider-content-type")
        .and_then(|value| value.to_str().ok());
    assert_eq!(content_type, Some("application/json"));
}

// A stream hands over the wrapper's projected response headers.
#[tokio::test]
async fn the_provider_headers_carry_the_stream_headers() {
    let harness = Harness::start(vec![wire::Answer {
        status: 200,
        content_type: "text/event-stream",
        body: String::from("data: {\"type\":\"message_stop\"}\n\n"),
    }])
    .await;
    let body = chat_body(r#","stream":true"#);
    let (_, upstream) = support::upstream(&harness, support::stream(&body))
        .await
        .expect("the streaming call opens");
    let content_type = upstream
        .headers()
        .get("llm_provider-content-type")
        .and_then(|value| value.to_str().ok());
    assert_eq!(content_type, Some("text/event-stream"));
}

// The request's extra headers become outbound headers instead of body fields.
#[tokio::test]
async fn the_extra_headers_reach_the_upstream_request() {
    let mut harness = Harness::start(vec![buffered()]).await;
    let result = support::complete(
        &harness,
        support::chat(&chat_body(r#","extra_headers":{"x-custom":"yes"}"#)),
    )
    .await;
    assert!(matches!(
        result,
        Ok(OperationResult::Anthropic(AnthropicResult::Reply(_)))
    ));
    let requests = harness.requests().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].header("x-custom"), Some("yes"));
    let text = from_utf8(&requests[0].body).expect("the request body is text");
    assert!(!text.contains("extra_headers"));
}

// additional_drop_params takes the named parameter out and leaves the extra body alone.
#[tokio::test]
async fn the_additional_drop_params_take_the_named_parameter_out() {
    let mut harness = Harness::start(vec![buffered()]).await;
    let result = support::complete(
        &harness,
        support::chat(&chat_body(
            r#","logit_bias":{"1":2},"extra_body":{"x-extra":{"keep":1}},"additional_drop_params":["logit_bias"]"#,
        )),
    )
    .await
    .expect("the additional drop keeps the call");
    assert!(matches!(
        result,
        OperationResult::Anthropic(AnthropicResult::Reply(_))
    ));
    let requests = harness.requests().await;
    assert_eq!(requests.len(), 1);
    let text = from_utf8(&requests[0].body).expect("the request body is text");
    assert!(!text.contains("logit_bias"));
    assert!(text.contains("x-extra"));
}

// Naming extra_body in the drop list takes the whole extra body out.
#[tokio::test]
async fn the_extra_body_can_be_dropped_by_name() {
    let mut harness = Harness::start(vec![buffered()]).await;
    let result = support::complete(
        &harness,
        support::chat(&chat_body(
            r#","extra_body":{"x-extra":{"keep":1}},"additional_drop_params":["extra_body"]"#,
        )),
    )
    .await
    .expect("the drop list keeps the call");
    assert!(matches!(
        result,
        OperationResult::Anthropic(AnthropicResult::Reply(_))
    ));
    let requests = harness.requests().await;
    assert_eq!(requests.len(), 1);
    let text = from_utf8(&requests[0].body).expect("the request body is text");
    assert!(!text.contains("x-extra"));
}

// output_config is a provider field: it reaches the body and turns on the effort beta.
#[tokio::test]
async fn the_provider_field_reaches_the_body_with_its_beta() {
    let mut harness = Harness::start(vec![buffered()]).await;
    harness.deployment.model = String::from(OPUS);
    let body =
        format!(r#"{{"model":"{OPUS}","messages":{USER},"output_config":{{"effort":"high"}}}}"#);
    let result = support::complete(&harness, support::chat(&body)).await;
    assert!(matches!(
        result,
        Ok(OperationResult::Anthropic(AnthropicResult::Reply(_)))
    ));
    let requests = harness.requests().await;
    assert_eq!(requests.len(), 1);
    let sent = sent_body(&requests[0]);
    let effort = sent.output_config.and_then(|config| config.effort);
    assert_eq!(effort.as_deref(), Some("high"));
    let beta = requests[0]
        .header("anthropic-beta")
        .expect("the effort beta header");
    assert!(beta.contains("effort-2025-11-24"));
}

// allowed_openai_params forwards a name the Anthropic mapper does not map, as sent.
#[tokio::test]
async fn the_allowed_openai_param_reaches_the_body() {
    let mut harness = Harness::start(vec![buffered()]).await;
    let result = support::complete(
        &harness,
        support::chat(&chat_body(
            r#","logprobs":true,"allowed_openai_params":["logprobs"]"#,
        )),
    )
    .await;
    assert!(matches!(
        result,
        Ok(OperationResult::Anthropic(AnthropicResult::Reply(_)))
    ));
    let requests = harness.requests().await;
    assert_eq!(requests.len(), 1);
    let sent = sent_body(&requests[0]);
    assert_eq!(sent.logprobs, Some(true));
}

// A loopback that accepts and never answers ends the call as the operation timeout.
#[tokio::test]
async fn the_upstream_timeout_is_the_operation_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the hanging listener");
    let base = format!(
        "http://{}",
        listener.local_addr().expect("the hanging address")
    );
    let mut hanging = JoinSet::new();
    hanging.spawn(hold(listener));
    let client = support::client();
    let deployment = support::deployment(&client, &base, MODEL, HANG);
    let harness = Harness {
        base,
        client,
        deployment,
        server: JoinSet::new(),
    };
    let body = chat_body(r#","stream":true"#);
    let result = support::complete(&harness, support::stream(&body)).await;
    assert!(matches!(result, Err(OperationError::Timeout)));
}

// The accepted connections stay open, so the client's read runs into its deadline.
async fn hold(listener: TcpListener) -> Vec<TcpStream> {
    let mut sockets = Vec::new();
    while let Ok((socket, _peer)) = listener.accept().await {
        sockets.push(socket);
    }
    sockets
}

// The legacy surface refuses at every stage, even with a runtime on the deployment.
#[tokio::test]
async fn the_legacy_surface_answers_unavailable() {
    let mut harness = Harness::start(vec![buffered()]).await;
    harness.deployment.timeout = HANG;
    let operation = support::chat(&chat_body(""));
    let messages = Provider::Anthropic(Anthropic {
        surface: Surface::Messages,
    });
    let legacy = Provider::Anthropic(Anthropic {
        surface: Surface::Legacy,
    });
    let prepared = messages
        .prepare(&operation, &harness.deployment, &support::catalog())
        .expect("the messages surface prepares chat");
    let refused = legacy.prepare(&operation, &harness.deployment, &support::catalog());
    assert!(matches!(
        refused,
        Err(OperationError::UnavailableAdapter { .. })
    ));
    let upstream = messages
        .execute(&harness.client, &prepared, &harness.deployment)
        .await
        .expect("the messages surface sends");
    let sent = legacy
        .execute(&harness.client, &prepared, &harness.deployment)
        .await;
    assert!(matches!(
        sent,
        Err(OperationError::UnavailableAdapter { .. })
    ));
    let decoded = legacy.decode(&prepared, upstream);
    assert!(matches!(
        decoded,
        Err(OperationError::UnavailableAdapter { .. })
    ));
    assert_eq!(harness.requests().await.len(), 1);
}
