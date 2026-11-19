use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_error::envelope::{ErrorObject, ErrorResponse, WireField};
use serde::Serialize;

use super::scalars::NumericIssue;
use super::{CACHE_HIT_FILTER_PARAM, SORT_BY_PARAM, SORT_ORDER_PARAM, SPAN_TYPE_PARAM};

const BAD_REQUEST_TYPE: &str = "bad_request";
const BAD_REQUEST_CODE: &str = "400";
const CONVERTED_HTTP_ERROR_TYPE: &str = "internal_server_error";

#[derive(Debug, Clone, Serialize)]
pub(super) struct QueryIssue {
    #[serde(rename = "type")]
    pub(super) kind: &'static str,
    pub(super) loc: [&'static str; 2],
    pub(super) msg: String,
}

impl QueryIssue {
    pub(super) fn parsing(name: &'static str, issue: NumericIssue) -> Self {
        let (kind, msg) = issue.parts();
        Self {
            kind,
            loc: ["query", name],
            msg: msg.to_owned(),
        }
    }

    pub(super) fn below_minimum(name: &'static str, minimum: i64) -> Self {
        Self {
            kind: "greater_than_equal",
            loc: ["query", name],
            msg: format!("Input should be greater than or equal to {minimum}"),
        }
    }

    pub(super) fn above_maximum(name: &'static str, maximum: i64) -> Self {
        Self {
            kind: "less_than_equal",
            loc: ["query", name],
            msg: format!("Input should be less than or equal to {maximum}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueryIssues {
    pub(super) issues: Vec<QueryIssue>,
}

#[derive(Serialize)]
struct DetailBody {
    detail: Vec<QueryIssue>,
}

impl IntoResponse for QueryIssues {
    fn into_response(self) -> Response {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(DetailBody {
                detail: self.issues,
            }),
        )
            .into_response()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Spend,
    TotalTokens,
    StartTime,
    EndTime,
    RequestDurationMs,
    Model,
    TtftMs,
}

impl SortColumn {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "spend" => Some(Self::Spend),
            "total_tokens" => Some(Self::TotalTokens),
            "startTime" => Some(Self::StartTime),
            "endTime" => Some(Self::EndTime),
            "request_duration_ms" => Some(Self::RequestDurationMs),
            "model" => Some(Self::Model),
            "ttft_ms" => Some(Self::TtftMs),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

impl SortDirection {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value.to_lowercase().as_str() {
            "asc" => Some(Self::Ascending),
            "desc" => Some(Self::Descending),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheHitFilter {
    Hit,
    Miss,
}

impl CacheHitFilter {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "hit" => Some(Self::Hit),
            "miss" => Some(Self::Miss),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendLogChoices {
    pub sort_column: SortColumn,
    pub sort_direction: SortDirection,
    pub cache_hit: Option<CacheHitFilter>,
    pub span_condition: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChoiceError {
    #[error(
        "Invalid sort_by: {0}. Must be one of: endTime, model, request_duration_ms, spend, startTime, total_tokens, ttft_ms"
    )]
    SortBy(String),
    #[error("Invalid sort_order: {0}. Must be one of: asc, desc")]
    SortOrder(String),
    #[error("Invalid cache_hit_filter: {0}. Must be one of: hit, miss")]
    CacheHitFilter(String),
    #[error("Invalid span_type: {0}. Must be one of: llm, agent, mcp, batch")]
    SpanType(String),
}

impl ChoiceError {
    pub(super) const fn parameter(&self) -> &'static str {
        match self {
            Self::SortBy(_) => SORT_BY_PARAM,
            Self::SortOrder(_) => SORT_ORDER_PARAM,
            Self::CacheHitFilter(_) => CACHE_HIT_FILTER_PARAM,
            Self::SpanType(_) => SPAN_TYPE_PARAM,
        }
    }
}

impl IntoResponse for ChoiceError {
    fn into_response(self) -> Response {
        proxy_bad_request(self.to_string(), self.parameter())
    }
}

#[must_use]
pub(super) fn proxy_bad_request(message: String, parameter: &str) -> Response {
    proxy_envelope(
        message,
        WireField::Text(BAD_REQUEST_TYPE.to_owned()),
        WireField::Text(parameter.to_owned()),
    )
}

// handle_exception_on_proxy turns the raised HTTPException into internal_server_error with a null param.
#[must_use]
pub(super) fn proxy_converted_http_error(message: String) -> Response {
    proxy_envelope(
        message,
        WireField::Text(CONVERTED_HTTP_ERROR_TYPE.to_owned()),
        WireField::Null,
    )
}

fn proxy_envelope(message: String, error_type: WireField, parameter: WireField) -> Response {
    let body = ErrorResponse {
        error: ErrorObject {
            message,
            error_type,
            param: parameter,
            code: WireField::Text(BAD_REQUEST_CODE.to_owned()),
        },
    };
    (StatusCode::BAD_REQUEST, Json(body)).into_response()
}
