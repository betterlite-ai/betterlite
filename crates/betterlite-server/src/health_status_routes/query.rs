use std::num::{IntErrorKind, ParseIntError};

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use form_urlencoded::parse;
use indexmap::IndexMap;
use serde::Serialize;

use crate::roi_connections::error::QueryIssue;

const DEFAULT_LIMIT: i64 = 100;
const MIN_LIMIT: i64 = 1;
const MAX_LIMIT: i64 = 1000;
const DEFAULT_OFFSET: i64 = 0;

const INT_PARSING: &str = "Input should be a valid integer, unable to parse string as an integer";
const LIMIT_MINIMUM: &str = "Input should be greater than or equal to 1";
const LIMIT_MAXIMUM: &str = "Input should be less than or equal to 1000";
const OFFSET_MINIMUM: &str = "Input should be greater than or equal to 0";

// GET /health's query: both optional strings, an empty one left as given
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HealthQuery {
    pub model: Option<String>,
    pub model_id: Option<String>,
}

// GET /health/history's query after pydantic's defaults and bounds
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryQuery {
    pub model: Option<String>,
    pub status_filter: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

// The 422 body FastAPI builds from the failing query parameters
#[derive(Debug)]
pub struct QueryFailure {
    pub detail: Vec<QueryIssue>,
}

#[derive(Serialize)]
struct QueryFailureBody {
    detail: Vec<QueryIssue>,
}

impl IntoResponse for QueryFailure {
    fn into_response(self) -> Response {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(QueryFailureBody {
                detail: self.detail,
            }),
        )
            .into_response()
    }
}

#[must_use]
pub fn decode_health_query(raw: Option<&str>) -> HealthQuery {
    let values = values(raw);
    HealthQuery {
        model: values.get("model").cloned(),
        model_id: values.get("model_id").cloned(),
    }
}

pub fn decode_history_query(raw: Option<&str>) -> Result<HistoryQuery, QueryFailure> {
    let values = values(raw);
    let mut issues = Vec::new();
    let mut limit = DEFAULT_LIMIT;
    let mut offset = DEFAULT_OFFSET;
    if let Some(raw_limit) = values.get("limit") {
        match parse_limit(raw_limit) {
            Ok(value) => limit = value,
            Err(issue) => issues.push(issue),
        }
    }
    if let Some(raw_offset) = values.get("offset") {
        match parse_offset(raw_offset) {
            Ok(value) => offset = value,
            Err(issue) => issues.push(issue),
        }
    }
    if !issues.is_empty() {
        return Err(QueryFailure { detail: issues });
    }
    Ok(HistoryQuery {
        model: values.get("model").cloned(),
        status_filter: values.get("status_filter").cloned(),
        limit,
        offset,
    })
}

// Starlette keeps the last value of a repeated parameter, and so does collecting into a map
fn values(raw: Option<&str>) -> IndexMap<String, String> {
    parse(raw.unwrap_or_default().as_bytes())
        .into_owned()
        .collect()
}

fn parse_limit(raw: &str) -> Result<i64, QueryIssue> {
    match raw.trim().parse::<i64>() {
        Ok(value) if value < MIN_LIMIT => Err(issue("greater_than_equal", "limit", LIMIT_MINIMUM)),
        Ok(value) if value > MAX_LIMIT => Err(issue("less_than_equal", "limit", LIMIT_MAXIMUM)),
        Ok(value) => Ok(value),
        Err(error) => Err(limit_parse_error(&error)),
    }
}

fn limit_parse_error(error: &ParseIntError) -> QueryIssue {
    if error.kind() == &IntErrorKind::PosOverflow {
        return issue("less_than_equal", "limit", LIMIT_MAXIMUM);
    }
    if error.kind() == &IntErrorKind::NegOverflow {
        return issue("greater_than_equal", "limit", LIMIT_MINIMUM);
    }
    issue("int_parsing", "limit", INT_PARSING)
}

fn parse_offset(raw: &str) -> Result<i64, QueryIssue> {
    let value = match raw.trim().parse::<i64>() {
        // ge=0 holds for a value past i64, and reading past the end of the table is an empty page
        Err(error) if error.kind() == &IntErrorKind::PosOverflow => return Ok(i64::MAX),
        Err(error) => return Err(offset_parse_error(&error)),
        Ok(value) => value,
    };
    if value < DEFAULT_OFFSET {
        return Err(issue("greater_than_equal", "offset", OFFSET_MINIMUM));
    }
    Ok(value)
}

fn offset_parse_error(error: &ParseIntError) -> QueryIssue {
    if error.kind() == &IntErrorKind::NegOverflow {
        return issue("greater_than_equal", "offset", OFFSET_MINIMUM);
    }
    issue("int_parsing", "offset", INT_PARSING)
}

fn issue(kind: &'static str, field: &'static str, message: &'static str) -> QueryIssue {
    QueryIssue {
        kind,
        loc: vec!["query", field],
        msg: message.to_owned(),
    }
}
