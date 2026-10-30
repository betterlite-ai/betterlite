use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use axum::{Json, Router};
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::embedding::{EmbeddingError, EmbeddingRequest};
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

use super::chat::headers::{failure_headers, success_headers};

#[derive(Serialize)]
struct ErrorBody {
    error: ErrorObject,
}

#[derive(Serialize)]
struct ErrorObject {
    message: String,
    #[serde(rename = "type")]
    error_type: String,
    param: Option<String>,
    code: String,
}

pub(crate) fn embedding_routes(executor: Arc<ChatExecutor>) -> Router {
    let router = Router::new()
        .route("/embeddings", post(embeddings))
        .route("/v1/embeddings", post(embeddings))
        .route("/engines/{model}/embeddings", post(embeddings))
        .route("/openai/deployments/{model}/embeddings", post(embeddings))
        .with_state(executor);
    router
}

async fn embeddings(
    State(executor): State<Arc<ChatExecutor>>,
    path_model: Option<Path<String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let started = Instant::now();
    let call_id = headers
        .get("x-litellm-call-id")
        .and_then(|value| value.to_str().ok())
        .map_or_else(|| Uuid::new_v4().to_string(), str::to_owned);
    let request = match EmbeddingRequest::parse(&body) {
        Ok(request) => request,
        Err(error) => return failure(&error, &call_id, None),
    };
    let model = path_model
        .map(|Path(model)| model)
        .or_else(|| request.model.clone())
        .unwrap_or_else(|| String::from("None"));
    let deployment = match executor.pick(&model) {
        Ok(deployment) => deployment,
        Err(_) => return failure(&EmbeddingError::UnknownModel { model }, &call_id, None),
    };
    match executor.send_embedding(deployment, &request).await {
        Ok(outcome) => {
            let response_headers = success_headers(
                &outcome.upstream.headers,
                deployment,
                &call_id,
                outcome.upstream_time,
                started.elapsed(),
            );
            (response_headers, Json(outcome.response)).into_response()
        }
        Err(error) => failure(&error, &call_id, Some(deployment)),
    }
}

fn failure(error: &EmbeddingError, call_id: &str, deployment: Option<&ChatDeployment>) -> Response {
    let (status, error_type) = match error {
        EmbeddingError::Provider { response } => (response.status, "invalid_request_error"),
        EmbeddingError::UnknownModel { .. } | EmbeddingError::Body { .. } => {
            (400, "invalid_request_error")
        }
        EmbeddingError::MissingApiKey => (401, "authentication_error"),
        EmbeddingError::UnsupportedProvider { .. } => (501, "internal_server_error"),
        EmbeddingError::Http { .. }
        | EmbeddingError::Decode { .. }
        | EmbeddingError::Encode { .. } => (500, "internal_server_error"),
    };
    let body = ErrorBody {
        error: ErrorObject {
            message: error.to_string(),
            error_type: error_type.to_owned(),
            param: None,
            code: status.to_string(),
        },
    };
    let chat_error = betterlite_openai::chat::execution::ChatError::UnknownModel {
        model: String::new(),
    };
    let response_headers = failure_headers(&chat_error, call_id, deployment);
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, response_headers, Json(body)).into_response()
}
