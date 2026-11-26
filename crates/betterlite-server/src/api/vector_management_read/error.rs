use std::convert::Infallible;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_auth::resolved_key::auth::AuthError;
use betterlite_core::api::vector_store_access::error::VectorStoreAccessError;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_models::decisions::Json as DecisionJson;
use serde::Serialize;

use super::query::QueryError;
use crate::api::vector_management_boundaries::error::VectorManagementError;
use crate::api::vector_management_boundaries::json::ManagementJsonError;
use crate::management_rbac::FeatureAccessError;
use crate::search_tool_routes::body::BodyError;

#[derive(Debug, thiserror::Error)]
pub enum ManagementReadError {
    #[error("the caller identity could not be read")]
    Identity {
        #[source]
        source: serde_json::Error,
    },
    #[error("the caller identity could not be built")]
    Key {
        #[source]
        source: AuthError,
    },
    #[error("{source}")]
    Access {
        #[source]
        source: VectorStoreAccessError,
    },
    #[error("{source}")]
    Rows {
        #[source]
        source: TableError,
    },
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
    // the status and the FastAPI detail the response carries; a request error renders its own body instead
    #[must_use]
    pub fn refusal(&self) -> (StatusCode, Option<DecisionJson>) {
        match self {
            Self::Feature(error) => {
                let status = error.status_code();
                let detail = match error {
                    FeatureAccessError::Disabled(feature) => {
                        let text = serde_json::to_string(&feature.denial()).unwrap_or_default();
                        serde_json::from_str(&text).ok()
                    }
                    FeatureAccessError::Privileges(_never) => None,
                };
                (status, detail)
            }
            Self::Management(error) => match error.http() {
                Some((status, detail)) => (
                    StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                    serde_json::from_str(&serde_json::to_string(&detail).unwrap_or_default()).ok(),
                ),
                None => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Some(DecisionJson::Text(error.to_string())),
                ),
            },
            Self::Identity { .. }
            | Self::Key { .. }
            | Self::DatabaseNotConnected
            | Self::Access { .. }
            | Self::Rows { .. }
            | Self::Metadata(_)
            | Self::MetadataShape
            | Self::Internal { .. }
            | Self::Json(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Some(DecisionJson::Text(self.to_string())),
            ),
            Self::Query(_) | Self::Request(_) => (StatusCode::UNPROCESSABLE_ENTITY, None),
        }
    }

    // the raises the info route rethrows untouched: an HTTPException, not the catch-all's 500
    #[must_use]
    pub fn is_http_exception(&self) -> bool {
        match self {
            Self::Management(error) => error.http().is_some(),
            Self::Feature(_)
            | Self::Identity { .. }
            | Self::Key { .. }
            | Self::Access { .. }
            | Self::Rows { .. }
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
            error @ (Self::Feature(_)
            | Self::Identity { .. }
            | Self::Key { .. }
            | Self::Access { .. }
            | Self::Rows { .. }
            | Self::DatabaseNotConnected
            | Self::Metadata(_)
            | Self::MetadataShape
            | Self::Management(_)
            | Self::Internal { .. }
            | Self::Json(_)) => {
                let (status, detail) = error.refusal();
                let body_detail = detail.unwrap_or(DecisionJson::Null);
                (
                    status,
                    Json(DetailBody {
                        detail: body_detail,
                    }),
                )
                    .into_response()
            }
        }
    }
}
