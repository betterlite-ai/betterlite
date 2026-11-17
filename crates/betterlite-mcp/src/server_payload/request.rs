use super::RequestError;
use crate::server_record::OAuth2Flow;
use betterlite_config::sections::mcp_env_values::StdioPolicy;
use betterlite_db::managed_resource_rows::mcp::McpEnvVar;
use betterlite_db::mcp_stored_contracts::pinned::MCPInfo;
use betterlite_db::mcp_stored_contracts::{MCPAuth, MCPCredentials, MCPTransport};
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::fmt::{Debug, Formatter};
mod tests;
mod validation;
pub type OrderedMap = IndexMap<String, String>;
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MCPApprovalStatus {
    PendingReview,
    Active,
    Rejected,
    Draft,
}
impl MCPApprovalStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PendingReview => "pending_review",
            Self::Active => "active",
            Self::Rejected => "rejected",
            Self::Draft => "draft",
        }
    }
}
const fn default_transport() -> MCPTransport {
    MCPTransport::Sse
}
const fn default_public_internet() -> bool {
    true
}
macro_rules! request_struct {
    ($name:ident, $server_id:ty, $require_auth_type:expr, $($extra:tt)*) => {
        #[derive(Clone, Deserialize, Serialize)]
        pub struct $name {
            pub server_id: $server_id,
            pub server_name: Option<String>,
            pub alias: Option<String>,
            pub description: Option<String>,
            #[serde(default = "default_transport")]
            pub transport: MCPTransport,
            pub auth_type: Option<MCPAuth>,
            pub credentials: Option<MCPCredentials>,
            pub url: Option<String>,
            pub spec_path: Option<String>,
            pub mcp_info: Option<MCPInfo>,
            #[serde(default)]
            pub mcp_access_groups: Vec<String>,
            pub allowed_tools: Option<Vec<String>>,
            pub tool_name_to_display_name: Option<OrderedMap>,
            pub tool_name_to_description: Option<OrderedMap>,
            pub extra_headers: Option<Vec<String>>,
            pub static_headers: Option<OrderedMap>,
            pub env_vars: Option<Vec<McpEnvVar>>,
            pub instructions: Option<String>,
            pub command: Option<String>,
            #[serde(default)]
            pub args: Vec<String>,
            #[serde(default)]
            pub env: OrderedMap,
            pub issuer: Option<String>,
            pub authorization_url: Option<String>,
            pub token_url: Option<String>,
            pub registration_url: Option<String>,
            pub oauth2_flow: Option<OAuth2Flow>,
            pub token_exchange_endpoint: Option<String>,
            pub audience: Option<String>,
            pub subject_token_type: Option<String>,
            pub token_exchange_profile: Option<String>,
            #[serde(default)]
            pub allow_all_keys: bool,
            #[serde(default = "default_public_internet")]
            pub available_on_public_internet: bool,
            #[serde(default)]
            pub delegate_auth_to_upstream: bool,
            #[serde(default)]
            pub oauth_passthrough: bool,
            pub dcr_bridge: Option<bool>,
            #[serde(default)]
            pub per_server_oauth_discovery: bool,
            #[serde(default)]
            pub is_byok: bool,
            #[serde(default)]
            pub byok_description: Vec<String>,
            pub byok_api_key_help_url: Option<String>,
            pub source_url: Option<String>,
            pub timeout: Option<f64>,
            pub max_concurrent_requests: Option<i64>,
            $($extra)*
        }
        impl Debug for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
                formatter
                    .debug_struct(stringify!($name))
                    .field("server_name", &self.server_name)
                    .field("alias", &self.alias)
                    .field("transport", &self.transport)
                    .field("auth_type", &self.auth_type)
                    .finish_non_exhaustive()
            }
        }
        impl $name {
            pub fn from_value(value: &Json, stdio: &StdioPolicy) -> Result<Self, RequestError> {
                validation::check(value, stdio, $require_auth_type)?;
                let text = serde_json::to_string(value)?;
                Ok(serde_json::from_str(&text)?)
            }
        }
    };
}
request_struct!(
    NewMCPServerRequest,
    Option<String>,
    true,
    pub approval_status: Option<String>,
    pub submitted_by: Option<String>,
    pub submitted_at: Option<String>,
);
request_struct!(UpdateMCPServerRequest, String, false,);
#[must_use]
// ast-grep-ignore: no-long-names
pub fn is_per_server_oauth_discovery_eligible(
    auth_type: Option<MCPAuth>,
    oauth2_flow: Option<OAuth2Flow>,
    delegate_auth_to_upstream: bool,
) -> bool {
    auth_type == Some(MCPAuth::Oauth2)
        && oauth2_flow == Some(OAuth2Flow::AuthorizationCode)
        && !delegate_auth_to_upstream
}
