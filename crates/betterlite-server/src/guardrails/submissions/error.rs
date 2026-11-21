use std::collections::BTreeMap;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_db::budget_permission_rows::JsonData;
use serde::Serialize;
use serde_json::error::Category;
use serde_path_to_error::{Path, Segment};
use thiserror::Error;

use super::rows::SubmissionRowsError;
use crate::tracing_routes::router::unauthenticated;

const JSON_DECODE_ERROR: &str = "JSON decode error";
const FIELD_REQUIRED: &str = "Field required";
const BODY: &str = "body";

#[derive(Debug, Error)]
pub enum SubmissionError {
    #[error("the request has no authenticated caller")]
    Unauthenticated,
    #[error("Prisma client not initialized")]
    NoDatabase,
    #[error("team_id is required. Provide it in the request body or use a team-scoped API key.")]
    TeamRequired,
    #[error("You are not a member of team '{team_id}'")]
    NotTeamMember { team_id: String },
    #[error(
        "Only guardrails with litellm_params.guardrail='generic_guardrail_api' are accepted for registration"
    )]
    GuardrailType,
    #[error("litellm_params.api_base is required for generic_guardrail_api")]
    ApiBaseRequired,
    #[error("litellm_params.api_base must use http or https scheme")]
    ApiBaseScheme,
    #[error("litellm_params.api_base must contain a valid hostname")]
    ApiBaseHost,
    #[error("litellm_params.mode is required (e.g. pre_call, post_call)")]
    ModeRequired,
    #[error("Guardrail with name '{name}' already exists")]
    DuplicateName { name: String },
    #[error("Guardrail submission not found")]
    SubmissionNotFound,
    #[error("You are not a member of the team that owns this submission")]
    NotOwningTeam,
    #[error("{0}")]
    Body(Box<RegisterBodyError>),
    #[error("{source}")]
    Rows { source: SubmissionRowsError },
    #[error("{source}")]
    Teams { source: UserLookupError },
    #[error("{source}")]
    Identity { source: serde_json::Error },
    #[error("{source}")]
    Encode { source: serde_json::Error },
}

#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

// LiteLLM's eager 500s: verbose_proxy_logger.exception(<context>: %s, e) then str(e) as the detail
pub(super) fn logged(context: &str, error: SubmissionError) -> SubmissionError {
    tracing::error!("{context}: {error}");
    error
}

impl SubmissionError {
    #[must_use]
    pub(super) const fn status(&self) -> StatusCode {
        match self {
            Self::Unauthenticated => StatusCode::UNAUTHORIZED,
            Self::Body(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::TeamRequired
            | Self::GuardrailType
            | Self::ApiBaseRequired
            | Self::ApiBaseScheme
            | Self::ApiBaseHost
            | Self::ModeRequired
            | Self::DuplicateName { .. } => StatusCode::BAD_REQUEST,
            Self::NotTeamMember { .. } | Self::NotOwningTeam => StatusCode::FORBIDDEN,
            Self::SubmissionNotFound => StatusCode::NOT_FOUND,
            Self::NoDatabase
            | Self::Rows { .. }
            | Self::Teams { .. }
            | Self::Identity { .. }
            | Self::Encode { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for SubmissionError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => unauthenticated(),
            Self::Body(error) => (*error).into_response(),
            Self::TeamRequired
            | Self::GuardrailType
            | Self::ApiBaseRequired
            | Self::ApiBaseScheme
            | Self::ApiBaseHost
            | Self::ModeRequired
            | Self::DuplicateName { .. }
            | Self::NotTeamMember { .. }
            | Self::NotOwningTeam
            | Self::SubmissionNotFound
            | Self::NoDatabase
            | Self::Rows { .. }
            | Self::Teams { .. }
            | Self::Identity { .. }
            | Self::Encode { .. } => {
                let detail = self.to_string();
                (self.status(), Json(DetailBody { detail })).into_response()
            }
        }
    }
}

// pydantic's 422 body as B3685 writes it: one issue with type, loc, msg and the rejected input
#[derive(Debug, Error, Serialize)]
#[error("{msg}")]
pub struct RegisterBodyError {
    #[serde(rename = "type")]
    issue_type: &'static str,
    loc: Vec<Location>,
    msg: String,
    input: JsonData,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum Location {
    Field(String),
    Position(usize),
}

#[derive(Serialize)]
struct ValidationBody {
    detail: [RegisterBodyError; 1],
}

impl RegisterBodyError {
    pub(super) fn invalid_json() -> Self {
        Self {
            issue_type: "json_invalid",
            loc: vec![Location::Field(BODY.to_owned())],
            msg: JSON_DECODE_ERROR.to_owned(),
            input: JsonData::Object(BTreeMap::new()),
        }
    }

    pub(super) fn serde_at(
        bytes: &[u8],
        source: &serde_path_to_error::Error<serde_json::Error>,
    ) -> Self {
        let message = source.inner().to_string();
        let mut loc = vec![Location::Field(BODY.to_owned())];
        append_path(&mut loc, source.path());
        if let Some(field) = missing_field(&message) {
            loc.push(Location::Field(field.to_owned()));
            let input = value_at(bytes, &[Location::Field(BODY.to_owned())]);
            return Self {
                issue_type: "missing",
                loc,
                msg: FIELD_REQUIRED.to_owned(),
                input,
            };
        }
        let (issue_type, msg) = match source.inner().classify() {
            Category::Syntax | Category::Eof => ("json_invalid", JSON_DECODE_ERROR.to_owned()),
            Category::Io | Category::Data => ("value_error", message),
        };
        let input = value_at(bytes, &loc);
        Self {
            issue_type,
            loc,
            msg,
            input,
        }
    }
}

impl IntoResponse for RegisterBodyError {
    fn into_response(self) -> Response {
        let body = ValidationBody { detail: [self] };
        (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
    }
}

fn missing_field(message: &str) -> Option<&str> {
    message
        .strip_prefix("missing field `")
        .and_then(|rest| rest.split_once('`'))
        .map(|(field, _position)| field)
}

fn append_path(loc: &mut Vec<Location>, path: &Path) {
    for segment in path {
        match segment {
            Segment::Seq { index } => loc.push(Location::Position(*index)),
            Segment::Map { key } => loc.push(Location::Field(key.clone())),
            Segment::Enum { variant } => loc.push(Location::Field(variant.clone())),
            Segment::Unknown => {}
        }
    }
}

// FastAPI reports the value at the rejected path, so the path is resolved against the raw body
fn value_at(bytes: &[u8], loc: &[Location]) -> JsonData {
    let Ok(root) = serde_json::from_slice::<JsonData>(bytes) else {
        return JsonData::Object(BTreeMap::new());
    };
    let mut current = &root;
    for part in loc.iter().skip(1) {
        let child = match (part, current) {
            (Location::Field(name), JsonData::Object(fields)) => fields.get(name),
            (Location::Position(index), JsonData::Array(items)) => items.get(*index),
            (Location::Field(_) | Location::Position(_), _) => None,
        };
        match child {
            Some(next) => current = next,
            None => break,
        }
    }
    current.clone()
}
