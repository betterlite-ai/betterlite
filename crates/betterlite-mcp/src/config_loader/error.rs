use betterlite_db::mcp_stored_contracts::pinned::PinnedError;
use thiserror::Error;

use crate::openapi_tools::error::OpenApiError;
use crate::server_build::BuildError;
use crate::server_payload::PayloadError;
use crate::server_record::ServerRecordError;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error(
        "Invalid config for MCP server '{server_name}': server_id must be a non-empty string (got {raw})."
    )]
    ServerId { server_name: String, raw: String },
    #[error(
        "Invalid config for MCP server '{server_name}': server_id '{server_id}' is already used by MCP server '{claimed_by}'. Each mcp_servers entry needs its own id."
    )]
    CollisionClaimed {
        server_name: String,
        server_id: String,
        claimed_by: String,
    },
    #[error(
        "Invalid config for MCP server '{server_name}': server_id '{server_id}' belongs to a database-backed MCP server. The database entry takes precedence over config.yaml, so this server would never be reachable."
    )]
    CollisionDatabase {
        server_name: String,
        server_id: String,
    },
    #[error(
        "Invalid config for MCP server '{server_name}': server_id '{server_id}' is the server_name or alias of MCP server '{owners}'. Permission entries naming '{server_id}' would resolve to '{server_name}' alone and no longer reach '{owners}'."
    )]
    CollisionIdentifier {
        server_name: String,
        server_id: String,
        owners: String,
    },
    #[error(
        "Invalid config for MCP server '{server_ref}': auth_type oauth2 requires an explicit oauth2_flow (got {flow}). Set oauth2_flow: client_credentials for machine-to-machine servers (the proxy mints a shared token at token_url using client_id/client_secret, no user interaction) or oauth2_flow: authorization_code for interactive servers (per-user tokens via browser sign-in, including delegate_auth_to_upstream)."
    )]
    Oauth2Flow { server_ref: String, flow: String },
    #[error(
        "Invalid config for MCP server '{server_ref}': per_server_oauth_discovery must be a boolean (got {raw})."
    )]
    PerServerDiscoveryType { server_ref: String, raw: String },
    #[error(
        "Invalid config for MCP server '{server_ref}': per_server_oauth_discovery is only supported for auth_type oauth2 with oauth2_flow authorization_code and without delegate_auth_to_upstream."
    )]
    PerServerDiscoveryEligible { server_ref: String },
    #[error(
        "Invalid config for MCP server '{server_ref}': dcr_bridge must be a boolean (got {raw})."
    )]
    DcrBridgeType { server_ref: String, raw: String },
    #[error(
        "Invalid config for MCP server '{server_ref}': dcr_bridge is only supported for auth_type true_passthrough or oauth_delegate (got {auth_type}). The DCR bridge serves gateway-hosted OAuth discovery for the client-forwarded token modes; interactive oauth2 servers already run the gateway authorization-code flow."
    )]
    DcrBridgeAuthType {
        server_ref: String,
        auth_type: String,
    },
    #[error("Invalid config for MCP server '{server_ref}': token_validation must be an object.")]
    TokenValidation { server_ref: String },
    #[error("Invalid config for MCP server '{server_ref}': {source}")]
    McpInfo {
        server_ref: String,
        #[source]
        source: PinnedError,
    },
    #[error("Invalid config for MCP server '{server_ref}': {source}")]
    PinnedTools {
        server_ref: String,
        #[source]
        source: PinnedError,
    },
    #[error(transparent)]
    Name(#[from] PayloadError),
    #[error(transparent)]
    Record(#[from] ServerRecordError),
    #[error(transparent)]
    Build(#[from] BuildError),
    #[error("Failed to register OpenAPI tools for server {server}: {source}")]
    OpenApi {
        server: String,
        #[source]
        source: OpenApiError,
    },
}
