#![cfg(test)]

use betterlite_models::decisions::Json;
use secrecy::SecretString;

use super::{API_BASE_URL, Api, ApiError, HttpMethod, TRACING_ENDPOINT};
use crate::deepeval::deepeval_logging::test_support::{
    Answer, Recorded, Recording, RecordingError,
};
use crate::deepeval::deepeval_logging::types::{
    BaseApiSpan, Environment, SpanApiType, SpanValue, TraceApi, TraceSpanApiStatus,
};

fn trace() -> TraceApi {
    TraceApi {
        uuid: "trace-id".to_owned(),
        base_spans: Vec::new(),
        agent_spans: Vec::new(),
        llm_spans: vec![BaseApiSpan {
            uuid: "span-id".to_owned(),
            name: "litellm_success_callback".to_owned(),
            status: TraceSpanApiStatus::Success,
            span_type: SpanApiType::Llm,
            trace_uuid: "trace-id".to_owned(),
            start_time: "2024-05-01T10:00:00.120Z".to_owned(),
            end_time: "2024-05-01T10:00:01.500Z".to_owned(),
            input: Some(SpanValue::Text("hi".to_owned())),
            output: Some(SpanValue::Text("hello".to_owned())),
            model: Some("gpt-4o".to_owned()),
            input_token_count: Some(3),
            output_token_count: Some(5),
        }],
        retriever_spans: Vec::new(),
        tool_spans: Vec::new(),
        start_time: "2024-05-01T10:00:00.120Z".to_owned(),
        end_time: "2024-05-01T10:00:01.500Z".to_owned(),
        environment: Environment::Development,
    }
}

fn api<A>(answers: A) -> Api<Recording>
where
    A: IntoIterator<Item = Answer>,
{
    Api::new(
        &SecretString::from("confident-key"),
        Recording::with_answers(answers),
    )
    .unwrap()
}

fn recorded(delivery: &Recording) -> Vec<Recorded> {
    delivery.requests.lock().unwrap().clone()
}

#[tokio::test]
async fn api_posts_the_trace_to_the_confident_tracing_endpoint() {
    let api = api([Answer::status(200)]);
    api.send_request(HttpMethod::Post, &trace()).await.unwrap();
    let requests = recorded(&api.delivery);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, format!("{API_BASE_URL}{TRACING_ENDPOINT}"));
    assert_eq!(requests[0].url, "https://api.confident-ai.com/v1/tracing");
}

#[tokio::test]
async fn api_sends_the_confident_key_and_a_json_content_type() {
    let api = api([Answer::status(200)]);
    api.send_request(HttpMethod::Post, &trace()).await.unwrap();
    let requests = recorded(&api.delivery);
    assert_eq!(requests[0].headers["confident_api_key"], "confident-key");
    assert_eq!(requests[0].headers["content-type"], "application/json");
}

#[tokio::test]
async fn api_posts_the_serialized_trace_body() {
    let api = api([Answer::status(200)]);
    api.send_request(HttpMethod::Post, &trace()).await.unwrap();
    let requests = recorded(&api.delivery);
    let expected: Json = serde_json::from_str(&serde_json::to_string(&trace()).unwrap()).unwrap();
    assert_eq!(requests[0].body, expected);
}

#[tokio::test]
async fn send_request_returns_the_response_on_status_200() {
    let api = api([Answer::reply(200, r#"{"accepted": true}"#)]);
    let response = api.send_request(HttpMethod::Post, &trace()).await.unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.body, br#"{"accepted": true}"#.to_vec());
}

#[tokio::test]
async fn send_request_raises_with_the_body_on_a_non_200_status() {
    let api = api([Answer::reply(500, r#"{"error": "boom"}"#)]);
    let Err(ApiError::Status { status, body }) = api.send_request(HttpMethod::Post, &trace()).await
    else {
        panic!("a 500 must not be accepted");
    };
    assert_eq!(status, 500);
    assert_eq!(body, r#"{"error": "boom"}"#);
}

#[tokio::test]
async fn send_request_rejects_a_201_that_is_not_200() {
    let api = api([Answer::status(201)]);
    let Err(ApiError::Status { status, .. }) = api.send_request(HttpMethod::Post, &trace()).await
    else {
        panic!("a 201 must not be accepted by the sync send");
    };
    assert_eq!(status, 201);
}

#[tokio::test]
async fn send_request_reports_a_transport_failure() {
    let api = api([Answer::Fail]);
    assert!(matches!(
        api.send_request(HttpMethod::Post, &trace()).await,
        Err(ApiError::Transport(RecordingError))
    ));
}

#[tokio::test]
async fn send_request_rejects_a_method_other_than_post() {
    let api = api([]);
    assert!(matches!(
        api.send_request(HttpMethod::Get, &trace()).await,
        Err(ApiError::Method)
    ));
    assert!(recorded(&api.delivery).is_empty());
}

#[tokio::test]
async fn a_send_request_posts_the_trace_and_returns_nothing() {
    let api = api([Answer::status(200)]);
    api.a_send_request(HttpMethod::Post, &trace())
        .await
        .unwrap();
    let requests = recorded(&api.delivery);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, "https://api.confident-ai.com/v1/tracing");
}

#[tokio::test]
async fn a_send_request_accepts_a_2xx_status_without_rechecking_200() {
    let api = api([Answer::status(201)]);
    api.a_send_request(HttpMethod::Post, &trace())
        .await
        .unwrap();
}

#[tokio::test]
async fn a_send_request_raises_on_a_redirect() {
    let api = api([Answer::status(302)]);
    let Err(ApiError::Status { status, .. }) = api.a_send_request(HttpMethod::Post, &trace()).await
    else {
        panic!("a 302 must not be accepted");
    };
    assert_eq!(status, 302);
}

#[tokio::test]
async fn a_send_request_raises_on_a_client_error() {
    let api = api([Answer::status(401)]);
    let Err(ApiError::Status { status, .. }) = api.a_send_request(HttpMethod::Post, &trace()).await
    else {
        panic!("a 401 must not be accepted");
    };
    assert_eq!(status, 401);
}

#[tokio::test]
async fn a_send_request_raises_on_a_server_error_status() {
    let api = api([Answer::reply(500, "boom")]);
    let Err(ApiError::Status { status, body }) =
        api.a_send_request(HttpMethod::Post, &trace()).await
    else {
        panic!("a 500 must not be accepted");
    };
    assert_eq!(status, 500);
    assert_eq!(body, "boom");
}

#[tokio::test]
async fn a_send_request_rejects_a_method_other_than_post() {
    let api = api([]);
    assert!(matches!(
        api.a_send_request(HttpMethod::Delete, &trace()).await,
        Err(ApiError::Method)
    ));
}

#[test]
fn api_rejects_a_key_that_is_not_a_header_value() {
    let result = Api::new(&SecretString::from("bad\nkey"), Recording::default());
    assert!(matches!(result, Err(ApiError::Header(_))));
}
