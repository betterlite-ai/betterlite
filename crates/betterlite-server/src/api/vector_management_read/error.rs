use std::convert::Infallible;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_models::decisions::Json as DecisionJson;
use serde::Serialize;

use super::query::QueryError;
use crate::api::vector_management_boundaries::error::VectorManagementError;
use crate::api::vector_management_boundaries::json::ManagementJsonError;
use crate::management_rbac::FeatureAccessError;
use crate::search_tool_routes::body::BodyError;

#[derive(Debug, thiserror::Error)]
pub enum ManagementReadError {
    #[error(transparent)]
    Feature(#[from] FeatureAccessError<Infallible>),
    #[error("Database not connected")]
    DatabaseNotConnected,
    #[error(transparent)]
    Metadata(#[from] ManagementJsonError),
    #[error("the stored vector store metadata is not a JSON object")]
    MetadataShape,
    // B2486's fetch and access helpers: 403/404 and a team failure stay HTTPExceptions, the rest is the 500
    #[error(transparent)]
    Management(#[from] VectorManagementError),
    #[error("{message}")]
    Internal { message: String },
    #[error("the response could not be serialized")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Query(#[from] QueryError),
    #[error(transparent)]
    Request(#[from] BodyError),
}

#[derive(Serialize)]
struct DetailBody<Detail> {
    detail: Detail,
}

impl ManagementReadError {
    #[must_use]
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Feature(error) => error.status_code(),
            Self::DatabaseNotConnected
            | Self::Metadata(_)
            | Self::MetadataShape
            | Self::Internal { .. }
            | Self::Json(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Management(error) => {
                error
                    .http()
                    .map_or(StatusCode::INTERNAL_SERVER_ERROR, |(status, _detail)| {
                        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
                    })
            }
            Self::Query(_) | Self::Request(_) => StatusCode::UNPROCESSABLE_ENTITY,
        }
    }

    // the FastAPI detail object a handler raises, for a caller rendering the response itself
    #[must_use]
    pub fn detail(&self) -> Option<DecisionJson> {
        match self {
            Self::Feature(FeatureAccessError::Disabled(feature)) => convert(feature.denial()),
            Self::Feature(FeatureAccessError::Privileges(never)) => match *never {},
            Self::DatabaseNotConnected => {
                Some(DecisionJson::Text("Database not connected".to_owned()))
            }
            Self::Management(error) => match error.http() {
                Some((_status, detail)) => convert(detail),
                None => Some(DecisionJson::Text(error.to_string())),
            },
            Self::Metadata(error) => Some(DecisionJson::Text(error.to_string())),
            Self::MetadataShape | Self::Internal { .. } => {
                Some(DecisionJson::Text(self.to_string()))
            }
            Self::Json(error) => Some(DecisionJson::Text(error.to_string())),
            Self::Query(_) | Self::Request(_) => None,
        }
    }

    // the raises the info route rethrows untouched: an HTTPException, not the catch-all's 500
    #[must_use]
    pub fn is_http_exception(&self) -> bool {
        match self {
            Self::Management(error) => error.http().is_some(),
            Self::Feature(_)
            | Self::DatabaseNotConnected
            | Self::Metadata(_)
            | Self::MetadataShape
            | Self::Internal { .. }
            | Self::Json(_)
            | Self::Query(_)
            | Self::Request(_) => false,
        }
    }
}

impl IntoResponse for ManagementReadError {
    fn into_response(self) -> Response {
        match self {
            Self::Query(error) => error.into_response(),
            Self::Request(error) => error.into_response(),
            error => {
                let status = error.status();
                let detail = error.detail().unwrap_or(DecisionJson::Null);
                (status, Json(DetailBody { detail })).into_response()
            }
        }
    }
}

// serde_json has no conversion from the arbitrary JSON value into the keyed decision tree
fn convert<T>(value: T) -> Option<DecisionJson>
where
    T: Serialize,
{
    let text = serde_json::to_string(&value).ok()?;
    serde_json::from_str(&text).ok()
}
