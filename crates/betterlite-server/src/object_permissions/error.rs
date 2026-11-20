use axum::http::StatusCode;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_logging::private_json::error::PrivateJsonError;
use thiserror::Error;

use crate::key_fields::error::DecodeError;

use super::ambiguity::AmbiguityError;

#[derive(Debug, Error)]
pub enum ObjectPermissionError {
    #[error("Prisma client not found")]
    PrismaClientMissing,
    #[error(transparent)]
    Database(#[from] TableError),
    #[error(transparent)]
    Ambiguous(#[from] AmbiguityError),
    #[error(transparent)]
    Serialization(#[from] PrivateJsonError),
    #[error(transparent)]
    Document(#[from] DecodeError),
    #[error("the stored object permission row has no object_permission_id")]
    MissingObjectPermissionId,
}

impl ObjectPermissionError {
    #[must_use]
    pub const fn status_code(&self) -> Option<StatusCode> {
        match self {
            Self::Ambiguous(error) => error.status_code(),
            Self::PrismaClientMissing
            | Self::Database(_)
            | Self::Serialization(_)
            | Self::Document(_)
            | Self::MissingObjectPermissionId => None,
        }
    }
}
