use betterlite_db::mcp_server_encoding::McpEncodingError;
use betterlite_db::mcp_stored_contracts::decode::McpStoredDecodeError;
use betterlite_secrets::error::SecretError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RotationCodecError {
    #[error(transparent)]
    Blob(#[from] McpStoredDecodeError),
    #[error("the stored MCP value is not valid UTF-8")]
    Utf8(#[source] std::str::Utf8Error),
    #[error(transparent)]
    Encoding(#[from] McpEncodingError),
    #[error(transparent)]
    Secret(#[from] SecretError),
    #[error("the rotated MCP value cannot be serialized")]
    Json(#[source] serde_json::Error),
}

#[derive(Debug, Error)]
pub enum RotationError<Store> {
    #[error(transparent)]
    Codec(#[from] RotationCodecError),
    #[error("the MCP rotation store failed")]
    Store(#[from] Store),
}
