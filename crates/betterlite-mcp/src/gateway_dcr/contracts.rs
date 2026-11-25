mod tests;

use crate::types::SessionAudience;

pub const GATEWAY_DCR_CLIENT_ID_PREFIX: &str = "llm_dcrc_";
pub const GATEWAY_AUTH_CODE_PREFIX: &str = "llm_gcode_";
pub const CONNECT_FLOW_COOKIE_PREFIX: &str = "mcp_connect_flow_";
pub const CONNECT_FLOW_TTL_SECONDS: i64 = 600;
pub const GATEWAY_AUTH_CODE_TTL_SECONDS: i64 = 120;
// LiteLLM gives a copy-pasted code 5 minutes so a manual delivery has time to reach the token endpoint
pub const MANUAL_DELIVERY_AUTH_CODE_TTL_SECONDS: i64 = 300; // ast-grep-ignore: no-long-names
pub const CLAIM_TTL_BUFFER_SECONDS: i64 = 60;

pub const CODE_CLAIM_KEY_PREFIX: &str = "mcp_gateway_dcr_code_used:";
pub const FLOW_CLAIM_KEY_PREFIX: &str = "mcp_gateway_dcr_flow_used:";
pub const REFRESH_CLAIM_KEY_PREFIX: &str = "mcp_gateway_dcr_refresh_used:";

pub const MAX_REDIRECT_URIS: usize = 4;
pub const MAX_REDIRECT_URI_LENGTH: usize = 256;
pub const MAX_CLIENT_ID_LENGTH: usize = 2048;
pub const MAX_STATE_LENGTH: usize = 1024;
pub const MIN_CODE_VERIFIER_LENGTH: usize = 43;
pub const MAX_CODE_VERIFIER_LENGTH: usize = 128;

pub const CLIENT_RECORD_DEBUG_KEY: &str = "gateway_dcr_client";
pub const CONNECT_FLOW_DEBUG_KEY: &str = "gateway_connect_flow";
pub const AUTH_CODE_DEBUG_KEY: &str = "gateway_authorization_code";

pub const DB_UNAVAILABLE_DESCRIPTION: &str = "the gateway database is unavailable; retry";
pub const DB_FAULTED_DESCRIPTION: &str = "the gateway database reported a fault that is not a transient outage; retrying will not help until the gateway deployment is repaired";

pub const PROXY_API_AUDIENCE: SessionAudience = SessionAudience::ProxyApi;

pub const TOKEN_EXCHANGE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
pub const ACCESS_TOKEN_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:access_token";
pub const SUBJECT_TOKEN_TYPES: [&str; 3] = [
    "urn:ietf:params:oauth:token-type:jwt",
    "urn:ietf:params:oauth:token-type:id_token",
    ACCESS_TOKEN_TOKEN_TYPE,
];

#[must_use]
pub fn supported_grant_types(token_exchange_available: bool) -> Vec<&'static str> {
    let mut grants = vec!["authorization_code", "refresh_token"];
    if token_exchange_available {
        grants.push(TOKEN_EXCHANGE_GRANT_TYPE);
    }
    grants
}
