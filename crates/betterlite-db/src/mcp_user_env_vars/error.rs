use betterlite_secrets::error::SecretError;
use thiserror::Error;

use crate::mcp::table_access::types::McpTableError;

#[derive(Debug, Error)]
pub enum UserEnvVarsError {
    #[error(transparent)]
    Database(#[from] McpTableError),
    #[error("the per-user env vars could not be encoded as JSON")]
    Encode(#[source] serde_json::Error),
    #[error("the per-user env vars could not be sealed")]
    Encrypt(#[source] SecretError),
    #[error("the sealed per-user env vars are not text")]
    NotText,
}
