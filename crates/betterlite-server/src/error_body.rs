use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_error::envelope::{ErrorObject, ErrorResponse, WireField};
use serde::Serialize;
mod tests;
pub(crate) enum AuthenticationFailure {
    Missing,
    Invalid,
}
pub(crate) fn json_error(
    status: StatusCode,
    message: &str,
    error_type: &str,
    code: &str,
) -> Response {
    let body = ErrorResponse {
        error: ErrorObject {
            message: message.to_owned(),
            error_type: WireField::Text(error_type.to_owned()),
            param: WireField::Null,
            code: WireField::Text(code.to_owned()),
        },
    };
    (status, Json(body)).into_response()
}

// LiteLLM's user_api_key_auth dependency refuses a request with no key with this 401 auth_error, param the text "None"
pub(crate) fn missing_credential_error() -> Response {
    let body = ErrorResponse {
        error: ErrorObject {
            message: "Authentication Error, No api key passed in.".to_owned(),
            error_type: WireField::Text("auth_error".to_owned()),
            param: WireField::Text("None".to_owned()),
            code: WireField::Text("401".to_owned()),
        },
    };
    (StatusCode::UNAUTHORIZED, Json(body)).into_response()
}

pub(crate) fn authentication_error(failure: &AuthenticationFailure) -> Response {
    match failure {
        AuthenticationFailure::Missing => {
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
        AuthenticationFailure::Invalid => json_error(
            StatusCode::BAD_REQUEST,
            "the API key is not valid",
            "no_db_connection",
            "400",
        ),
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
