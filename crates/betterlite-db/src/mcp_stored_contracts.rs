use std::collections::BTreeMap;
use std::fmt::{self, Debug, Formatter};

use secrecy::{ExposeSecret as _, SecretString};
use serde::de::Error as _;
use serde::{Deserialize, Serialize};
pub const MAX_STORED_BYTES: usize = 0x0010_0000;
pub const MAX_STORED_DEPTH: usize = 64;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MCPTransport {
    Sse,
    Http,
    Stdio,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
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
pub type MCPLegacyVersion = String;

pub const MCP_LEGACY_VERSIONS: [&str; 4] = ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MCPUpstreamProtocol {
    Auto,
    Legacy(String),
}

impl<'de> Deserialize<'de> for MCPUpstreamProtocol {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value == "auto" {
            return Ok(Self::Auto);
        }
        if MCP_LEGACY_VERSIONS.contains(&value.as_str()) {
            return Ok(Self::Legacy(value));
        }
        Err(D::Error::custom("unsupported MCP protocol version"))
    }
}

impl Serialize for MCPUpstreamProtocol {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Auto => serializer.serialize_str("auto"),
            Self::Legacy(value) => serializer.serialize_str(value),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MCPTokenEndpointAuthMethod {
    ClientSecretBasic,
    ClientSecretPost,
}

type Maybe<T> = Option<Option<T>>;

fn deserialize_maybe<'de, D, T>(deserializer: D) -> Result<Maybe<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Some(Option::<T>::deserialize(deserializer)?))
}

#[derive(Clone)]
pub struct StoredSecret(SecretString);

impl<'de> Deserialize<'de> for StoredSecret {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Self(SecretString::deserialize(deserializer)?))
    }
}

impl Serialize for StoredSecret {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.0.expose_secret())
    }
}

impl Debug for StoredSecret {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl StoredSecret {
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        self.0.expose_secret()
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct MCPCredentials {
    #[serde(
        default,
        deserialize_with = "deserialize_maybe",
        skip_serializing_if = "Option::is_none"
    )]
    pub auth_value: Maybe<StoredSecret>,
    #[serde(
        default,
        deserialize_with = "deserialize_maybe",
        skip_serializing_if = "Option::is_none"
    )]
    pub client_id: Maybe<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_maybe",
        skip_serializing_if = "Option::is_none"
    )]
    pub client_secret: Maybe<StoredSecret>,
    #[serde(
        default,
        deserialize_with = "deserialize_maybe",
        skip_serializing_if = "Option::is_none"
    )]
    pub scopes: Maybe<Vec<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_maybe",
        skip_serializing_if = "Option::is_none"
    )]
    pub aws_access_key_id: Maybe<StoredSecret>,
    #[serde(
        default,
        deserialize_with = "deserialize_maybe",
        skip_serializing_if = "Option::is_none"
    )]
    pub aws_secret_access_key: Maybe<StoredSecret>,
    #[serde(
        default,
        deserialize_with = "deserialize_maybe",
        skip_serializing_if = "Option::is_none"
    )]
    pub aws_session_token: Maybe<StoredSecret>,
    #[serde(
        default,
        deserialize_with = "deserialize_maybe",
        skip_serializing_if = "Option::is_none"
    )]
    pub aws_region_name: Maybe<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_maybe",
        skip_serializing_if = "Option::is_none"
    )]
    pub aws_service_name: Maybe<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_maybe",
        skip_serializing_if = "Option::is_none"
    )]
    pub aws_role_name: Maybe<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_maybe",
        skip_serializing_if = "Option::is_none"
    )]
    pub aws_session_name: Maybe<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_maybe",
        skip_serializing_if = "Option::is_none"
    )]
    pub audience: Maybe<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_maybe",
        skip_serializing_if = "Option::is_none"
    )]
    pub token_exchange_endpoint: Maybe<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_maybe",
        skip_serializing_if = "Option::is_none"
    )]
    pub id_jag_resource_token_endpoint: Maybe<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_maybe",
        skip_serializing_if = "Option::is_none"
    )]
    pub id_jag_resource: Maybe<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_maybe",
        skip_serializing_if = "Option::is_none"
    )]
    pub upstream_resource: Maybe<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_maybe",
        skip_serializing_if = "Option::is_none"
    )]
    pub upstream_token_header: Maybe<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_maybe",
        skip_serializing_if = "Option::is_none"
    )]
    pub client_private_key: Maybe<StoredSecret>,
    #[serde(
        default,
        deserialize_with = "deserialize_maybe",
        skip_serializing_if = "Option::is_none"
    )]
    pub client_private_key_id: Maybe<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_maybe",
        skip_serializing_if = "Option::is_none"
    )]
    pub client_assertion_signing_alg: Maybe<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_maybe",
        skip_serializing_if = "Option::is_none"
    )]
    pub token_endpoint_auth_method: Maybe<MCPTokenEndpointAuthMethod>,
    #[serde(
        default,
        deserialize_with = "deserialize_maybe",
        skip_serializing_if = "Option::is_none"
    )]
    pub redirect_uris: Maybe<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_exchange_profile: Maybe<String>,
}

impl Debug for MCPCredentials {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("MCPCredentials").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PinnedMCPTool {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub input_schema: BTreeMap<String, JsonValue>,
}

pub mod decode;
pub mod pinned;

mod tests;
