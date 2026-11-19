use thiserror::Error;

#[derive(Debug, Error)]
pub enum OAuthCacheError {
    #[error(
        "MCP server '{server_id}' missing required OAuth2 fields: client_id={client_id}, client_secret={client_secret}, token_url={token_url}"
    )]
    MissingFields {
        server_id: String,
        client_id: bool,
        client_secret: bool,
        token_url: bool,
    },
    #[error("OAuth2 token request for MCP server '{server_id}' failed with status {status_code}")]
    Denied { server_id: String, status_code: u16 },
    #[error("OAuth2 token request for MCP server '{server_id}' failed: {detail}")]
    Unreachable { server_id: String, detail: String },
    #[error("OAuth2 token response for MCP server '{server_id}' missing 'access_token'")]
    MissingAccessToken { server_id: String },
    #[error("OAuth2 token request for MCP server '{server_id}' could not be built")]
    RequestBuild { server_id: String },
}
