mod tests;

use std::collections::BTreeMap;
use std::fmt::{self, Debug, Formatter};

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use secrecy::SecretString;

pub(crate) const REDACTED: &str = "<redacted>";

#[derive(Clone)]
pub struct McpAuthenticatedUser {
    pub user_api_key_auth: Option<UserAPIKeyAuth>,
    pub mcp_auth_header: Option<SecretString>,
    pub mcp_servers: Option<Vec<String>>,
    pub mcp_server_auth_headers: BTreeMap<String, BTreeMap<String, SecretString>>,
    pub oauth2_headers: Option<BTreeMap<String, SecretString>>,
    pub mcp_protocol_version: Option<String>,
    pub raw_headers: Option<BTreeMap<String, SecretString>>,
    pub client_ip: Option<String>,
}

// clippy.toml stops a function at five arguments, so the constructor takes all eight at once
#[derive(Default)]
pub struct McpAuthFields {
    pub user_api_key_auth: Option<UserAPIKeyAuth>,
    pub mcp_auth_header: Option<SecretString>,
    pub mcp_servers: Option<Vec<String>>,
    pub mcp_server_auth_headers: Option<BTreeMap<String, BTreeMap<String, SecretString>>>,
    pub oauth2_headers: Option<BTreeMap<String, SecretString>>,
    pub mcp_protocol_version: Option<String>,
    pub raw_headers: Option<BTreeMap<String, SecretString>>,
    pub client_ip: Option<String>,
}

impl McpAuthenticatedUser {
    #[must_use]
    pub fn new(fields: McpAuthFields) -> Self {
        Self {
            user_api_key_auth: fields.user_api_key_auth,
            mcp_auth_header: fields.mcp_auth_header,
            mcp_servers: fields.mcp_servers,
            // LiteLLM's constructor turns an absent per-server header map into an empty one
            mcp_server_auth_headers: fields.mcp_server_auth_headers.unwrap_or_default(),
            oauth2_headers: fields.oauth2_headers,
            mcp_protocol_version: fields.mcp_protocol_version,
            raw_headers: fields.raw_headers,
            client_ip: fields.client_ip,
        }
    }
}

impl Debug for McpAuthenticatedUser {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        // every header map carries client credentials, so Debug reports their size only
        let oauth2_headers = self.oauth2_headers.as_ref().map(BTreeMap::len);
        let raw_headers = self.raw_headers.as_ref().map(BTreeMap::len);
        f.debug_struct("McpAuthenticatedUser")
            .field("user_api_key_auth", &self.user_api_key_auth)
            .field(
                "mcp_auth_header",
                &self.mcp_auth_header.as_ref().map(|_| REDACTED),
            )
            .field("mcp_servers", &self.mcp_servers)
            .field(
                "mcp_server_auth_headers",
                &self.mcp_server_auth_headers.len(),
            )
            .field("oauth2_headers", &oauth2_headers)
            .field("mcp_protocol_version", &self.mcp_protocol_version)
            .field("raw_headers", &raw_headers)
            .field("client_ip", &self.client_ip)
            .finish()
    }
}
