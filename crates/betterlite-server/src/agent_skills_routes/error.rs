use axum::Json;
use axum::http::StatusCode;
use axum::http::header::InvalidHeaderValue;
use axum::response::{IntoResponse, Response};
use betterlite_core::api::stored_skill_reads::error::StoredSkillReadError;
use serde::Serialize;
use thiserror::Error;
use tokio::task::JoinError;

#[derive(Debug, Error)]
pub(crate) enum AgentSkillsError {
    #[error("Not Found")]
    NotFound,
    #[error("No installable skill archive for: {skill_id}")]
    NoArchive { skill_id: String },
    #[error("the stored skills could not be listed")]
    List(#[source] StoredSkillReadError),
    #[error("the stored skill could not be read")]
    Read(#[source] StoredSkillReadError),
    #[error("the skill timestamp could not be rendered as an ISO datetime")]
    Timestamp(#[source] serde_json::Error),
    #[error("the skill archive build task returned nothing")]
    MissingBuild,
    #[error("the skill archive build task failed")]
    Build(#[source] JoinError),
    #[error("the archive download header is not valid")]
    Header(#[source] InvalidHeaderValue),
}

#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

impl IntoResponse for AgentSkillsError {
    fn into_response(self) -> Response {
        match self {
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                Json(DetailBody {
                    detail: "Not Found".to_owned(),
                }),
            )
                .into_response(),
            Self::NoArchive { skill_id } => (
                StatusCode::NOT_FOUND,
                Json(DetailBody {
                    detail: format!("No installable skill archive for: {skill_id}"),
                }),
            )
                .into_response(),
            internal @ (Self::List(_)
            | Self::Read(_)
            | Self::Timestamp(_)
            | Self::MissingBuild
            | Self::Build(_)
            | Self::Header(_)) => {
                // Starlette answers an unhandled exception with a plain 500 body
                tracing::warn!(error = %internal, "agent skills route failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
            }
        }
    }
}
