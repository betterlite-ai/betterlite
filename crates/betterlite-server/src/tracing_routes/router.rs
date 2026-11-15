mod queries;
mod reads;

use axum::body::Body;
use axum::extract::{Extension, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse as _, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use betterlite_error::envelope::{ErrorObject, ErrorResponse, WireField};
use serde::{Deserialize, Serialize};

use crate::auth::auth_identity::AuthContext;
use crate::error_body::json_error;
use crate::tracing_routes::TraceRoutesState;
use crate::tracing_routes::access::{AccessError, TraceAuth, provide_trace_access};
use crate::tracing_routes::otlp::{otlp_encoded_response, otlp_retry_response};
use crate::tracing_routes::receiver::{IngestError, TraceReadError};

const MS_PER_DAY: i64 = 86_400_000;

#[derive(Serialize)]
pub(crate) struct DetailBody {
    pub(crate) detail: String,
}

#[derive(Serialize)]
pub(crate) struct ValidationIssue {
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
    pub(crate) loc: Vec<String>,
    pub(crate) msg: &'static str,
}

#[derive(Serialize)]
pub(crate) struct ValidationBody {
    pub(crate) detail: Vec<ValidationIssue>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TraceRefQuery {
    #[serde(default)]
    pub(crate) trace_ref: String,
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    start_ms: Option<i64>,
    end_ms: Option<i64>,
    cursor: Option<String>,
}

pub fn router(state: TraceRoutesState) -> Router {
    Router::new()
        .route("/v1/traces", post(ingest_traces).get(list_traces))
        .route("/v1/traces/query", post(queries::query_traces))
        .route("/v1/traces/query/help", get(queries::query_help))
        .route("/v1/traces/{trace_id}", get(reads::get_trace))
        .route(
            "/v1/traces/{trace_id}/spans/{span_id}",
            get(reads::get_span),
        )
        .route(
            "/v1/traces/{trace_id}/spans/{span_id}/error",
            get(reads::get_span_error),
        )
        .with_state(state)
}

async fn ingest_traces(
    State(state): State<TraceRoutesState>,
    caller: Option<Extension<AuthContext>>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let Some(Extension(context)) = caller else {
        return unauthenticated();
    };
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    let content_encoding = headers
        .get(header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok());
    let auth = TraceAuth::from_principal(&context.principal);
    let access = provide_trace_access(&auth, state.receiver.clone());
    let (receiver, tenant) = match access.writer() {
        Ok(pair) => pair,
        Err(error) => {
            return otlp_encoded_response(
                content_type,
                access_status(&error),
                Some(&error.to_string()),
            );
        }
    };
    match receiver
        .ingest(body, content_type, content_encoding, tenant)
        .await
    {
        Ok(_rows) => otlp_encoded_response(content_type, StatusCode::OK, None),
        Err(error) => ingest_failure(content_type, &error),
    }
}

async fn list_traces(
    State(state): State<TraceRoutesState>,
    caller: Option<Extension<AuthContext>>,
    uri: Uri,
) -> Response {
    let Some(Extension(context)) = caller else {
        return unauthenticated();
    };
    let Ok(Query(params)) = Query::<ListQuery>::try_from_uri(&uri) else {
        let body = DetailBody {
            detail: "invalid trace query parameters".to_owned(),
        };
        return (StatusCode::BAD_REQUEST, Json(body)).into_response();
    };
    let now_ms = state.clock.now_ms();
    let start_ms = params
        .start_ms
        .unwrap_or_else(|| now_ms.saturating_sub(MS_PER_DAY));
    let end_ms = params.end_ms.unwrap_or(now_ms);
    let auth = TraceAuth::from_principal(&context.principal);
    let access = provide_trace_access(&auth, state.receiver.clone());
    let (receiver, scope) = match access.reader() {
        Ok(pair) => pair,
        Err(error) => return access_failure(&error),
    };
    let cursor = params.cursor.as_deref();
    match receiver
        .list_traces(scope, (start_ms, end_ms), cursor)
        .await
    {
        Ok(page) => Json(page).into_response(),
        Err(error) => read_failure(&error),
    }
}

// LiteLLM's auth_error carries param "None" rather than null, which WireField::MISSING serializes
pub(crate) fn unauthenticated() -> Response {
    let body = ErrorResponse {
        error: ErrorObject {
            message: "the API key is not valid".to_owned(),
            error_type: WireField::Text("auth_error".to_owned()),
            param: WireField::MISSING,
            code: WireField::Text("401".to_owned()),
        },
    };
    (StatusCode::UNAUTHORIZED, Json(body)).into_response()
}

pub(crate) fn access_failure(error: &AccessError) -> Response {
    let status = access_status(error);
    let body = DetailBody {
        detail: error.to_string(),
    };
    (status, Json(body)).into_response()
}

pub(crate) const fn access_status(error: &AccessError) -> StatusCode {
    match error {
        AccessError::Unavailable => StatusCode::NOT_IMPLEMENTED,
        AccessError::NotAllowedToView | AccessError::NotAllowedToIngest => StatusCode::FORBIDDEN,
    }
}

pub(crate) fn read_failure(error: &TraceReadError) -> Response {
    match error {
        TraceReadError::InvalidCursor(message) => {
            let body = DetailBody {
                detail: message.clone(),
            };
            (StatusCode::BAD_REQUEST, Json(body)).into_response()
        }
        TraceReadError::Storage => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
            "internal_server_error",
            "500",
        ),
    }
}

fn ingest_failure(content_type: Option<&str>, error: &IngestError) -> Response {
    match error {
        IngestError::TooLarge(message) => {
            otlp_encoded_response(content_type, StatusCode::PAYLOAD_TOO_LARGE, Some(message))
        }
        IngestError::Invalid(message) => {
            otlp_encoded_response(content_type, StatusCode::BAD_REQUEST, Some(message))
        }
        IngestError::Overloaded | IngestError::BodyRead(_) | IngestError::Storage => {
            otlp_retry_response(
                content_type,
                StatusCode::SERVICE_UNAVAILABLE,
                "Trace ingestion is temporarily unavailable",
            )
        }
    }
}
