use std::collections::BTreeMap;

use axum::Json;
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse as _, Response};
use serde::Serialize;
use thiserror::Error;

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
    default: i64,
) -> Result<i64, QueryError> {
    let Some(text) = values.get(name) else {
        return Ok(default);
    };
    text.parse::<i64>()
        .map_err(|_parse| QueryError::NotAnInteger { name })
}

// the strings pydantic accepts as a boolean, without trimming
pub(crate) fn boolean(
    values: &BTreeMap<String, String>,
    name: &'static str,
    default: bool,
) -> Result<bool, QueryError> {
    let Some(text) = values.get(name) else {
        return Ok(default);
    };
    match text.to_ascii_lowercase().as_str() {
        // ast-grep-ignore: no-python-habits - pydantic's spellings for a declared query bool, not env text
        "1" | "on" | "t" | "true" | "y" | "yes" => Ok(true),
        // ast-grep-ignore: no-python-habits - the same for the false spellings
        "0" | "off" | "f" | "false" | "n" | "no" => Ok(false),
        _ => Err(QueryError::NotABoolean { name }),
    }
}
