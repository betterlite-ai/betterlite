use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::team_repository::repository::TeamQueryError;

use crate::team_fields::rows::RefreshError;

#[derive(Debug, thiserror::Error)]
pub enum CleanupError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Table(#[from] TableError),
    #[error(transparent)]
    Team(#[from] TeamQueryError),
    #[error("stored model aliases on row {id} are not a JSON object")]
    Aliases { id: i64 },
    #[error(transparent)]
    Refresh(#[from] RefreshError),
}
