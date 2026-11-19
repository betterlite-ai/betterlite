use betterlite_models::params::ParameterError;
use serde::Deserialize;

use crate::anthropic_dispatch::AnthropicResult;
use crate::anthropic_dispatch::native::tests::support::wire;
use crate::anthropic_dispatch::tests::gateway::support::{self, Harness, MODEL};
use crate::operation::{OperationError, OperationResult};

const CHAT_REPLY: &str = r#"{"id":"msg_gateway","type":"message","role":"assistant","model":"claude-3-haiku-20240307","content":[{"type":"text","text":"native answer"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":3,"output_tokens":2}}"#;
const ERROR_REPLY: &str =
    r#"{"type":"error","error":{"type":"invalid_request_error","message":"model is required"}}"#;
const USER: &str = r#"[{"role":"user","content":"hi"}]"#;

#[derive(Deserialize)]
struct Sent {
    model: String,
    messages: Vec<SentMessage>,
}

#[derive(Deserialize)]
struct SentMessage {
    content: SentContent,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SentContent {
    Text(String),
    Parts(Vec<SentPart>),
}

#[derive(Deserialize)]
struct SentPart {
    #[serde(rename = "type")]
    kind: String,
    source: SentSource,
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

pub(super) async fn http() {
    let mut harness = Harness::start(vec![buffered()]).await;
    let result = support::complete(&harness, support::chat(chat_body("")))
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
    let sent = &requests[0];
    assert_eq!(sent.method, "POST");
    assert_eq!(sent.path, "/v1/messages");
    assert_eq!(sent.header("x-api-key"), Some("sk-test"));
    assert_eq!(sent.header("anthropic-version"), Some("2023-06-01"));
    let body = sent_body(sent);
    assert_eq!(body.model, MODEL);
    let SentContent::Text(text) = &body.messages[0].content else {
        panic!("a string message is sent as a string block");
    };
    assert_eq!(text, "hi");
}

pub(super) async fn error() {
    let mut harness = Harness::start(vec![wire::Answer {
        status: 400,
        content_type: "application/json",
        body: String::from(ERROR_REPLY),
    }])
    .await;
    let result = support::complete(&harness, support::chat(chat_body(""))).await;
    let Err(OperationError::Provider { response }) = result else {
        panic!("the upstream 400 is the provider's own error");
    };
    assert_eq!(response.status, 400);
    let body = String::from_utf8_lossy(&response.body);
    assert!(body.contains("model is required"));
    assert_eq!(harness.requests().await.len(), 1);
}

pub(super) async fn unsupported_param() {
    let mut harness = Harness::start(Vec::new()).await;
    let result = support::complete(
        &harness,
        support::chat(chat_body(r#","logit_bias":{"1":2}"#)),
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
        support::chat(chat_body(r#","logit_bias":{"1":2},"drop_params":true"#)),
    )
    .await
    .expect("drop_params sends the call without the parameter");
    assert!(matches!(
        result,
        OperationResult::Anthropic(AnthropicResult::Reply(_))
    ));
    let requests = harness.requests().await;
    assert_eq!(requests.len(), 1);
    let text = String::from_utf8_lossy(&requests[0].body);
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
    let result = support::complete(&harness, support::chat(file_body(&url)))
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
    let SentContent::Parts(parts) = &body.messages[0].content else {
        panic!("the fetched file is sent as a content block");
    };
    assert_eq!(parts[0].kind, "document");
    assert_eq!(parts[0].source.kind, "base64");
    assert_eq!(parts[0].source.media_type, "application/pdf");
    assert_eq!(parts[0].source.data, "JVBERi0xLjQK");
}

pub(super) async fn remote_file_error() {
    let mut harness = Harness::start(vec![wire::Answer {
        status: 404,
        content_type: "text/plain",
        body: String::from("missing"),
    }])
    .await;
    let url = format!("{}/missing.pdf", harness.base);
    let result = support::complete(&harness, support::chat(file_body(&url))).await;
    let Err(OperationError::Provider { response }) = result else {
        panic!("a failed file fetch is the 400 LiteLLM raises for the message build");
    };
    assert_eq!(response.status, 400);
    assert!(String::from_utf8_lossy(&response.body).contains(&url));
    let requests = harness.requests().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
}
