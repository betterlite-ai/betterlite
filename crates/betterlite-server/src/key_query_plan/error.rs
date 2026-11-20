use betterlite_auth::resolved_key::auth::AuthError;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::user_repository::RowDecodeError;

use crate::object_permissions::error::ObjectPermissionError;

use super::sort::SortError;

#[derive(Debug, thiserror::Error)]
pub enum KeyQueryPlanError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("failed to read the users expanded from the key page")]
    Users(#[from] DbError<RowDecodeError>),
    #[error("the stored key row is not valid JSON")]
    Row(#[source] serde_json::Error),
    #[error("the stored key row is not a JSON object")]
    RowObject,
    #[error("the stored deleted key row is invalid")]
    Deleted {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Permission(#[from] ObjectPermissionError),
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Sort(#[from] SortError),
    #[error("the stored key row carries no token")]
    MissingToken,
    #[error("the page and size do not fit a skip offset")]
    Pagination,
}
