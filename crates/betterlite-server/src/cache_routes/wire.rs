use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use serde::Serialize;

#[derive(Serialize)]
pub(crate) struct DetailBody {
    pub(crate) detail: String,
}

#[derive(Serialize)]
pub(crate) struct SuccessBody {
    pub(crate) status: &'static str,
}

pub(crate) fn unhealthy(message: &str) -> Response {
    let body = DetailBody {
        detail: format!("Service Unhealthy ({message})"),
    };
    (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
}

// LiteLLM's /cache/delete blanket handler catches its own 503 and rewrites every failure to 500.
pub(crate) fn delete_failed(message: &str) -> Response {
    let body = DetailBody {
        detail: format!("Cache Delete Failed({message})"),
    };
    (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
}
