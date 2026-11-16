use betterlite_secrets::error::SecretError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::mcp::table_access::types::McpTableError;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CredentialType {
    #[default]
    Oauth2,
    Byok,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OAuthGrantState {
    Valid,
    Refreshable,
    Absent,
}

impl OAuthGrantState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Refreshable => "refreshable",
            Self::Absent => "absent",
        }
    }
}

// LiteLLM writes this payload into the encrypted column beside plain BYOK keys and tells them apart by "type"
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthCredentialPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(rename = "type", default)]
    pub credential_type: CredentialType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connected_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scopes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_binding_proof: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpUserCredentialListItem {
    pub user_id: String,
    pub credential_type: CredentialType,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub connected_at: Option<String>,
    pub updated_at: String,
}

pub struct StoreOAuthCredential<'a> {
    pub user_id: &'a str,
    pub server_id: &'a str,
    pub access_token: &'a str,
    pub refresh_token: Option<&'a str>,
    pub expires_in: Option<i64>,
    pub scopes: Option<&'a [String]>,
    pub skip_byok_guard: bool,
    pub identity_binding_proof: Option<&'a str>,
}

#[derive(Debug, Error)]
pub enum CredentialError {
    #[error(transparent)]
    Table(#[from] McpTableError),
    #[error(transparent)]
    Secret(#[from] SecretError),
    #[error("the stored credential payload is not valid JSON")]
    Json(#[source] serde_json::Error),
    #[error("the stored credential row has no {field} text field")]
    MissingField { field: &'static str },
    #[error(
        "the credential for user {user_id} and server {server_id} is not an OAuth2 token; refusing to overwrite it"
    )]
    RefusesByokOverwrite { user_id: String, server_id: String },
    #[error("expires_in of {seconds} seconds does not fit a timestamp")]
    ExpiresInOutOfRange { seconds: i64 },
    #[error("the credential cipher did not return the sealed text")]
    SealedNotText,
}
