use std::error::Error;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::resource_repository::budget::BudgetRepositoryError;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
// ast-grep-ignore: no-long-names
use betterlite_error::proxy_exception::model_access_denied::ModelAccessDeniedProxyException;
use serde::Serialize;
use thiserror::Error;

use crate::auth::auth_model_patterns::{ModelAccessRejection, rejection_response};
use crate::key_patch_fields::error::KeyPatchError;
use crate::object_permissions::error::ObjectPermissionError;
use crate::organization_create::budget::BudgetTransformError;

pub const ALL_MODELS_DENIED: &str =
    "User not allowed to give access to all models. Select models you want org to have access to.";
pub const UNHANDLED_MESSAGE: &str = "Internal server error";

#[derive(Debug, Error)]
pub enum OrganizationCreateError {
    #[error("the database is not connected")]
    DatabaseNotConnected,
    #[error("only proxy admins can create organizations")]
    NotAdmin { role: String },
    #[error("no models are configured on the proxy")]
    NoRouter,
    #[error(transparent)]
    Budget(#[from] KeyPatchError),
    #[error("{field} must be a number the budget table can store. Received: {received}")]
    BudgetRange {
        field: &'static str,
        received: String,
    },
    #[error("the caller may not hand every model to the organization")]
    AllModelsDenied,
    #[error(transparent)]
    // ast-grep-ignore: no-long-names
    ModelDenied(Box<ModelAccessDeniedProxyException>),
    #[error("the model fallback depth was exceeded for {model}")]
    RecurseDepth { model: String },
    #[error(transparent)]
    Permission(#[from] ObjectPermissionError),
    #[error(transparent)]
    Transform(#[from] BudgetTransformError),
    #[error("the request could not be encoded")]
    Encoding(#[source] serde_json::Error),
    #[error("the caller identity could not be read")]
    Caller(#[source] serde_json::Error),
    #[error("the database request failed: {0}")]
    Database(String),
    #[error("the stored row could not be read: {0}")]
    Row(String),
}

impl From<TableError> for OrganizationCreateError {
    fn from(error: TableError) -> Self {
        Self::Database(error.to_string())
    }
}

impl From<BudgetRepositoryError> for OrganizationCreateError {
    fn from(error: BudgetRepositoryError) -> Self {
        Self::Database(error.to_string())
    }
}

#[derive(Serialize)]
struct ErrorDetail {
    error: String,
}

#[derive(Serialize)]
struct DetailBody {
    detail: ErrorDetail,
}

#[derive(Serialize)]
struct InternalMessage {
    message: &'static str,
    #[serde(rename = "type")]
    error_type: &'static str,
}

#[derive(Serialize)]
struct InternalBody {
    error: InternalMessage,
}

fn detail_error(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(DetailBody {
            detail: ErrorDetail {
                error: message.to_owned(),
            },
        }),
    )
        .into_response()
}

fn unhandled(error: &dyn Error) -> Response {
    tracing::error!(error = %error, "the organization request failed");
    let body = InternalBody {
        error: InternalMessage {
            message: UNHANDLED_MESSAGE,
            error_type: "internal_server_error",
        },
    };
    (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
}

fn permission_response(error: &ObjectPermissionError) -> Response {
    if error.status_code() == Some(StatusCode::BAD_REQUEST) {
        return detail_error(StatusCode::BAD_REQUEST, &error.to_string());
    }
    unhandled(error)
}

impl IntoResponse for OrganizationCreateError {
    fn into_response(self) -> Response {
        match &self {
            Self::DatabaseNotConnected => detail_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                CommonProxyErrors::DbNotConnectedError.as_str(),
            ),
            Self::NotAdmin { role } => detail_error(
                StatusCode::UNAUTHORIZED,
                &format!("Only admins can create orgs. Your role is = {role}"),
            ),
            Self::NoRouter => detail_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                CommonProxyErrors::NoLlmRouter.as_str(),
            ),
            Self::Budget(error) => detail_error(StatusCode::BAD_REQUEST, &error.to_string()),
            Self::BudgetRange { .. } => detail_error(StatusCode::BAD_REQUEST, &self.to_string()),
            Self::AllModelsDenied => detail_error(StatusCode::BAD_REQUEST, ALL_MODELS_DENIED),
            Self::ModelDenied(denial) => {
                rejection_response(&ModelAccessRejection::Denied(Box::clone(denial)))
            }
            Self::RecurseDepth { model } => {
                rejection_response(&ModelAccessRejection::RecurseDepth {
                    model: model.clone(),
                })
            }
            Self::Permission(error) => permission_response(error),
            Self::Transform(_)
            | Self::Encoding(_)
            | Self::Caller(_)
            | Self::Database(_)
            | Self::Row(_) => unhandled(&self),
        }
    }
}
