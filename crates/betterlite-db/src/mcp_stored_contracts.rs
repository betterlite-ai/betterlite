use std::collections::BTreeMap;
use std::fmt::{self, Debug, Formatter};

use secret::StoredSecret;
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, rust::double_option};
use strum::EnumString;
pub const MAX_STORED_BYTES: usize = 0x0010_0000;
pub const MAX_STORED_DEPTH: usize = 64;

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

impl Debug for JsonValue {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        // Stored input can contain credentials before it reaches the typed credential record.
        f.write_str("[REDACTED]")
    }
}

#[derive(Debug, Clone, Copy, DeserializeFromStr, Serialize, PartialEq, Eq, EnumString)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum MCPTransport {
    Sse,
    Http,
    Stdio,
}

#[derive(Debug, Clone, Copy, DeserializeFromStr, Serialize, PartialEq, Eq, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum MCPAuth {
    None,
    ApiKey,
    BearerToken,
    Basic,
    Authorization,
    Oauth2,
    AwsSigv4,
    Token,
    Oauth2TokenExchange,
    Oauth2IdJag,
    TruePassthrough,
    OauthDelegate,
}

pub type MCPTransportType = MCPTransport;
pub type MCPAuthType = Option<MCPAuth>;
#[derive(Debug, Clone, Copy, DeserializeFromStr, Serialize, PartialEq, Eq, EnumString)]
pub enum MCPLegacyVersion {
    #[serde(rename = "2024-11-05")]
    #[strum(serialize = "2024-11-05")]
    Nov2024,
    #[serde(rename = "2025-03-26")]
    #[strum(serialize = "2025-03-26")]
    Mar2025,
    #[serde(rename = "2025-06-18")]
    #[strum(serialize = "2025-06-18")]
    Jun2025,
    #[serde(rename = "2025-11-25")]
    #[strum(serialize = "2025-11-25")]
    Nov2025,
}

pub const MCP_LEGACY_VERSIONS: [MCPLegacyVersion; 4] = [
    MCPLegacyVersion::Nov2024,
    MCPLegacyVersion::Mar2025,
    MCPLegacyVersion::Jun2025,
    MCPLegacyVersion::Nov2025,
];

#[derive(Debug, Clone, Copy, DeserializeFromStr, Serialize, PartialEq, Eq, EnumString)]
pub enum MCPUpstreamProtocol {
    #[serde(rename = "auto")]
    #[strum(serialize = "auto")]
    Auto,
    #[serde(rename = "2024-11-05")]
    #[strum(serialize = "2024-11-05")]
    Nov2024,
    #[serde(rename = "2025-03-26")]
    #[strum(serialize = "2025-03-26")]
    Mar2025,
    #[serde(rename = "2025-06-18")]
    #[strum(serialize = "2025-06-18")]
    Jun2025,
    #[serde(rename = "2025-11-25")]
    #[strum(serialize = "2025-11-25")]
    Nov2025,
}

#[derive(Debug, Clone, Copy, DeserializeFromStr, Serialize, PartialEq, Eq, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum MCPTokenEndpointAuthMethod {
    ClientSecretBasic,
    ClientSecretPost,
}

type Maybe<T> = Option<Option<T>>;

#[derive(Clone, Deserialize, Serialize)]
pub struct MCPCredentials {
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub auth_value: Maybe<StoredSecret>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub client_id: Maybe<String>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub client_secret: Maybe<StoredSecret>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub scopes: Maybe<Vec<String>>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub aws_access_key_id: Maybe<StoredSecret>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub aws_secret_access_key: Maybe<StoredSecret>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub aws_session_token: Maybe<StoredSecret>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub aws_region_name: Maybe<String>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub aws_service_name: Maybe<String>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub aws_role_name: Maybe<String>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub aws_session_name: Maybe<String>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub audience: Maybe<String>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub token_exchange_endpoint: Maybe<String>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub subject_token_type: Maybe<String>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub id_jag_resource_token_endpoint: Maybe<String>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub id_jag_resource: Maybe<String>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub upstream_resource: Maybe<String>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub upstream_token_header: Maybe<String>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub client_private_key: Maybe<StoredSecret>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub client_private_key_id: Maybe<String>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub client_assertion_signing_alg: Maybe<String>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub token_endpoint_auth_method: Maybe<MCPTokenEndpointAuthMethod>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub redirect_uris: Maybe<Vec<String>>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub token_exchange_profile: Maybe<String>,
}

impl Debug for MCPCredentials {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("MCPCredentials").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ToolDefinition {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub input_schema: BTreeMap<String, JsonValue>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(transparent)]
pub struct PinnedMCPTool(ToolDefinition);

impl AsRef<ToolDefinition> for PinnedMCPTool {
    fn as_ref(&self) -> &ToolDefinition {
        &self.0
    }
}

pub mod decode;
pub mod pinned;
pub mod secret;

mod tests;
