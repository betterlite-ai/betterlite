#![cfg(test)]

use axum::body::Body;
use axum::body::to_bytes;
use axum::http::{HeaderMap, Request, StatusCode};
use betterlite_config::loader::ConfigLoader;
use betterlite_http::proxy::UpstreamResponse;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use std::collections::BTreeMap;
use std::sync::Arc;
use tower::ServiceExt as _;

use super::{EmbeddingError, embedding_routes, failure};
use crate::server::chat_setup::chat_executor;

#[derive(serde::Deserialize)]
struct ErrorEnvelope {
    error: ErrorFields,
}

#[derive(serde::Deserialize)]
struct ErrorFields {
    #[serde(rename = "type")]
    error_type: String,
    param: Option<String>,
    code: String,
}

#[test]
fn embedding_failure_returns_bad_request() {
    let error = EmbeddingError::Body {
        source: serde_json::from_str::<String>("not-json").expect_err("invalid JSON"),
    };
    let response = failure(&error, "embedding-test", None);
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response.headers().get("x-litellm-call-id").unwrap(),
        "embedding-test"
    );
}

#[tokio::test]
async fn embedding_provider_failure_preserves_message() {
    let error = EmbeddingError::Provider {
        response: Box::new(UpstreamResponse {
            status: 429,
            headers: HeaderMap::new(),
            body: br#"{"error":{"message":"upstream bad","type":"requests","param":"input"}}"#
                .to_vec(),
        }),
    };
    let response = failure(&error, "embedding-provider", None);
    let body = to_bytes(response.into_body(), 4096)
        .await
        .expect("error body");
    assert!(body.windows(12).any(|window| window == b"upstream bad"));
    let parsed: ErrorEnvelope = serde_json::from_slice(&body).expect("error JSON");
    assert_eq!(parsed.error.error_type, "throttling_error");
    assert_eq!(parsed.error.param, None);
}

#[rstest::rstest]
#[case::plain("/embeddings")]
#[case::versioned("/v1/embeddings")]
#[case::engines("/engines/embed/embeddings")]
#[case::deployments("/openai/deployments/embed/embeddings")]
#[tokio::test]
async fn embedding_route_rejects_malformed_body(#[case] path: &str) {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("model_list:\n")
    .expect("config");
    let executor =
        chat_executor(&config, &BTreeMap::new(), StdRng::seed_from_u64(1)).expect("executor");
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from("not-json"))
        .expect("request");
    let response = embedding_routes(Arc::new(executor))
        .oneshot(request)
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[rstest::rstest]
#[case::invalid(400, "invalid_request_error", 400, "invalid_request_error", None)]
#[case::rate_limit(429, "requests", 429, "throttling_error", None)]
#[case::model_missing(
    400,
    "invalid_request_error model_not_found",
    404,
    "invalid_request_error",
    None
)]
#[case::custom_type(400, "provider_custom", 400, "invalid_request_error", None)]
#[case::authentication(401, "invalid_api_key", 401, "authentication_error", None)]
#[tokio::test]
async fn embedding_error_classification(
    #[case] status: u16,
    #[case] provider_type: &str,
    #[case] expected_status: u16,
    #[case] expected_type: &str,
    #[case] expected_param: Option<&str>,
) {
    let mut headers = HeaderMap::new();
    headers.insert("x-request-id", "provider-call".parse().expect("header"));
    let error = EmbeddingError::Provider {
        response: Box::new(UpstreamResponse {
            status,
            headers,
            body: format!(
                r#"{{"error":{{"message":"failed","type":"{provider_type}","param":"input"}}}}"#
            )
            .into_bytes(),
        }),
    };
    let response = failure(&error, "embedding-error", None);
    assert_eq!(response.status().as_u16(), expected_status);
    assert_eq!(
        response.headers()["llm_provider-x-request-id"],
        "provider-call"
    );
    let body = to_bytes(response.into_body(), 4096).await.expect("body");
    let envelope: ErrorEnvelope = serde_json::from_slice(&body).expect("JSON");
    assert_eq!(envelope.error.error_type, expected_type);
    assert_eq!(envelope.error.param.as_deref(), expected_param);
}

