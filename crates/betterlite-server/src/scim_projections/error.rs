use crate::scim_records::email::EmailError;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::team_repository::repository::TeamQueryError;
use betterlite_db::user_repository::RowDecodeError;
use http::StatusCode;
use thiserror::Error;

// Display is the `error` text LiteLLM puts in the HTTPException detail
#[derive(Debug, Error)]
pub enum ProjectionError {
    #[error("No database connected")]
    NoDatabase,
    #[error("User not found with ID: {user_id}")]
    UserNotFound { user_id: String },
    #[error("Group not found with ID: {team_id}")]
    GroupNotFound { team_id: String },
    #[error("the database could not be reached")]
    Database {
        #[source]
        source: DbError,
    },
    #[error("the user lookup failed")]
    UserQuery {
        #[source]
        source: DbError<RowDecodeError>,
    },
    #[error("the team lookup failed")]
    TeamQuery {
        #[source]
        source: TeamQueryError,
    },
    #[error("scim_metadata is not an object")]
    ScimMetadataShape,
    #[error("scim_metadata does not match the SCIM name model")]
    ScimMetadata {
        #[source]
        source: serde_json::Error,
    },
    #[error("a stored value cannot be written as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the user's email address is not valid")]
    Email {
        #[source]
        source: EmailError,
    },
    #[error("a team without team_alias has no SCIM displayName")]
    GroupDisplayName,
}

impl ProjectionError {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::UserNotFound { .. } | Self::GroupNotFound { .. } => StatusCode::NOT_FOUND,
            Self::NoDatabase
            | Self::Database { .. }
            | Self::UserQuery { .. }
            | Self::TeamQuery { .. }
            | Self::ScimMetadataShape
            | Self::ScimMetadata { .. }
            | Self::Encode { .. }
            | Self::Email { .. }
            | Self::GroupDisplayName => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<DbError> for ProjectionError {
    fn from(source: DbError) -> Self {
        if matches!(source, DbError::NoDbConnected) {
            Self::NoDatabase
        } else {
            Self::Database { source }
        }
    }
}

impl From<DbError<RowDecodeError>> for ProjectionError {
    fn from(source: DbError<RowDecodeError>) -> Self {
        if matches!(source, DbError::NoDbConnected) {
            Self::NoDatabase
        } else {
            Self::UserQuery { source }
        }
    }
}

impl From<TeamQueryError> for ProjectionError {
    fn from(source: TeamQueryError) -> Self {
        if matches!(
            source,
            TeamQueryError::Database {
                source: DbError::NoDbConnected
            }
        ) {
            Self::NoDatabase
        } else {
            Self::TeamQuery { source }
        }
    }
}
