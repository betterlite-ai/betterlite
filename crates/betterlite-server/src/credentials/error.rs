use axum::extract::rejection::BytesRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_db::resource_rows::ResourceRowsError;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use betterlite_secrets::error::SecretError;
use thiserror::Error;

use super::store::CredentialStoreError;
use crate::error_body::json_error;

#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("the credential request body is too large")]
    BodyTooLarge,
    #[error("the credential request body could not be read within its limit")]
    ReadBody {
        #[source]
        source: BytesRejection,
    },
    #[error("the credential request body does not decode")]
    Body {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Rows(#[from] ResourceRowsError),
    #[error("{}", CommonProxyErrors::DbNotConnectedError.as_str())]
    DbNotConnected,
    #[error("LLM router not found")]
    RouterMissing,
    #[error("LLM router not found. Please ensure you have a valid router instance.")]
    DeploymentRouterMissing,
    #[error("Model not found")]
    ModelNotFound,
    #[error("Credential not found. Got credential name: {credential_name}")]
    CredentialNotFound { credential_name: String },
    #[error("Credential not found in DB.")]
    CredentialNotFoundInDb,
    #[error(
        "Credential '{credential_name}' already exists. Update it with PATCH /credentials/{credential_name}, or delete it first."
    )]
    Duplicate { credential_name: String },
    #[error("Credential values are required. Unable to infer credential values from model ID.")]
    MissingValues,
    #[error("credential encryption failed")]
    Encryption {
        #[source]
        source: SecretError,
    },
    #[error("credential storage failed")]
    Store {
        #[from]
        source: CredentialStoreError,
    },
}

impl CredentialError {
    const fn status(&self) -> StatusCode {
        match self {
            Self::BodyTooLarge
            | Self::ReadBody { .. }
            | Self::Body { .. }
            | Self::Rows(_)
            | Self::MissingValues => StatusCode::BAD_REQUEST,
            Self::DbNotConnected
            | Self::RouterMissing
            | Self::DeploymentRouterMissing
            | Self::Encryption { .. }
            | Self::Store { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Self::ModelNotFound
            | Self::CredentialNotFound { .. }
            | Self::CredentialNotFoundInDb => StatusCode::NOT_FOUND,
            Self::Duplicate { .. } => StatusCode::CONFLICT,
        }
    }
}

impl IntoResponse for CredentialError {
    fn into_response(self) -> Response {
        let status = self.status();
        json_error(
            status,
            &self.to_string(),
            "internal_server_error",
            status.as_str(),
        )
    }
}
