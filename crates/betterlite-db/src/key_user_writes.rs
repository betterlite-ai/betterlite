mod assignments;
pub mod columns;
pub mod dispatch;
pub mod hash;
pub mod json_text;
pub mod retry;
mod rows;
mod sql;
mod tests;
mod updates;
mod writes;

use secrecy::SecretString;
use thiserror::Error;
use tokio_postgres::error::SqlState;

use crate::identity_rows::key::LiteLLMVerificationToken;
use crate::identity_rows::user::LiteLLMUserTable;
use crate::key_repository::decode::TokenDecodeError;
use crate::migration_ledger::lock::JitterSource;
use crate::postgres_store::errors::DbError;
use crate::tool_spend::RetryPause;
use crate::user_repository::projection::UserRecordError;

use columns::{UserColumn, UserUpdateOps, UserWrite};
use retry::{FailureSink, WriteClock};

#[derive(Debug, Error)]
pub enum WriteRowError {
    #[error("the stored key row cannot be decoded")]
    Key(#[from] TokenDecodeError),
    #[error("the stored user row cannot be decoded")]
    User(#[from] UserRecordError),
    #[error("the stored row is not a JSON object")]
    NotAnObject,
}

#[derive(Debug, Error)]
pub enum KeyUserWriteError {
    #[error(transparent)]
    Database { source: DbError<WriteRowError> },
    #[error(transparent)]
    Row(#[from] WriteRowError),
    #[error("the write value cannot be encoded")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the {field} value is missing from the write payload")]
    MissingField { field: &'static str },
    #[error("the custom update for {column} needs a number")]
    NonNumericUpdate { column: &'static str },
    #[error(
        "Foreign Key Constraint failed. Organization ID={organization_id} does not exist in LiteLLM_OrganizationTable. Create via `/organization/new`."
    )]
    OrganizationMissing {
        organization_id: String,
        #[source]
        source: DbError<WriteRowError>,
    },
}

impl KeyUserWriteError {
    #[must_use]
    pub const fn http_status(&self) -> Option<u16> {
        match self {
            Self::OrganizationMissing { .. } => Some(400),
            Self::Database { .. }
            | Self::Row(_)
            | Self::Encode { .. }
            | Self::MissingField { .. }
            | Self::NonNumericUpdate { .. } => None,
        }
    }
}

pub struct WritePorts<'ports> {
    pub clock: &'ports dyn WriteClock,
    pub jitter: &'ports dyn JitterSource,
    pub pause: &'ports dyn RetryPause,
    pub sink: &'ports dyn FailureSink,
}

pub struct KeyUserWrites<'service, Client> {
    pub client: &'service Client,
    pub ports: WritePorts<'service>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserUpdate {
    pub user_id: Option<String>,
    pub fields: UserWrite,
    pub update_fields: Option<UserWrite>,
    pub custom_update_fields: Option<UserUpdateOps>,
}

#[derive(Debug)]
pub struct KeyUpdateResult {
    pub token: SecretString,
    pub data: Option<LiteLLMVerificationToken>,
}

#[derive(Debug)]
pub struct UserUpdateResult {
    pub user_id: String,
    pub data: LiteLLMUserTable,
}

const ORGANIZATION_FOREIGN_KEY: &str = "LiteLLM_UserTable_organization_id_fkey";

// only the named organization foreign key becomes the 400 that points at /organization/new; any other failure keeps its type
fn translate_organization_error(error: KeyUserWriteError, fields: &UserWrite) -> KeyUserWriteError {
    let KeyUserWriteError::Database { source } = error else {
        return error;
    };
    if !is_named_organization_failure(&source) {
        return KeyUserWriteError::Database { source };
    }
    match fields.text(UserColumn::OrganizationId) {
        Some(organization_id) => KeyUserWriteError::OrganizationMissing {
            organization_id: organization_id.to_owned(),
            source,
        },
        None => KeyUserWriteError::Database { source },
    }
}

fn is_named_organization_failure(error: &DbError<WriteRowError>) -> bool {
    match error {
        DbError::SqlState {
            code, constraint, ..
        } => {
            code == &SqlState::FOREIGN_KEY_VIOLATION
                && constraint.as_deref() == Some(ORGANIZATION_FOREIGN_KEY)
        }
        DbError::NoDbConnected
        | DbError::Pool { .. }
        | DbError::Backend { .. }
        | DbError::InvalidRecord { .. }
        | DbError::CreateNull
        | DbError::UniqueViolation { .. } => false,
    }
}
