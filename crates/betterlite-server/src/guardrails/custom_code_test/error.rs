use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use serde::Serialize;
use serde_json::error::Category;
use serde_path_to_error::Path;
use serde_path_to_error::Segment;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TestRouteError {
    #[error("the request body does not decode as the test custom code request")]
    Body {
        #[source]
        source: serde_path_to_error::Error<serde_json::Error>,
    },
    #[error("the request body holds more than one JSON value")]
    Trailing {
        #[source]
        source: serde_json::Error,
    },
    #[error("the number {value} does not fit a JSON number")]
    Number { value: String },
}

#[derive(Debug, Serialize)]
struct ValidationBody {
    detail: Vec<ValidationIssue>,
}

#[derive(Debug, Serialize)]
struct ValidationIssue {
    #[serde(rename = "type")]
    issue_type: &'static str,
    loc: Vec<LocPart>,
    msg: String,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum LocPart {
    Field(String),
    Index(usize),
}

impl IntoResponse for TestRouteError {
    fn into_response(self) -> Response {
        let body = ValidationBody {
            detail: vec![self.issue()],
        };
        (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
    }
}

impl TestRouteError {
    fn issue(&self) -> ValidationIssue {
        match self {
            Self::Body { source } => body_issue(source),
            Self::Trailing { .. } => ValidationIssue {
                issue_type: "json_invalid",
                loc: body_location(None),
                msg: JSON_DECODE_ERROR.to_owned(),
            },
            Self::Number { .. } => ValidationIssue {
                issue_type: "value_error",
                loc: body_location(None),
                msg: self.to_string(),
            },
        }
    }
}

fn body_issue(source: &serde_path_to_error::Error<serde_json::Error>) -> ValidationIssue {
    let message = source.inner().to_string();
    if let Some(field) = message
        .strip_prefix("missing field `")
        .and_then(|rest| rest.strip_suffix('`'))
    {
        let mut loc = body_location(Some(source.path()));
        loc.push(LocPart::Field(field.to_owned()));
        return ValidationIssue {
            issue_type: "missing",
            loc,
            msg: FIELD_REQUIRED.to_owned(),
        };
    }
    let (issue_type, msg) = match source.inner().classify() {
        Category::Syntax | Category::Eof => ("json_invalid", JSON_DECODE_ERROR.to_owned()),
        Category::Io | Category::Data => ("value_error", message),
    };
    ValidationIssue {
        issue_type,
        loc: body_location(Some(source.path())),
        msg,
    }
}

fn body_location(path: Option<&Path>) -> Vec<LocPart> {
    let mut loc = vec![LocPart::Field("body".to_owned())];
    let Some(segments) = path else {
        return loc;
    };
    for segment in segments {
        match segment {
            Segment::Seq { index } => loc.push(LocPart::Index(*index)),
            Segment::Map { key } => loc.push(LocPart::Field(key.clone())),
            Segment::Enum { variant } => loc.push(LocPart::Field(variant.clone())),
            Segment::Unknown => {}
        }
    }
    loc
}

const JSON_DECODE_ERROR: &str = "JSON decode error";
const FIELD_REQUIRED: &str = "Field required";
