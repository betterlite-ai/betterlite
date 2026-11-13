pub mod blank;
pub mod credentials;
pub mod dump;
pub mod env_vars;
pub mod prepare;
pub mod request;
mod tests;

use betterlite_secrets::error::SecretError;

#[derive(Debug, thiserror::Error)]
pub enum McpEncodingError {
    #[error("MCP server data is not valid JSON")]
    Json(#[source] serde_json::Error),
    #[error("MCP credentials are not a JSON object")]
    CredentialsNotObject,
    #[error("MCP env var entry is not a JSON object")]
    EnvVarNotObject,
    #[error("MCP env var {name} cannot carry a null {field}")]
    EnvVarNull { name: String, field: &'static str },
    #[error("the stored MCP row env_vars is neither a list nor a JSON string")]
    EnvVarsShape,
    #[error("the MCP secret map for {field} holds a value that is not a string")]
    SecretMapValue { field: &'static str },
    #[error(transparent)]
    Secret(#[from] SecretError),
}
