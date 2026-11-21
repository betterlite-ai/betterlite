use std::time::Duration;

use betterlite_foundry::rejected_fields::MAX_ATTEMPTS;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::controls::call_params::DropParams;
use http::{HeaderMap, Method};
use tokio::net::TcpListener;
use url::Url;

use super::{client, deployment, serve};
use crate::foundry_dispatch::{AzureAi, AzureAiPrepared};
use crate::operation::OperationError;
use crate::prepared::{PreparedChat, PreparedHttp, PreparedRequest, Upstream};
use crate::transform_retry::{TransformOnUnprocessable, TransformRetry, send_with_retry};

const CHAT: &str = r#"{"model":"deployment","messages":[{"role":"user","content":"hi"}],"temperature":2,"top_p":0.5}"#;
const TOOLS_CHAT: &str = r#"{"model":"deployment","messages":[{"role":"user","content":"hi"}],"tools":[{"type":"function","extra":1},{"type":"function","extra":2}]}"#;
const REJECTED_TEMPERATURE: &str = r#"{"error":{"message":"{\"detail\":[{\"loc\":[\"body\",\"temperature\"],\"type\":\"extra_forbidden\"}],\"msg\":\"Extra inputs are not permitted\"}","type":"invalid_request_error"}}"#;
const EXTRA_INPUTS_ONLY: &str =
    r#"{"error":{"message":"Extra inputs are not permitted","type":"invalid_request_error"}}"#;
const TOOL_LEVEL: &str = r#"{"error":{"message":"Extra inputs are not permitted: tools[0].extra","type":"invalid_request_error"}}"#;
const INVALID_INDEX: &str = r#"{"error":{"message":"unknown field: parameter index is not a valid field","type":"invalid_request_error"}}"#;
const UNRELATED: &str =
    r#"{"error":{"message":"a field the model does not know","type":"invalid_request_error"}}"#;

fn chat(url: Url, body: &str, retry: TransformRetry) -> PreparedRequest {
    let mut headers = HeaderMap::new();
    headers.insert("authorization", "Bearer fixture".parse().unwrap());
    PreparedRequest::Chat(Box::new(PreparedChat {
        http: PreparedHttp {
            method: Method::POST,
            url,
            headers,
            body: body.as_bytes().to_vec(),
            deadline: Duration::from_secs(5),
            body_limit: None,
        },
        retry,
        client_model: None,
    }))
}

fn error_response(body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status: 422,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

#[tokio::test]
async fn a_rejected_temperature_is_resent_without_it() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let client = client();
    let request = chat(url, CHAT, TransformRetry::bounded(MAX_ATTEMPTS).unwrap());
    let hook = AzureAi;
    let mut inputs = deployment();
    inputs.global_drop_params = true;
    let (bodies, answer) = tokio::join!(
        serve(
            listener,
            vec![(422, REJECTED_TEMPERATURE), (422, REJECTED_TEMPERATURE)]
        ),
        send_with_retry(&client, &request, &inputs, &hook)
    );
    let Upstream::Buffered(response) = answer.unwrap() else {
        panic!("a plain buffered chat answers buffered");
    };
    assert_eq!(response.status, 422);
    assert_eq!(
        bodies,
        vec![
            CHAT.to_owned(),
            String::from(
                r#"{"messages":[{"content":"hi","role":"user"}],"model":"deployment","top_p":0.5}"#
            )
        ]
    );
}

#[tokio::test]
async fn a_tool_level_field_needs_no_drop_policy() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let client = client();
    let request = chat(
        url,
        TOOLS_CHAT,
        TransformRetry::bounded(MAX_ATTEMPTS).unwrap(),
    );
    let hook = AzureAi;
    let inputs = deployment();
    let (bodies, answer) = tokio::join!(
        serve(listener, vec![(422, TOOL_LEVEL), (422, TOOL_LEVEL)]),
        send_with_retry(&client, &request, &inputs, &hook)
    );
    let Upstream::Buffered(response) = answer.unwrap() else {
        panic!("a plain buffered chat answers buffered");
    };
    assert_eq!(response.status, 422);
    assert_eq!(bodies.len(), 2);
    assert!(!bodies.get(1).unwrap().contains("\"extra\""));
}

#[tokio::test]
async fn an_unrelated_error_is_sent_once() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let client = client();
    let request = chat(url, CHAT, TransformRetry::bounded(MAX_ATTEMPTS).unwrap());
    let hook = AzureAi;
    let inputs = deployment();
    let (bodies, answer) = tokio::join!(
        serve(listener, vec![(422, UNRELATED)]),
        send_with_retry(&client, &request, &inputs, &hook)
    );
    let Upstream::Buffered(response) = answer.unwrap() else {
        panic!("a plain buffered chat answers buffered");
    };
    assert_eq!(response.status, 422);
    assert_eq!(bodies, vec![CHAT.to_owned()]);
}

