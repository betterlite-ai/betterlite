pub mod flow;
pub mod issuer;
pub mod log;
mod tests;

use async_trait::async_trait;
use betterlite_db::mcp_server_encoding::credentials::{
    CredentialBlob, credentials_blob_to_dict, decrypt_credentials,
};
use betterlite_db::mcp_stored_contracts::{JsonValue, MCPAuth};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_mcp::oauth_resource::python_url;
use betterlite_mcp::oauth_resource::resource_identity::canonicalize_url_identity;
use betterlite_mcp::server_record::OAuth2Flow;

pub const FLOW_BACKFILL_ACTOR: &str = "oauth2_flow_backfill";
pub const DISCOVERY_ACTOR: &str = "mcp_oauth_discovery";
pub const ISSUER_STAMP_BACKFILL_ACTOR: &str = "mcp_oauth_issuer_stamp_backfill";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BackfillRule {
    PerUserTokens,
    AuthorizationUrl,
    RegistrationUrl,
    AmbiguousM2mShape,
    InteractiveDefault,
}

impl BackfillRule {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PerUserTokens => "per_user_tokens",
            Self::AuthorizationUrl => "authorization_url",
            Self::RegistrationUrl => "registration_url",
            Self::AmbiguousM2mShape => "ambiguous_m2m_shape",
            Self::InteractiveDefault => "interactive_default",
        }
    }
}

#[must_use]
pub const fn flow_name(flow: OAuth2Flow) -> &'static str {
    match flow {
        OAuth2Flow::ClientCredentials => "client_credentials",
        OAuth2Flow::AuthorizationCode => "authorization_code",
    }
}

#[derive(Debug, Clone)]
pub struct FlowCandidateRow {
    pub server_id: String,
    pub authorization_url: Option<String>,
    pub registration_url: Option<String>,
    pub token_url: Option<String>,
    pub credentials: Option<JsonValue>,
}

#[derive(Debug, Clone)]
pub struct UserCredentialRow {
    pub server_id: String,
    pub credential_b64: String,
}

#[derive(Debug, Clone)]
pub struct IssuerCandidateRow {
    pub server_id: String,
    pub alias: Option<String>,
    pub server_name: Option<String>,
    pub auth_type: Option<MCPAuth>,
    pub issuer: Option<String>,
    pub authorization_url: Option<String>,
    pub token_url: Option<String>,
    pub registration_url: Option<String>,
    pub updated_by: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlowClassification {
    pub flow: Option<OAuth2Flow>,
    pub rule: BackfillRule,
}

pub struct FlowSignals<'row> {
    pub has_per_user_tokens: bool,
    pub authorization_url: Option<&'row str>,
    pub registration_url: Option<&'row str>,
    pub token_url: Option<&'row str>,
    pub credentials: Option<&'row CredentialBlob>,
}

// Signal strength is fixed: per-user tokens, then an authorization endpoint, then DCR, then the ambiguous shape, then the default.
#[must_use]
pub fn classify_null_flow_row(signals: &FlowSignals<'_>) -> FlowClassification {
    let authorization_code = Some(OAuth2Flow::AuthorizationCode);
    if signals.has_per_user_tokens {
        return FlowClassification {
            flow: authorization_code,
            rule: BackfillRule::PerUserTokens,
        };
    }
    if signals
        .authorization_url
        .is_some_and(|value| !value.is_empty())
    {
        return FlowClassification {
            flow: authorization_code,
            rule: BackfillRule::AuthorizationUrl,
        };
    }
    if signals
        .registration_url
        .is_some_and(|value| !value.is_empty())
    {
        return FlowClassification {
            flow: authorization_code,
            rule: BackfillRule::RegistrationUrl,
        };
    }
    if signals.token_url.is_some_and(|value| !value.is_empty())
        && has_client_credentials(signals.credentials)
    {
        return FlowClassification {
            flow: None,
            rule: BackfillRule::AmbiguousM2mShape,
        };
    }
    FlowClassification {
        flow: authorization_code,
        rule: BackfillRule::InteractiveDefault,
    }
}

#[must_use]
pub fn decrypted_credentials(raw: Option<&JsonValue>, cipher: &DbCipher) -> Option<CredentialBlob> {
    let mut blob = credentials_blob_to_dict(raw?).ok()?;
    decrypt_credentials(&mut blob, cipher);
    Some(blob)
}

#[must_use]
pub fn is_stamped_issuer_row(row: &IssuerCandidateRow) -> bool {
    if row.updated_by.as_deref() != Some(DISCOVERY_ACTOR) {
        return false;
    }
    let Some(issuer) = row
        .issuer
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    if !matches!(
        row.auth_type,
        Some(MCPAuth::Oauth2 | MCPAuth::TruePassthrough | MCPAuth::OauthDelegate)
    ) {
        return false;
    }
    let configured: Vec<&str> = [
        row.authorization_url.as_deref(),
        row.token_url.as_deref(),
        row.registration_url.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .collect();
    if configured.is_empty() {
        return false;
    }
    let Some(issuer_origin) = origin(issuer) else {
        return false;
    };
    configured
        .iter()
        .all(|endpoint| origin(endpoint).as_deref() == Some(issuer_origin.as_str()))
}

// The scheme-and-authority identity of a URL, built on the shared canonicalizer so default ports and host case match.
#[must_use]
pub fn origin(url: &str) -> Option<String> {
    let canonical = canonicalize_url_identity(url).ok()?;
    let parsed = python_url::parse(&canonical).ok()?;
    if parsed.scheme.is_empty() || parsed.netloc.is_empty() {
        return None;
    }
    Some(format!("{}://{}", parsed.scheme, parsed.netloc))
}

#[async_trait]
pub trait FlowBackfillTable: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    async fn null_flow_rows(&self) -> Result<Vec<FlowCandidateRow>, Self::Error>;

    async fn user_credentials(
        &self,
        server_ids: &[String],
    ) -> Result<Vec<UserCredentialRow>, Self::Error>;

    // Only rows whose oauth2_flow is still null are stamped, so a concurrent write wins.
    async fn stamp_flow(&self, server_ids: &[String], flow: OAuth2Flow) -> Result<(), Self::Error>;
}

#[async_trait]
pub trait IssuerBackfillTable: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    async fn discovery_stamped_rows(&self) -> Result<Vec<IssuerCandidateRow>, Self::Error>;

    async fn clear_issuer(&self, server_id: &str) -> Result<(), Self::Error>;
}

fn has_client_credentials(credentials: Option<&CredentialBlob>) -> bool {
    credentials.is_some_and(|blob| {
        blob.get("client_id")
            .is_some_and(|value| matches!(value, JsonValue::String(text) if !text.is_empty()))
            && blob
                .get("client_secret")
                .is_some_and(|value| matches!(value, JsonValue::String(text) if !text.is_empty()))
    })
}
