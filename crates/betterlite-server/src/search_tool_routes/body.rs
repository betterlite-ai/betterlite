use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::error::Category;
use serde_path_to_error::Segment;
use thiserror::Error;

const BODY: &str = "body";
const FIELD_REQUIRED: &str = "Field required";
const JSON_DECODE_ERROR: &str = "JSON decode error";

// FastAPI's request validation error: one issue with the path into the body that failed
#[derive(Debug, Error, Serialize)]
#[error("{msg}")]
pub(super) struct BodyError {
    #[serde(rename = "type")]
    issue_type: &'static str,
    loc: Vec<Location>,
    msg: String,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum Location {
    Field(String),
    Position(usize),
}

#[derive(Serialize)]
struct ValidationBody {
    detail: [BodyError; 1],
}

// FastAPI reads an empty or null body as a missing one, and any other bytes as JSON
pub(super) fn decode_body<Body>(bytes: &[u8]) -> Result<Body, BodyError>
where
    Body: DeserializeOwned,
{
    if bytes.is_empty() || bytes.trim_ascii() == b"null" {
        return Err(BodyError {
            issue_type: "missing",
            loc: vec![Location::Field(BODY.to_owned())],
            msg: FIELD_REQUIRED.to_owned(),
        });
    }
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let decoded =
        serde_path_to_error::deserialize(&mut deserializer).map_err(|error| issue(&error))?;
    deserializer.end().map_err(|_trailing| BodyError {
        issue_type: "json_invalid",
        loc: vec![Location::Field(BODY.to_owned())],
        msg: JSON_DECODE_ERROR.to_owned(),
    })?;
    Ok(decoded)
}

fn issue(error: &serde_path_to_error::Error<serde_json::Error>) -> BodyError {
    let mut loc = vec![Location::Field(BODY.to_owned())];
    if matches!(error.inner().classify(), Category::Syntax | Category::Eof) {
        return BodyError {
            issue_type: "json_invalid",
            loc,
            msg: JSON_DECODE_ERROR.to_owned(),
        };
    }
    for segment in error.path() {
        match segment {
            Segment::Seq { index } => loc.push(Location::Position(*index)),
            Segment::Map { key } => loc.push(Location::Field(key.clone())),
            Segment::Enum { variant } => loc.push(Location::Field(variant.clone())),
            Segment::Unknown => {}
        }
    }
    let message = error.inner().to_string();
    let missing = message
        .strip_prefix("missing field `")
        .and_then(|rest| rest.split_once('`'));
    if let Some((field, _position)) = missing {
        loc.push(Location::Field(field.to_owned()));
        return BodyError {
            issue_type: "missing",
            loc,
            msg: FIELD_REQUIRED.to_owned(),
        };
    }
    BodyError {
        issue_type: "value_error",
        loc,
        msg: message,
    }
}

impl IntoResponse for BodyError {
    fn into_response(self) -> Response {
        let body = ValidationBody { detail: [self] };
        (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
    }
}
