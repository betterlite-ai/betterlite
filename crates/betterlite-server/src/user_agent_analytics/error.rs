use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse as _;
use axum::response::Response;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use chrono::ParseError;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AnalyticsError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("the analytics window is outside the supported calendar range")]
    Window,
    #[error("period offset {offset} is outside the supported range")]
    Offset { offset: i64 },
    #[error("the period bucket {bucket} is not an offset")]
    Bucket { bucket: String },
    #[error("the analytics totals exceed the supported range")]
    Overflow,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct QueryIssue {
    #[serde(rename = "type")]
    pub(super) kind: &'static str,
    pub(super) loc: [&'static str; 2],
    pub(super) msg: String,
}

#[derive(Debug)]
pub(super) struct QueryIssues {
    pub(super) issues: Vec<QueryIssue>,
}

#[derive(Debug, Serialize)]
pub(super) struct TextDetailBody {
    pub(super) detail: String,
}

#[derive(Debug, Serialize)]
struct IssuesBody {
    detail: Vec<QueryIssue>,
}

#[derive(Debug, Serialize)]
struct NestedDetailBody {
    detail: NestedDetail,
}

#[derive(Debug, Serialize)]
struct NestedDetail {
    error: &'static str,
}

impl IntoResponse for QueryIssues {
    fn into_response(self) -> Response {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(IssuesBody {
                detail: self.issues,
            }),
        )
            .into_response()
    }
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

pub(super) fn invalid_date(source: &ParseError) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(TextDetailBody {
            detail: format!("Invalid date format. Use YYYY-MM-DD: {source}"),
        }),
    )
        .into_response()
}
