use std::collections::BTreeMap;

use axum::Json;
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use thiserror::Error;

const INTEGER_MESSAGE: &str =
    "Input should be a valid integer, unable to parse string as an integer";

#[derive(Debug, Error)]
pub enum QueryError {
    #[error("the {name} query parameter is not an integer")]
    NotAnInteger { name: &'static str },
}

#[derive(Serialize)]
struct Issue {
    #[serde(rename = "type")]
    issue_type: &'static str,
    loc: [&'static str; 2],
    msg: &'static str,
}

#[derive(Serialize)]
struct ValidationBody {
    detail: [Issue; 1],
}

impl IntoResponse for QueryError {
    fn into_response(self) -> Response {
        let Self::NotAnInteger { name } = self;
        let body = ValidationBody {
            detail: [Issue {
                issue_type: "int_parsing",
                loc: ["query", name],
                msg: INTEGER_MESSAGE,
            }],
        };
        (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
    }
}

// the list's two pagination parameters in the order FastAPI declares them
pub fn pagination(uri: &Uri) -> Result<(i64, i64), QueryError> {
    let page = integer(uri, "page", 1)?;
    let page_size = integer(uri, "page_size", 100)?;
    Ok((page, page_size))
}

// FastAPI's query reader: an absent value takes its default, Starlette keeps the last of a repeated one
pub fn integer(uri: &Uri, name: &'static str, default: i64) -> Result<i64, QueryError> {
    let values: BTreeMap<String, String> =
        form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    values.get(name).map_or(Ok(default), |text| {
        text.parse::<i64>()
            .map_err(|_parse| QueryError::NotAnInteger { name })
    })
}
