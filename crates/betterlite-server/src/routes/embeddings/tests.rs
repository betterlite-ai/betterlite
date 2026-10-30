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
