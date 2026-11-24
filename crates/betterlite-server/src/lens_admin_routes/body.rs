use std::num::ParseIntError;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use garde::{Report, Validate};
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::search_tool_routes::body::{BodyError, decode_body};

#[derive(Debug, Serialize)]
pub struct Issue {
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
    pub(crate) loc: Vec<String>,
    pub(crate) msg: String,
}

#[derive(Serialize)]
pub(crate) struct ValidationBody {
    pub(crate) detail: Vec<Issue>,
}

#[derive(Debug, Error)]
pub enum BodyFailure {
    #[error(transparent)]
    Json(#[from] BodyError),
    #[error("the request body fields are invalid")]
    Fields(Vec<Issue>),
}

#[derive(Debug, Error)]
pub enum QueryFailure {
    #[error("the offset query parameter is not an integer")]
    Integer(#[source] ParseIntError),
    #[error("the offset query parameter is below zero")]
    Negative,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ContentQuery {
    pub(crate) cursor: String,
    pub(crate) offset: i64,
}

// FastAPI validates the declared body and query before the handler, so both fail before any role check
pub fn decode<Body>(bytes: &[u8]) -> Result<Body, BodyFailure>
where
    Body: DeserializeOwned + Validate<Context = ()>,
{
    let decoded: Body = decode_body(bytes)?;
    match decoded.validate() {
        Ok(()) => Ok(decoded),
        Err(report) => Err(BodyFailure::Fields(issues(&report))),
    }
}

pub fn decode_content_query(raw: &[u8]) -> Result<ContentQuery, QueryFailure> {
    let mut cursor = String::new();
    let mut offset = None;
    for (name, value) in form_urlencoded::parse(raw).into_owned() {
        match name.as_str() {
            "cursor" => cursor = value,
            "offset" => offset = Some(value),
            _ => {}
        }
    }
    let parsed = match offset {
        Some(text) => parse_offset(&text)?,
        None => 0,
    };
    Ok(ContentQuery {
        cursor,
        offset: parsed,
    })
}

fn parse_offset(text: &str) -> Result<i64, QueryFailure> {
    let value = text.parse::<i64>().map_err(QueryFailure::Integer)?;
    if value < 0 {
        return Err(QueryFailure::Negative);
    }
    Ok(value)
}

fn issues(report: &Report) -> Vec<Issue> {
    report
        .iter()
        .map(|(path, error)| Issue {
            kind: "value_error",
            loc: vec!["body".to_owned(), path.to_string()],
            msg: error.to_string(),
        })
        .collect()
}

impl IntoResponse for BodyFailure {
    fn into_response(self) -> Response {
        match self {
            Self::Json(error) => error.into_response(),
            Self::Fields(detail) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ValidationBody { detail }),
            )
                .into_response(),
        }
    }
}

impl IntoResponse for QueryFailure {
    fn into_response(self) -> Response {
        let issue = match self {
            Self::Integer(_source) => Issue {
                kind: "int_parsing",
                loc: vec!["query".to_owned(), "offset".to_owned()],
                msg: "Input should be a valid integer, unable to parse string as an integer"
                    .to_owned(),
            },
            Self::Negative => Issue {
                kind: "greater_than_equal",
                loc: vec!["query".to_owned(), "offset".to_owned()],
                msg: "Input should be greater than or equal to 0".to_owned(),
            },
        };
        BodyFailure::Fields(vec![issue]).into_response()
    }
}
