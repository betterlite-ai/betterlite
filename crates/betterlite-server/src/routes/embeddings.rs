use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use axum::{Json, Router};
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::{ChatError, ChatExecutor};
use betterlite_openai::chat::provider_error::{ProviderFailure, classify};
use betterlite_openai::embedding::{self, EmbeddingError, EmbeddingRequest};
use serde::Serialize;
use std::str;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

use super::chat::headers::{failure_headers, success_headers};

mod tests;

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

#[derive(serde::Deserialize)]
struct ProviderErrorBody {
    error: ProviderErrorFields,
}

#[derive(serde::Deserialize)]
struct ProviderErrorFields {
    message: String,
    #[serde(rename = "type")]
    error_type: String,
    param: Option<String>,
    code: Option<String>,
}

pub(crate) fn embedding_routes(executor: Arc<ChatExecutor>) -> Router {
    let mut router = Router::new();
    router = router.route("/embeddings", post(embeddings));
    router = router.route("/v1/embeddings", post(embeddings));
    router = router.route("/engines/{model}/embeddings", post(embeddings));
    router = router.route("/openai/deployments/{model}/embeddings", post(embeddings));
    router.with_state(executor)
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
    let Ok(deployment) = executor.pick(&model) else {
        return failure(&EmbeddingError::UnknownModel { model }, &call_id, None);
    };
    match embedding::send(&executor, deployment, &request).await {
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
    let failure = embedding_failure(error);
    let message = match error {
        EmbeddingError::Provider { response } => {
            provider_message(response.status, &response.body, error)
        }
        EmbeddingError::Body { .. }
        | EmbeddingError::Encode { .. }
        | EmbeddingError::Decode { .. }
        | EmbeddingError::Http { .. }
        | EmbeddingError::UnknownModel { .. }
        | EmbeddingError::UnsupportedProvider { .. }
        | EmbeddingError::MissingApiKey
        | EmbeddingError::Endpoint { .. } => error.to_string(),
    };
    let body = ErrorBody {
        error: ErrorObject {
            message,
            error_type: failure.error_type,
            param: failure.param,
            code: failure.status.to_string(),
        },
    };
    let response_headers = failure_headers_for_embedding(error, call_id, deployment);
    let status_code =
        StatusCode::from_u16(failure.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status_code, response_headers, Json(body)).into_response()
}

fn embedding_failure(error: &EmbeddingError) -> ProviderFailure {
    match error {
        EmbeddingError::Provider { response } => classify(response.status, &response.body),
        EmbeddingError::UnknownModel { .. } | EmbeddingError::Body { .. } => ProviderFailure {
            status: 400,
            error_type: String::from("invalid_request_error"),
            param: None,
        },
        EmbeddingError::MissingApiKey => ProviderFailure {
            status: 401,
            error_type: String::from("authentication_error"),
            param: None,
        },
        EmbeddingError::UnsupportedProvider { .. } => ProviderFailure {
            status: 501,
            error_type: String::from("internal_server_error"),
            param: None,
        },
        EmbeddingError::Http { .. }
        | EmbeddingError::Decode { .. }
        | EmbeddingError::Encode { .. }
        | EmbeddingError::Endpoint { .. } => ProviderFailure {
            status: 500,
            error_type: String::from("internal_server_error"),
            param: None,
        },
    }
}

fn failure_headers_for_embedding(
    error: &EmbeddingError,
    call_id: &str,
    deployment: Option<&ChatDeployment>,
) -> HeaderMap {
    match error {
        EmbeddingError::Provider { response } => {
            failure_headers(None, Some(&response.headers), call_id, deployment)
        }
        EmbeddingError::Body { .. }
        | EmbeddingError::Encode { .. }
        | EmbeddingError::Decode { .. }
        | EmbeddingError::Http { .. }
        | EmbeddingError::UnknownModel { .. }
        | EmbeddingError::UnsupportedProvider { .. }
        | EmbeddingError::MissingApiKey
        | EmbeddingError::Endpoint { .. } => {
            let chat_error = ChatError::UnknownModel {
                model: String::new(),
            };
            failure_headers(Some(&chat_error), None, call_id, deployment)
        }
    }
}

fn provider_message(status: u16, body: &[u8], fallback: &EmbeddingError) -> String {
    if let Ok(parsed) = serde_json::from_slice::<ProviderErrorBody>(body) {
        let error = parsed.error;
        return format!(
            "Error code: {status} - {{'error': {{'message': '{}', 'type': '{}', 'param': {}, 'code': {}}}}}",
            error.message,
            error.error_type,
            error
                .param
                .map_or_else(|| String::from("None"), |value| format!("'{value}'")),
            error
                .code
                .map_or_else(|| String::from("None"), |value| format!("'{value}'")),
        );
    }
    fallback.to_string()
}
