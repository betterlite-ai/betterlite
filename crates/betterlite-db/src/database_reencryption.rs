use betterlite_secrets::error::SecretError;

use crate::encryption_scan::ScanError;

pub mod counting;
pub mod outcome;
pub mod registry;
pub mod runner;
pub mod storage;
pub mod traversal;

mod tests;

// master_key_boot_check.py declares both names at L17-L18; B758 reads them from here
pub const PREVIOUS_KEY_VARIABLE: &str = "LITELLM_MIGRATE_FROM_MASTER_KEY";
pub const SALT_KEY_ENV_VAR: &str = "LITELLM_SALT_KEY";
pub const MAX_CAUSE_CHARS: usize = 300;

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("the database read failed")]
    Read(#[source] tokio_postgres::Error),
    #[error("the database write failed")]
    Write(#[source] tokio_postgres::Error),
    #[error("a stored row in {table}.{column} cannot be read")]
    InvalidRow {
        table: String,
        column: String,
        #[source]
        source: Option<tokio_postgres::Error>,
    },
    #[error("a stored value is not valid JSON")]
    Decode(#[source] ScanError),
    #[error("a scalar stored value is not a string")]
    NotText,
    #[error("encrypting a stored value failed")]
    Encrypt(#[source] SecretError),
    #[error("serializing a stored value failed")]
    Json(#[source] serde_json::Error),
    #[error("{message}")]
    Reported { message: String },
}
