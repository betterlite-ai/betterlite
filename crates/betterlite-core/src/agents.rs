use betterlite_auth::identity;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_with::rust::double_option;
use std::collections::BTreeMap;
pub mod json;
mod tests;
mod validation;

use json::{AgentJson, SecurityRequirements};
pub const CALLER_USER_ID_HEADER: &str = "x-litellm-user-id";
pub const CALLER_TEAM_ID_HEADER: &str = "x-litellm-team-id";
macro_rules! wire_enum {
    ($(#[$attr:meta])* $name:ident { $($variants:tt)* }) => {
        #[derive(Clone, Debug, Deserialize, Serialize)]
        $(#[$attr])*
        pub enum $name { $($variants)* }
    };
}
macro_rules! record {
    ($name:ident { $($fields:tt)* }) => {
        #[derive(Clone, Debug, Deserialize, Serialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name { $($fields)* }
    };
}
macro_rules! response {
    ($name:ident, $case:literal { $($required:tt)* } { $($field:ident: $ty:ty,)* }) => {
        record! { $name { $($required)* $(pub $field: Option<$ty>,)* } }
    };
}
macro_rules! dictionary {
    ($name:ident, $case:literal { $($required:tt)* } { $($(#[$attr:meta])* $field:ident: $ty:ty,)* }) => {
        #[derive(Clone, Debug, Deserialize, Serialize)]
        #[serde(rename_all = $case, deny_unknown_fields)]
        pub struct $name {
            $($required)*
            $($(#[$attr])* #[serde(default, skip_serializing_if = "Option::is_none", with = "double_option")]
            pub $field: Option<Option<$ty>>,)*
        }
    };
}
dictionary! { AgentProvider, "camelCase" {} {
    organization: String,
    url: String,
}}
dictionary! { AgentExtension, "camelCase" {} {
    uri: String,
    description: String,
    required: bool,
    params: BTreeMap<String, AgentJson>,
}}
dictionary! { AgentCapabilities, "camelCase" {} {
    streaming: bool,
    push_notifications: bool,
    state_transition_history: bool,
    extensions: Vec<AgentExtension>,
}}
dictionary! { AgentSkill, "camelCase" {} {
    id: String,
    name: String,
    description: String,
    tags: Vec<String>,
    examples: Vec<String>,
    input_modes: Vec<String>,
    output_modes: Vec<String>,
    security: SecurityRequirements,
}}
dictionary! { AgentInterface, "camelCase" {} {
    url: String,
    transport: String,
}}
dictionary! { AgentCardSignature, "camelCase" {} {
    protected: String,
    signature: String,
    header: BTreeMap<String, AgentJson>,
}}
macro_rules! cards {
    ($name:ident { $($extra:tt)* }) => {
        dictionary! { $name, "camelCase" { $($extra)* } {
            protocol_version: String,
            name: String,
            description: String,
            url: String,
            version: String,
            capabilities: AgentCapabilities,
            default_input_modes: Vec<String>,
            default_output_modes: Vec<String>,
            skills: Vec<AgentSkill>,
            preferred_transport: String,
            additional_interfaces: Vec<AgentInterface>,
            icon_url: String,
            provider: AgentProvider,
            documentation_url: String,
            security_schemes: BTreeMap<String, SecurityScheme>,
            security: SecurityRequirements,
            #[serde(rename = "supportsAuthenticatedExtendedCard")] supports_authenticated_card: bool,
            signatures: Vec<AgentCardSignature>,
        }}
    };
}
cards!(AgentCard {});
cards!(AugmentedAgentCard { #[serde(rename = "is_public")] pub is_public: bool, });
dictionary! { AgentObjectPermission, "snake_case" {} {
    mcp_servers: Vec<String>,
    mcp_access_groups: Vec<String>,
    mcp_toolsets: Vec<String>,
    mcp_tool_permissions: BTreeMap<String, Vec<String>>,
    models: Vec<String>,
    agents: Vec<String>,
}}
macro_rules! registration {
    ($name:ident, $boundary:ident, $permission:ty { $($required:tt)* } { $($extra:tt)* }) => {
        $boundary! { $name, "snake_case" { $($required)* } {
            $($extra)*
            litellm_params: BTreeMap<String, AgentJson>,
            object_permission: $permission,
            tpm_limit: i64,
            rpm_limit: i64,
            session_tpm_limit: i64,
            session_rpm_limit: i64,
            static_headers: BTreeMap<String, String>,
            extra_headers: Vec<String>,
            access_group_ids: Vec<String>,
            kill_switch: AgentKillSwitchConfig,
        }}
    };
}
macro_rules! requests {
    ($name:ident { $($required:tt)* } { $($extra:tt)* }) => {
        registration! { $name, dictionary, AgentObjectPermission { $($required)* } {
            $($extra)*
            identity: identity::EntraIdentityConfig,
            enabled: bool,
            execution_mode: identity::ExecutionMode,
            agent_card_params: AgentCard,
        }}
    };
}
requests!(AgentConfig { pub agent_name: String, } {});
requests!(PatchAgentRequest {} { agent_name: String, });
wire_enum! { #[serde(tag = "type")] SecurityScheme {
    #[serde(rename = "apiKey")] ApiKey(SecurityApiKey),
    #[serde(rename = "http")] Http(SecurityHttp),
    #[serde(rename = "mutualTLS")] MutualTls(SecurityMutualTls),
    #[serde(rename = "oauth2")] Oauth2(SecurityOauth2),
    #[serde(rename = "openIdConnect")] OpenIdConnect(SecurityOpenIdConnect),
}}
dictionary! { SecurityApiKey, "camelCase" { #[serde(rename = "in_")] pub location: SecurityLocation, pub name: String, } {
    description: String,
}}
dictionary! { SecurityHttp, "camelCase" { pub scheme: String, } {
    bearer_format: String,
    description: String,
}}
dictionary! { SecurityMutualTls, "camelCase" {} { description: String, }}
dictionary! { SecurityOauth2, "camelCase" { pub flows: OAuthFlows, } {
    oauth2_metadata_url: String,
    description: String,
}}
dictionary! { OAuthFlows, "camelCase" {} {
    authorization_code: BTreeMap<String, AgentJson>,
    client_credentials: BTreeMap<String, AgentJson>,
    implicit: BTreeMap<String, AgentJson>,
    password: BTreeMap<String, AgentJson>,
}}
dictionary! { SecurityOpenIdConnect, "camelCase" { pub open_id_connect_url: String, } { description: String, }}
wire_enum! { #[serde(rename_all = "lowercase")] SecurityLocation { Query, Header, Cookie } }
fn default_api_key_header() -> String {
    let value = "x-api-key";
    String::from(value)
}
wire_enum! { #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)] AgentKillSwitchAuth {
    Bearer { #[serde(skip_serializing)] token: SecretString },
    ApiKey {
        #[serde(default = "default_api_key_header")] header_name: String,
        #[serde(skip_serializing)] api_key: SecretString,
    },
    Basic { username: String, #[serde(skip_serializing)] password: SecretString },
}}
wire_enum! { #[derive(Default, PartialEq, Eq)] #[serde(rename_all = "UPPERCASE")] AgentKillSwitchMethod {
    #[default] Post, Put, Patch, Delete, Get,
}}
record! { AgentKillSwitchConfig {
    #[serde(deserialize_with = "validation::absolute_http_url")] pub url: String,
    #[serde(default)] pub method: AgentKillSwitchMethod,
    #[serde(default)] pub headers: BTreeMap<String, String>,
    #[serde(default)] pub query_params: BTreeMap<String, String>,
    pub body: Option<BTreeMap<String, AgentJson>>,
    pub auth: Option<AgentKillSwitchAuth>,
}}
record! { AgentKillSwitchResult {
    pub agent_id: String,
    pub url: String,
    pub method: AgentKillSwitchMethod,
    pub status_code: Option<u16>,
    pub response_body: Option<String>,
    pub error: Option<String>,
}}
impl AgentKillSwitchResult {
    #[must_use]
    pub fn succeeded(&self) -> bool {
        let status = self.status_code;
        status.is_some_and(|value| (200..300).contains(&value))
    }
}
record! { AgentCaller {
    pub user_id: Option<String>,
    pub team_id: Option<String>,
}}
record! { AgentKeySummary {
    pub token: String,
    pub key_alias: Option<String>,
    pub key_name: Option<String>,
}}
registration! { AgentResponse, response, BTreeMap<String, AgentJson> {
    pub identity: Option<identity::AgentIdentityBinding>,
    #[serde(default)] pub identity_managed: bool,
    #[serde(default)] pub enabled: identity::ActiveState,
    #[serde(default)] pub execution_mode: identity::ExecutionMode,
    #[serde(default)] pub jwt_auth_configured: bool,
    pub agent_id: String,
    pub agent_name: String,
    pub agent_card_params: BTreeMap<String, AgentJson>,
} {
    spend: Decimal,
    keys: Vec<AgentKeySummary>,
    search_score: Decimal,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    created_by: String,
    updated_by: String,
}}
record! { AgentMakePublicResponse {
    pub message: String,
    pub public_agent_groups: Vec<String>,
    pub updated_by: String,
}}
record! { MakeAgentsPublicRequest {
    pub agent_ids: Vec<String>,
}}
