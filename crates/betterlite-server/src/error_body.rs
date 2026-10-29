use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_error::envelope::{ErrorObject, ErrorResponse, WireField};
use serde::Serialize;
pub(crate) enum AuthenticationFailure {
    Missing,
    Invalid,
}
pub(crate) fn authentication_error(failure: &AuthenticationFailure) -> Response {
    match failure {
        AuthenticationFailure::Missing => {
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
        AuthenticationFailure::Invalid => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: ErrorObject {
                    message: "the API key is not valid".to_owned(),
                    error_type: WireField::Text("no_db_connection".to_owned()),
                    param: WireField::Null,
                    code: WireField::Text("400".to_owned()),
                },
            }),
        )
            .into_response(),
    }
}
pub(crate) enum ModelError {
    InvalidFlags(Vec<&'static str>),
    Detail(StatusCode, String),
    Unsupported(String),
}
#[derive(Serialize)]
struct DetailBody<Detail> {
    detail: Detail,
}
#[derive(Serialize)]
struct FlagIssue {
    loc: [&'static str; 2],
    msg: &'static str,
    #[serde(rename = "type")]
    issue_type: &'static str,
}
impl IntoResponse for ModelError {
    fn into_response(self) -> Response {
        match self {
            Self::InvalidFlags(names) => {
                let detail: Vec<FlagIssue> = names
                    .into_iter()
                    .map(|name| FlagIssue {
                        loc: ["query", name],
                        msg: "not a boolean",
                        issue_type: "bool_parsing",
                    })
                    .collect();
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(DetailBody { detail }),
                )
                    .into_response()
            }
            Self::Detail(status, detail) => (status, Json(DetailBody { detail })).into_response(),
            Self::Unsupported(feature) => {
                let detail = format!("betterlite does not support {feature} yet");
                (StatusCode::NOT_IMPLEMENTED, Json(DetailBody { detail })).into_response()
            }
        }
    }
}
pub(crate) fn request_timeout_error() -> Response {
    let error = ErrorResponse {
        error: ErrorObject {
            message: "request timed out".to_owned(),
            error_type: WireField::Text("invalid_request_error".to_owned()),
            param: WireField::Null,
            code: WireField::Text("408".to_owned()),
        },
    };
    (StatusCode::REQUEST_TIMEOUT, Json(error)).into_response()
}
