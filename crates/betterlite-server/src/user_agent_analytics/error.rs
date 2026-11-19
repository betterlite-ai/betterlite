use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse as _;
use axum::response::Response;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AnalyticsError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("the analytics window is outside the supported calendar range")]
    Window,
    #[error("week offset {week_offset} is outside the supported range")]
    WeekOffset { week_offset: i64 },
}

#[derive(Debug, Serialize)]
pub(super) struct TextDetailBody {
    pub(super) detail: String,
}

#[derive(Debug, Serialize)]
struct NestedDetailBody {
    detail: NestedDetail,
}

#[derive(Debug, Serialize)]
struct NestedDetail {
    error: &'static str,
}

pub(super) fn db_not_connected() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(NestedDetailBody {
            detail: NestedDetail {
                error: CommonProxyErrors::DbNotConnectedError.as_str(),
            },
        }),
    )
        .into_response()
}

pub(super) fn query_failed(prefix: &str, source: &AnalyticsError) -> Response {
    tracing::warn!(error = %source, "the user agent analytics query failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(TextDetailBody {
            detail: format!("{prefix}: {source}"),
        }),
    )
        .into_response()
}
