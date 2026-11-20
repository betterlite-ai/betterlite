use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use thiserror::Error;

const ADMIN_VIEW_REQUIRED: &str = "Admin-only endpoint. Not allowed to access this.";
const NO_ADAPTIVE_ROUTER: &str = "No adaptive_router is configured on this proxy.";

#[derive(Debug, Error)]
pub(super) enum RootRouteError {
    #[error("the caller does not have admin view")]
    AdminViewRequired,
    #[error("no adaptive_router is configured on this proxy")]
    NoAdaptiveRouter,
}

#[derive(Serialize)]
struct DetailBody {
    detail: Detail,
}

#[derive(Serialize)]
struct Detail {
    error: &'static str,
}

impl IntoResponse for RootRouteError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::AdminViewRequired => (StatusCode::FORBIDDEN, ADMIN_VIEW_REQUIRED),
            Self::NoAdaptiveRouter => (StatusCode::NOT_FOUND, NO_ADAPTIVE_ROUTER),
        };
        (
            status,
            Json(DetailBody {
                detail: Detail { error: message },
            }),
        )
            .into_response()
    }
}
