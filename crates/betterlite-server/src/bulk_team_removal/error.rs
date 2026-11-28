use betterlite_auth::auth_key_lookup::jwt::JwtError;
use betterlite_db::key_repository::decode::TokenDecodeError;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::team_repository::repository::TeamQueryError;
use betterlite_db::user_repository::projection::UserRecordError;

use crate::audit_events::record::AuditRecordError;
use crate::key_archive::KeyArchiveError;
use crate::list_api::problem::ManagementProblem;
use crate::team_access::error::TeamAccessError;

#[derive(Debug, thiserror::Error)]
pub enum BulkRemovalError {
    #[error(transparent)]
    Problem(#[from] ManagementProblem),
    #[error(transparent)]
    Access(#[from] TeamAccessError),
    #[error(transparent)]
    Team(#[from] TeamQueryError),
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    UserRecord(#[from] UserRecordError),
    #[error(transparent)]
    KeyRecord(#[from] TokenDecodeError),
    #[error(transparent)]
    Jwt(#[from] JwtError),
    #[error(transparent)]
    Archive(#[from] KeyArchiveError),
    #[error(transparent)]
    Audit(#[from] AuditRecordError),
    #[error("the bulk removal row cannot be encoded or read")]
    Encode(#[from] serde_json::Error),
    #[error("the stored row selected by the bulk removal is missing")]
    MissingRow,
    #[error("the bulk removal transaction did not finish within {seconds} seconds")]
    Timeout { seconds: u64 },
}
