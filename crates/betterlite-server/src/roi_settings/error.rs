use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_db::resource_repository::config::ConfigError;
use betterlite_error::envelope::{ErrorObject, ErrorResponse, WireField};
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use betterlite_roi::settings::ROISettingsUpdate;
use betterlite_roi::settings::SettingsError;
use betterlite_secrets::error::SecretError;

use super::outage::is_database_outage;
use super::store::StoredSettingsError;
use serde::Serialize;
use serde_json::error::Category;
use serde_path_to_error::Segment;
use thiserror::Error;

use crate::tracing_routes::router::unauthenticated;

pub(super) const INVALID_SETTINGS: &str = "Stored ROI Calculator settings are invalid.";
pub(super) const INVALID_REPORT: &str = "Stored ROI Calculator report is invalid.";
pub(super) const NULL_URL: &str = "GitHub API URL cannot be null.";
pub(super) const DB_UNAVAILABLE: &str = "the ROI Calculator settings database is unavailable";
const FIELD_REQUIRED: &str = "Field required";
const JSON_DECODE_ERROR: &str = "JSON decode error";
const MODEL_ATTRIBUTES_MESSAGE: &str =
    "Input should be a valid dictionary or object to extract fields from";
const EXTRA_FORBIDDEN_MESSAGE: &str = "Extra inputs are not permitted";

