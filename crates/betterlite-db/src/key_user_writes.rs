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

use crate::identity_rows::key::LiteLLMVerificationToken;
use crate::identity_rows::user::LiteLLMUserTable;
use crate::key_repository::decode::TokenDecodeError;
use crate::migration_ledger::lock::JitterSource;
use crate::postgres_store::errors::DbError;
use crate::tool_spend::RetryPause;
use crate::user_repository::projection::UserRecordError;

use columns::UserWrite;
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
    #[error(
        "Foreign Key Constraint failed. Organization ID={organization_id} does not exist in LiteLLM_OrganizationTable. Create via `/organization/new`."
    )]
    OrganizationMissing { organization_id: String },
}

impl KeyUserWriteError {
    #[must_use]
    pub const fn http_status(&self) -> Option<u16> {
        match self {
            Self::OrganizationMissing { .. } => Some(400),
            Self::Database { .. }
            | Self::Row(_)
            | Self::Encode { .. }
            | Self::MissingField { .. } => None,
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
    pub custom_update_fields: Option<UserWrite>,
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
