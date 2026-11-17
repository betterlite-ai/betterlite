use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;
use serde::Serialize;

use super::BodyError;
use super::issue::{IssueContext, LocPart, ValidationIssue};

const DECODE: &str = "JSON decode error";
const UNPARSABLE: &str = "There was an error parsing the body";

#[derive(Serialize)]
struct ValidationBody {
    detail: Vec<ValidationIssue>,
}

#[derive(Serialize)]
struct PlainDetail {
    detail: String,
}

impl IntoResponse for BodyError {
    fn into_response(self) -> Response {
        match self {
            Self::Issues { issues } => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ValidationBody { detail: issues }),
            )
                .into_response(),
            Self::InvalidJson { source } => {
                let issue = ValidationIssue {
                    kind: "json_invalid",
                    loc: vec![LocPart::Field("body")],
                    msg: DECODE,
                    input: JsonValue::Object(JsonMap::new()),
                    ctx: Some(IssueContext::Error {
                        error: source.to_string(),
                    }),
                };
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(ValidationBody {
                        detail: vec![issue],
                    }),
                )
                    .into_response()
            }
            Self::NotUtf8 { .. } => (
                StatusCode::BAD_REQUEST,
                Json(PlainDetail {
                    detail: UNPARSABLE.to_owned(),
                }),
            )
                .into_response(),
        }
    }
}
