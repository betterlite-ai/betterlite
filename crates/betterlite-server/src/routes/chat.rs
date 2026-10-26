mod headers;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use axum::{Json, Router};
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::{ChatError, ChatExecutor};
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

use headers::{failure_headers, success_headers};

// LiteLLM's MAX_LITELLM_CALL_ID_LENGTH
const MAX_CALL_ID_LENGTH: usize = 256;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_specific_fields: Option<RouterErrorFields>,
}

// LiteLLM's router repeats its message here when no deployment matches
#[derive(Serialize)]
struct RouterErrorFields {
    error: String,
}

pub(crate) fn chat_routes(executor: Arc<ChatExecutor>) -> Router {
    Router::new()
        .route("/chat/completions", post(chat_completion))
        .route("/v1/chat/completions", post(chat_completion))
        .route("/engines/{model}/chat/completions", post(chat_completion))
        .route(
            "/openai/deployments/{model}/chat/completions",
            post(chat_completion),
        )
        .with_state(executor)
}

async fn chat_completion(
    State(executor): State<Arc<ChatExecutor>>,
    path_model: Option<Path<String>>,
    request_headers: HeaderMap,
    body: Bytes,
) -> Response {
    let started = Instant::now();
    let call_id = request_headers
        .get("x-litellm-call-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| (1..=MAX_CALL_ID_LENGTH).contains(&value.len()))
        .map_or_else(|| Uuid::new_v4().to_string(), str::to_owned);
    let request = match ChatRequest::parse(&body) {
        Ok(request) => request,
        Err(source) => return failure(&ChatError::from(source), &call_id, None),
    };
    // LiteLLM lets the model in the path win over the one in the body
    let model = path_model.map(|Path(model)| model).or_else(|| request.model.clone());
    let deployment = match executor.pick(model.as_deref().unwrap_or("None")) {
        Ok(deployment) => deployment,
        Err(error) => return failure(&error, &call_id, None),
    };
    match executor.send(deployment, &request).await {
        Ok(outcome) => {
            let headers = success_headers(&outcome, deployment, &call_id, started.elapsed());
            (headers, Json(outcome.response)).into_response()
        }
        Err(error) => failure(&error, &call_id, Some(deployment)),
    }
}

fn failure(error: &ChatError, call_id: &str, deployment: Option<&ChatDeployment>) -> Response {
    let failure = error.failure();
    let status = StatusCode::from_u16(failure.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let message = error.to_string();
    let provider_specific_fields = matches!(error, ChatError::UnknownModel { .. }).then(|| {
        RouterErrorFields {
            error: message.clone(),
        }
    });
    let body = ErrorBody {
        error: ErrorObject {
            message,
            error_type: failure.error_type,
            param: failure.param,
            code: status.as_u16().to_string(),
            provider_specific_fields,
        },
    };
    let headers = failure_headers(error, call_id, deployment);
    (status, headers, Json(body)).into_response()
}

mod tests;
