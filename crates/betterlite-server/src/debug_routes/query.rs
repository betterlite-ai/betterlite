use std::collections::BTreeMap;

use axum::Json;
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use thiserror::Error;

use crate::model_query::read_query;

const INTEGER_MESSAGE: &str =
    "Input should be a valid integer, unable to parse string as an integer";
const BOOLEAN_MESSAGE: &str = "Input should be a valid boolean, unable to interpret input";

#[derive(Debug, Error)]
pub(crate) enum QueryError {
    #[error("the {name} query parameter is not an integer")]
    NotAnInteger { name: &'static str },
    #[error("the {name} query parameter is not a boolean")]
    NotABoolean { name: &'static str },
}

#[derive(Serialize)]
struct QueryIssueBody {
    #[serde(rename = "type")]
    kind: &'static str,
    loc: [&'static str; 2],
    msg: &'static str,
}

#[derive(Serialize)]
struct QueryIssuesBody {
    detail: [QueryIssueBody; 1],
}

impl QueryError {
    fn body(&self) -> QueryIssuesBody {
        let (kind, name, msg) = match self {
            Self::NotAnInteger { name } => ("int_parsing", *name, INTEGER_MESSAGE),
            Self::NotABoolean { name } => ("bool_parsing", *name, BOOLEAN_MESSAGE),
        };
        QueryIssuesBody {
            detail: [QueryIssueBody {
                kind,
                loc: ["query", name],
                msg,
            }],
        }
    }
}

impl IntoResponse for QueryError {
    fn into_response(self) -> Response {
        (StatusCode::UNPROCESSABLE_ENTITY, Json(self.body())).into_response()
    }
}

// Starlette keeps the last value of a repeated parameter, and so does collecting into a map
pub(crate) fn values(uri: &Uri) -> BTreeMap<String, String> {
    form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes())
        .into_owned()
        .collect()
}

pub(crate) fn integer(
    values: &BTreeMap<String, String>,
    name: &'static str,
) -> Result<Option<i64>, QueryError> {
    match values.get(name) {
        None => Ok(None),
        Some(text) => text
            .parse::<i64>()
            .map(Some)
            .map_err(|_parse| QueryError::NotAnInteger { name }),
    }
}

// the shared pydantic reader recognizes the same words, with no trimming
pub(crate) fn boolean(uri: &Uri, name: &'static str, default: bool) -> Result<bool, QueryError> {
    let (values, [value]) =
        read_query(uri.query(), [name]).map_err(|_error| QueryError::NotABoolean { name })?;
    Ok(if values.contains_key(name) {
        value
    } else {
        default
    })
}