#[derive(Debug, Error)]
pub enum RoiSettingsError {
    #[error("the request has no authenticated caller")]
    Unauthenticated,
    #[error("{0}")]
    Forbidden(&'static str),
    #[error("Stored ROI Calculator settings are invalid.")]
    StoredSettings(#[source] StoredSettingsError),
    #[error("Stored ROI Calculator report is invalid.")]
    StoredReport(#[source] serde_json::Error),
    #[error("GitHub API URL cannot be null.")]
    NullUrl,
    #[error("the request body is empty")]
    MissingBody,
    #[error("the request body is not valid JSON")]
    InvalidJson,
    #[error("the request body does not decode")]
    Body(#[source] Box<serde_path_to_error::Error<serde_json::Error>>),
    #[error("the request does not match the declared types")]
    Request(Vec<Issue>),
    #[error("the settings do not validate")]
    Validation(#[source] SettingsError),
    #[error("the ROI settings request failed")]
    Internal(#[from] InternalError),
}

#[derive(Debug, Error)]
pub enum InternalError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Encode(#[from] serde_json::Error),
    #[error("the settings secret could not be encrypted")]
    Encryption(#[source] SecretError),
    #[error("the database is not connected")]
    DbNotConnected,
}

#[derive(Debug, Serialize)]
pub struct Issue {
    #[serde(rename = "type")]
    kind: &'static str,
    loc: Vec<LocPart>,
    msg: String,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum LocPart {
    Field(String),
    Index(usize),
}

#[derive(Debug, Serialize)]
struct DetailBody {
    detail: Detail,
}

// otel_unhandled_exception_handler's body for an uncaught exception: no param or code fields
#[derive(Debug, Serialize)]
struct InternalBody {
    error: InternalMessage,
}

#[derive(Debug, Serialize)]
struct InternalMessage {
    message: &'static str,
    #[serde(rename = "type")]
    error_type: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum Detail {
    Text(String),
    Issues(Vec<Issue>),
}

impl IntoResponse for RoiSettingsError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => unauthenticated(),
            Self::Forbidden(detail) => detail_response(StatusCode::FORBIDDEN, detail.to_owned()),
            Self::StoredSettings(_) => detail_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                INVALID_SETTINGS.to_owned(),
            ),
            Self::StoredReport(_) => {
                detail_response(StatusCode::INTERNAL_SERVER_ERROR, INVALID_REPORT.to_owned())
            }
            Self::NullUrl => detail_response(StatusCode::UNPROCESSABLE_ENTITY, NULL_URL.to_owned()),
            Self::MissingBody => issues_response(vec![Issue {
                kind: "missing",
                loc: vec![LocPart::Field("body".to_owned())],
                msg: FIELD_REQUIRED.to_owned(),
            }]),
            Self::InvalidJson => issues_response(vec![Issue {
                kind: "json_invalid",
                loc: vec![LocPart::Field("body".to_owned())],
                msg: JSON_DECODE_ERROR.to_owned(),
            }]),
            Self::Body(source) => issues_response(vec![body_issue(&source)]),
            Self::Request(issues) => issues_response(issues),
            Self::Validation(error) => issues_response(vec![settings_issue(&error)]),
            Self::Internal(error) => internal_response(&error),
        }
    }
}

fn internal_response(error: &InternalError) -> Response {
    if matches!(error, InternalError::Config(source) if is_database_outage(source)) {
        return database_unavailable();
    }
    if let InternalError::DbNotConnected = error {
        return detail_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            CommonProxyErrors::DbNotConnectedError.as_str().to_owned(),
        );
    }
    tracing::error!(error = ?error, "the ROI settings request failed");
    let body = InternalBody {
        error: InternalMessage {
            message: "Internal server error",
            error_type: "internal_server_error",
        },
    };
    (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
}

fn detail_response(status: StatusCode, detail: String) -> Response {
    let body = DetailBody {
        detail: Detail::Text(detail),
    };
    (status, Json(body)).into_response()
}

fn issues_response(issues: Vec<Issue>) -> Response {
    let body = DetailBody {
        detail: Detail::Issues(issues),
    };
    (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
}

fn database_unavailable() -> Response {
    let body = ErrorResponse {
        error: ErrorObject {
            message: DB_UNAVAILABLE.to_owned(),
            error_type: WireField::Text("no_db_connection".to_owned()),
            param: WireField::MISSING,
            code: WireField::Text("503".to_owned()),
        },
    };
    (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
}

fn body_issue(source: &serde_path_to_error::Error<serde_json::Error>) -> Issue {
    if matches!(source.inner().classify(), Category::Syntax | Category::Eof) {
        return Issue {
            kind: "json_invalid",
            loc: vec![LocPart::Field("body".to_owned())],
            msg: JSON_DECODE_ERROR.to_owned(),
        };
    }
    let message = source.inner().to_string();
    if let Some(field) = unknown_field(&message) {
        let loc = vec![LocPart::Field("body".to_owned()), LocPart::Field(field)];
        return Issue {
            kind: "extra_forbidden",
            loc,
            msg: EXTRA_FORBIDDEN_MESSAGE.to_owned(),
        };
    }
    let loc = body_loc(source);
    if message.starts_with("invalid type: null") {
        return Issue {
            kind: "missing",
            loc,
            msg: FIELD_REQUIRED.to_owned(),
        };
    }
    if loc.len() == 1 {
        return Issue {
            kind: "model_attributes_type",
            loc,
            msg: MODEL_ATTRIBUTES_MESSAGE.to_owned(),
        };
    }
    Issue {
        kind: "value_error",
        loc,
        msg: message,
    }
}

fn body_loc(source: &serde_path_to_error::Error<serde_json::Error>) -> Vec<LocPart> {
    let mut loc = vec![LocPart::Field("body".to_owned())];
    for segment in source.path() {
        match segment {
            Segment::Seq { index } => loc.push(LocPart::Index(*index)),
            Segment::Map { key } => loc.push(LocPart::Field(key.clone())),
            Segment::Enum { variant } => loc.push(LocPart::Field(variant.clone())),
            Segment::Unknown => {}
        }
    }
    loc
}

fn unknown_field(message: &str) -> Option<String> {
    let rest = message.strip_prefix("unknown field `")?;
    let (field, _rest) = rest.split_once('`')?;
    Some(field.to_owned())
}

fn settings_issue(error: &SettingsError) -> Issue {
    let (kind, field) = match error {
        SettingsError::NotFinite { field } => ("finite_number", *field),
        SettingsError::TooSmall { field, .. } => ("greater_than_equal", *field),
        SettingsError::TooLarge { field, .. } => ("less_than_equal", *field),
        SettingsError::Interval => ("value_error", "update_interval_minutes"),
        SettingsError::MissingGitHubUrl
        | SettingsError::GitHubUrl
        | SettingsError::Ipv6Url
        | SettingsError::IpvFuture
        | SettingsError::BracketedIpv4
        | SettingsError::BracketedHost { .. }
        | SettingsError::NetlocNormalization { .. } => ("value_error", "github_api_url"),
        SettingsError::Repositories => ("value_error", "repos"),
        SettingsError::Prompt => ("value_error", "estimator_prompt"),
        SettingsError::Login | SettingsError::Identity => ("value_error", "identity_map"),
    };
    let msg = if kind == "value_error" {
        format!("Value error, {error}")
    } else {
        error.to_string()
    };
    Issue {
        kind,
        loc: vec![LocPart::Field(field.to_owned())],
        msg,
    }
}

pub(super) fn request_issues(patch: &ROISettingsUpdate) -> Vec<Issue> {
    let Err(error) = patch.clone().validate() else {
        return Vec::new();
    };
    let mut issue = settings_issue(&error);
    issue.loc.insert(0, LocPart::Field("body".to_owned()));
    vec![issue]
}