#[derive(serde::Deserialize)]
struct MockEmbeddingReply {
    model: String,
    object: String,
    data: Vec<MockEmbeddingData>,
    usage: MockEmbeddingUsage,
}

#[derive(serde::Deserialize)]
struct MockEmbeddingData {
    object: String,
    embedding: Vec<f64>,
    index: u32,
}

#[derive(serde::Deserialize)]
struct MockEmbeddingUsage {
    #[serde(rename = "prompt_tokens")]
    prompt: u64,
    #[serde(rename = "completion_tokens")]
    completion: u64,
    #[serde(rename = "total_tokens")]
    total: u64,
}

#[tokio::test]
async fn mock_calls_behavior_tests_behavior() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(
        "model_list:\n  - {model_name: embed, litellm_params: {model: openai/embed, api_base: 'http://127.0.0.1:1', timeout: 0.1, mock_response: [0.1, 0.2]}}\n",
    )
    .expect("config");
    let executor =
        chat_executor(&config, &BTreeMap::new(), StdRng::seed_from_u64(1)).expect("executor");
    let request = Request::post("/v1/embeddings")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"model":"embed","input":"hello"}"#))
        .expect("request");
    let response = embedding_routes(Arc::new(executor))
        .oneshot(request)
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 8192).await.expect("body");
    let reply: MockEmbeddingReply = serde_json::from_slice(&body).expect("reply");
    assert_eq!(reply.model, "embed");
    assert_eq!(reply.object, "list");
    let record = reply.data.first().expect("embedding");
    assert_eq!(record.object, "embedding");
    assert_eq!(record.embedding, vec![0.1_f64, 0.2_f64]);
    assert_eq!(record.index, 0);
    assert_eq!(reply.usage.prompt, 10);
    assert_eq!(reply.usage.completion, 0);
    assert_eq!(reply.usage.total, 0);
}

#[rstest::rstest]
#[case::mock_deployment(true)]
#[case::plain_deployment(false)]
#[tokio::test]
async fn a_mock_testing_param_is_rejected_on_embeddings(#[case] mocked: bool) {
    let source = [
        "model_list:\n  - {model_name: embed, litellm_params: {model: openai/embed, api_base: 'http://127.0.0.1:1', api_key: k",
        if mocked { ", mock_response: [0.1]" } else { "" },
        "}}\n",
    ]
    .concat();
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&source)
    .expect("config");
    let executor =
        chat_executor(&config, &BTreeMap::new(), StdRng::seed_from_u64(1)).expect("executor");
    let request = Request::post("/v1/embeddings")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"model":"embed","input":"hello","mock_timeout":true}"#,
        ))
        .expect("request");
    let response = embedding_routes(Arc::new(executor))
        .oneshot(request)
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), 8192).await.expect("body");
    let envelope: ErrorEnvelope = serde_json::from_slice(&body).expect("error JSON");
    assert_eq!(envelope.error.error_type, "invalid_request_error");
    assert_eq!(envelope.error.code, "400");
}

#[tokio::test]
async fn a_mock_embedding_answers_with_the_stripped_model() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(
        "model_list:\n  - {model_name: emb, litellm_params: {model: openai/text-embedding-3-small, api_base: 'http://127.0.0.1:1', mock_response: [0.1]}}\n",
    )
    .expect("config");
    let executor =
        chat_executor(&config, &BTreeMap::new(), StdRng::seed_from_u64(1)).expect("executor");
    let request = Request::post("/openai/deployments/emb/embeddings")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"input":"hello"}"#))
        .expect("request");
    let response = embedding_routes(Arc::new(executor))
        .oneshot(request)
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 8192).await.expect("body");
    let reply: MockEmbeddingReply = serde_json::from_slice(&body).expect("reply");
    assert_eq!(reply.model, "text-embedding-3-small");
}
