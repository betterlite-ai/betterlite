use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use super::BodyError;
use super::issue::{IssueKind, LocPart, ValidationIssue};

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
            Self::InvalidJson { .. } => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ValidationBody {
                    detail: vec![ValidationIssue::of(
                        IssueKind::JsonInvalid,
                        vec![LocPart::Field("body")],
                    )],
                }),
            )
                .into_response(),
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
