use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::team_repository::repository::TeamQueryError;
use http::StatusCode;
use thiserror::Error;

// Display is the `error` text LiteLLM puts in the HTTPException detail
#[derive(Debug, Error)]
pub enum ScimMemberIdentityError {
    #[error("Invalid member: user ID cannot be empty.")]
    EmptyMemberValue,
    #[error(
        "Member ID '{value}' names more than one member of this group, so the removal cannot be attributed. Send the LiteLLM user ID as the member value, or resolve the duplicate."
    )]
    AmbiguousRemoval { value: String },
    #[error("No database connected")]
    NoDatabase,
    #[error("the database could not be reached")]
    Database {
        #[source]
        source: DbError,
    },
    #[error("the team lookup failed")]
    TeamQuery {
        #[source]
        source: TeamQueryError,
    },
}

impl ScimMemberIdentityError {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::EmptyMemberValue | Self::AmbiguousRemoval { .. } => StatusCode::BAD_REQUEST,
            Self::NoDatabase | Self::Database { .. } | Self::TeamQuery { .. } => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }
}

impl From<DbError> for ScimMemberIdentityError {
    fn from(source: DbError) -> Self {
        if matches!(source, DbError::NoDbConnected) {
            Self::NoDatabase
        } else {
            Self::Database { source }
        }
    }
}

impl From<TeamQueryError> for ScimMemberIdentityError {
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