#[tokio::test]
async fn a_body_that_cannot_be_rewritten_fails_the_call() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let client = client();
    let request = chat(
        url,
        r#"{"model":"deployment","messages":"hi"}"#,
        TransformRetry::bounded(MAX_ATTEMPTS).unwrap(),
    );
    let hook = AzureAi;
    let inputs = deployment();
    let (bodies, answer) = tokio::join!(
        serve(listener, vec![(422, INVALID_INDEX)]),
        send_with_retry(&client, &request, &inputs, &hook)
    );
    assert!(matches!(answer, Err(OperationError::Encode { .. })));
    assert_eq!(bodies.len(), 1);
}

#[tokio::test]
async fn an_error_that_cannot_be_read_fails_the_call() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let client = client();
    let request = chat(
        url,
        r#"{"model":"deployment","messages":null}"#,
        TransformRetry::bounded(MAX_ATTEMPTS).unwrap(),
    );
    let hook = AzureAi;
    let inputs = deployment();
    let (bodies, answer) = tokio::join!(
        serve(
            listener,
            vec![(422, "unknown field: parameter index is not a valid field")]
        ),
        send_with_retry(&client, &request, &inputs, &hook)
    );
    assert!(matches!(answer, Err(OperationError::RejectedFields { .. })));
    assert_eq!(bodies.len(), 1);
}

#[test]
fn the_repair_leaves_the_url_headers_and_deadline_alone() {
    let url = Url::parse("https://studio.example.test/chat/completions").unwrap();
    let hook = AzureAi;
    let mut request = chat(
        url.clone(),
        CHAT,
        TransformRetry::bounded(MAX_ATTEMPTS).unwrap(),
    );
    let PreparedRequest::Chat(before) = &request else {
        panic!("the fixture is a chat request");
    };
    let headers = before.http.headers.clone();
    let deadline = before.http.deadline;
    let mut inputs = deployment();
    inputs.global_drop_params = true;
    hook.transform_request(&mut request, &error_response(REJECTED_TEMPERATURE), &inputs)
        .unwrap();
    let PreparedRequest::Chat(after) = &request else {
        panic!("the fixture stays a chat request");
    };
    assert_eq!(after.http.url, url);
    assert_eq!(after.http.headers, headers);
    assert_eq!(after.http.deadline, deadline);
    assert_eq!(
        after.http.body,
        br#"{"messages":[{"content":"hi","role":"user"}],"model":"deployment","top_p":0.5}"#
            .to_vec()
    );
}

#[test]
fn the_hook_reads_both_drop_params_flags() {
    let hook = AzureAi;
    let response = error_response(EXTRA_INPUTS_ONLY);
    assert!(!hook.should_retry(&response, &deployment()));
    let mut global = deployment();
    global.global_drop_params = true;
    assert!(hook.should_retry(&response, &global));
    let mut per_call = deployment();
    per_call.drop_params = Some(DropParams::Flag(true));
    assert!(hook.should_retry(&response, &per_call));
    let mut text = deployment();
    text.drop_params = Some(DropParams::Text(String::from("yes")));
    assert!(hook.should_retry(&response, &text));
    let mut off = deployment();
    off.drop_params = Some(DropParams::Flag(false));
    assert!(!hook.should_retry(&response, &off));
    // a tool-level field retries without either flag
    assert!(hook.should_retry(&error_response(TOOL_LEVEL), &deployment()));
}

#[test]
fn an_embedding_body_is_repaired_too() {
    let hook = AzureAi;
    let mut request = PreparedRequest::Embedding(Box::new(PreparedChat {
        http: PreparedHttp {
            method: Method::POST,
            url: Url::parse("https://studio.example.test/embeddings").unwrap(),
            headers: HeaderMap::new(),
            body: br#"{"model":"deployment","input":"hi","temperature":2}"#.to_vec(),
            deadline: Duration::from_secs(5),
            body_limit: None,
        },
        retry: TransformRetry::bounded(MAX_ATTEMPTS).unwrap(),
        client_model: None,
    }));
    let mut inputs = deployment();
    inputs.global_drop_params = true;
    hook.transform_request(&mut request, &error_response(REJECTED_TEMPERATURE), &inputs)
        .unwrap();
    let PreparedRequest::Embedding(prepared) = &request else {
        panic!("the fixture stays an embedding request");
    };
    assert_eq!(
        prepared.http.body,
        br#"{"input":"hi","model":"deployment"}"#.to_vec()
    );
}

#[test]
fn the_repair_leaves_other_prepared_requests_alone() {
    let hook = AzureAi;
    let mut request = PreparedRequest::AzureAi(AzureAiPrepared);
    hook.transform_request(&mut request, &error_response(TOOL_LEVEL), &deployment())
        .unwrap();
}
