#![cfg(test)]

use axum::http::StatusCode;
use betterlite_http::proxy::limits::ReadError;
use betterlite_prompts::arguments::json::{Json, Object};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Serialize;

use crate::purview_graph_client::error::PurviewGraphError;
use crate::purview_graph_client::graph_api::{MAX_RESPONSE_BYTES, graph_post};
use crate::purview_graph_client::test_support::{
    MockReply, RecordedRequest, guardrail, json_reply, standard_reply, start,
};

#[derive(Serialize)]
struct Probe {
    probe: &'static str,
}

fn answer_with_etag(request: &RecordedRequest) -> MockReply {
    match request.path.as_str() {
        "/probe" => MockReply {
            status: StatusCode::OK,
            headers: vec![("etag", "\"abc\"")],
            body: r#"{"answer":"yes"}"#.to_owned(),
        },
        _ => standard_reply(request),
    }
}

#[tokio::test]
async fn graph_api_returns_the_json_and_the_response_headers() {
    let mock = start(answer_with_etag).await;
    let base = guardrail(&mock.base);
    let url = format!("{}/probe", mock.base);

    let (json, headers): (Object, HeaderMap) =
        graph_post(&base, &url, &Probe { probe: "x" }, &HeaderMap::new())
            .await
            .unwrap();

    assert_eq!(json.get("answer"), Some(&Json::Text("yes".to_owned())));
    assert_eq!(headers.get("etag").unwrap(), "\"abc\"");
}

#[tokio::test]
async fn graph_api_sends_the_bearer_token_json_and_extra_headers() {
    let mock = start(standard_reply).await;
    let base = guardrail(&mock.base);
    let url = format!("{}/probe", mock.base);
    let mut extra = HeaderMap::new();
    extra.insert("if-none-match", HeaderValue::from_static("\"tag\""));

    graph_post::<_, Object>(&base, &url, &Probe { probe: "x" }, &extra)
        .await
        .unwrap();

    let requests = mock.requests.lock().unwrap().clone();
    let request = requests.last().unwrap();
    assert_eq!(request.path, "/probe");
    assert_eq!(request.headers.get("authorization").unwrap(), "Bearer tok");
    assert_eq!(
        request.headers.get("content-type").unwrap(),
        "application/json"
    );
    assert_eq!(request.headers.get("if-none-match").unwrap(), "\"tag\"");
    assert_eq!(request.body, r#"{"probe":"x"}"#);
}

fn throttled(request: &RecordedRequest) -> MockReply {
    if request.path.ends_with("/token") {
        return standard_reply(request);
    }
    MockReply {
        status: StatusCode::TOO_MANY_REQUESTS,
        headers: vec![("retry-after", "17")],
        body: "{}".to_owned(),
    }
}

#[tokio::test]
async fn graph_api_keeps_the_status_and_retry_after_of_a_failed_call() {
    let mock = start(throttled).await;
    let base = guardrail(&mock.base);
    let url = format!("{}/probe", mock.base);

    let error = graph_post::<_, Object>(&base, &url, &Probe { probe: "x" }, &HeaderMap::new())
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        PurviewGraphError::Status { status, retry_after }
            if status.as_u16() == 429 && retry_after.as_deref() == Some("17")
    ));
}

fn not_json(request: &RecordedRequest) -> MockReply {
    if request.path.ends_with("/token") {
        return standard_reply(request);
    }
    json_reply("[1, 2]")
}

#[tokio::test]
async fn graph_api_refuses_an_answer_that_is_not_a_json_object() {
    let mock = start(not_json).await;
    let base = guardrail(&mock.base);
    let url = format!("{}/probe", mock.base);

    let error = graph_post::<_, Object>(&base, &url, &Probe { probe: "x" }, &HeaderMap::new())
        .await
        .unwrap_err();

    assert!(matches!(error, PurviewGraphError::ResponseJson { .. }));
}

#[tokio::test]
async fn graph_api_does_not_call_graph_when_the_token_request_fails() {
    let mock = start(throttled_token).await;
    let base = guardrail(&mock.base);
    let url = format!("{}/probe", mock.base);

    let error = graph_post::<_, Object>(&base, &url, &Probe { probe: "x" }, &HeaderMap::new())
        .await
        .unwrap_err();

    assert!(matches!(error, PurviewGraphError::TokenStatus { .. }));
    assert_eq!(mock.count("/probe"), 0);
}

fn oversized(request: &RecordedRequest) -> MockReply {
    if request.path.ends_with("/token") {
        return standard_reply(request);
    }
    let padding = "x".repeat(MAX_RESPONSE_BYTES);
    json_reply(&format!("{{\"pad\":\"{padding}\"}}"))
}

#[tokio::test]
async fn graph_api_refuses_an_answer_over_the_size_limit() {
    let mock = start(oversized).await;
    let base = guardrail(&mock.base);
    let url = format!("{}/probe", mock.base);

    let error = graph_post::<_, Object>(&base, &url, &Probe { probe: "x" }, &HeaderMap::new())
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        PurviewGraphError::Body { source: ReadError::TooLarge { limit } }
            if limit == MAX_RESPONSE_BYTES
    ));
}

fn throttled_token(_request: &RecordedRequest) -> MockReply {
    MockReply {
        status: StatusCode::FORBIDDEN,
        headers: Vec::new(),
        body: "{}".to_owned(),
    }
}
