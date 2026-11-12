pub mod aws_params;
pub mod external_account;

use std::str::{Utf8Error, from_utf8};

use secrecy::zeroize::Zeroize;
use secrecy::{CloneableSecret, ExposeSecret as _, SecretBox, SecretString, SerializableSecret};
use serde::{Deserialize, Serialize};
use serde_json::Error as JsonError;

use external_account::ExternalAccountSource;

pub const MAX_CREDENTIALS_BYTES: usize = 1024 * 1024;
const AUTHORIZED_USER_TYPE: &str = "authorized_user";
const SERVICE_ACCOUNT_TYPE: &str = "service_account";

// SecretBox keeps the key material out of Debug and writes it only into the document the SDK reads
#[derive(Clone, Deserialize, Serialize)]
#[serde(transparent)]
pub struct KeyText(String);

impl Zeroize for KeyText {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

impl CloneableSecret for KeyText {}

impl SerializableSecret for KeyText {}

pub type Secret = SecretBox<KeyText>;

#[derive(Debug, Clone, Deserialize)]
pub struct ServiceAccountSource {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub private_key_id: Option<String>,
    pub private_key: Secret,
    pub client_email: String,
    #[serde(default)]
    pub universe_domain: Option<String>,
}

// the SDK reads project_id and private_key_id as String, so an absent one is written as "" not null
#[derive(Serialize)]
struct ServiceAccountDocument<'source> {
    #[serde(rename = "type")]
    credential_type: &'static str,
    project_id: &'source str,
    private_key_id: &'source str,
    private_key: &'source Secret,
    client_email: &'source str,
    universe_domain: Option<&'source str>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthorizedUserSource {
    #[serde(rename = "type", default = "default_credential_type")]
    pub credential_type: String,
    pub client_id: String,
    pub client_secret: Secret,
    pub refresh_token: Secret,
    #[serde(default)]
    pub token_uri: Option<String>,
    #[serde(default)]
    pub quota_project_id: Option<String>,
    #[serde(default)]
    pub universe_domain: Option<String>,
}

fn default_credential_type() -> String {
    AUTHORIZED_USER_TYPE.to_owned()
}

#[derive(Debug, Clone)]
pub enum CredentialSource {
    ServiceAccount(ServiceAccountSource),
    AuthorizedUser(AuthorizedUserSource),
    ExternalAccount(Box<ExternalAccountSource>),
    // google.auth.default loads an impersonated_service_account ADC file; the SDK reads its whole text
    Impersonated(Secret),
}

impl CredentialSource {
    #[must_use]
    pub fn project_id(&self) -> Option<&str> {
        match self {
            Self::ServiceAccount(account) => account.project_id.as_deref(),
            Self::AuthorizedUser(_) | Self::ExternalAccount(_) | Self::Impersonated(_) => None,
        }
    }

    #[must_use]
    pub fn quota_project_id(&self) -> Option<&str> {
        match self {
            Self::AuthorizedUser(user) => user.quota_project_id.as_deref(),
            Self::ServiceAccount(_) | Self::ExternalAccount(_) | Self::Impersonated(_) => None,
        }
    }

    // the JSON the Google auth SDK builders read, the one place the key material is written out
    #[must_use]
    pub fn document(&self) -> String {
        let document = match self {
            Self::ServiceAccount(account) => serde_json::to_string(&ServiceAccountDocument {
                credential_type: SERVICE_ACCOUNT_TYPE,
                project_id: account.project_id.as_deref().unwrap_or_default(),
                private_key_id: account.private_key_id.as_deref().unwrap_or_default(),
                private_key: &account.private_key,
                client_email: &account.client_email,
                universe_domain: account.universe_domain.as_deref(),
            }),
            Self::AuthorizedUser(user) => serde_json::to_string(user),
            Self::ExternalAccount(account) => serde_json::to_string(account),
            Self::Impersonated(text) => Ok(text.expose_secret().0.clone()),
        };
        document.unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub enum ConfiguredCredentials {
    Source(CredentialSource),
    // a filename or malformed JSON that may still hold a private key
    Text(SecretString),
}

impl ConfiguredCredentials {
    pub fn from_text(text: String) -> Result<Self, CredentialDecodeError> {
        if serde_json::from_str::<CredentialKind>(&text).is_err() {
            return Ok(Self::Text(SecretString::from(text)));
        }
        decode_vertex_credentials(text.as_bytes()).map(Self::Source)
    }

    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text.expose_secret()),
            Self::Source(_) => None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialDecodeError {
    #[error("vertex credentials exceed {limit} bytes")]
    TooLarge { limit: usize },
    #[error("vertex credentials are not UTF-8")]
    Utf8(#[source] Utf8Error),
    #[error("vertex credentials are not a credential document")]
    Json(#[source] JsonError),
    #[error(
        "vertex_credentials names the executable credential source, which betterlite never runs"
    )]
    Executable,
    #[error("vertex credentials of type {kind} have no Google auth SDK builder")]
    Unsupported { kind: String },
}

#[derive(Deserialize)]
struct CredentialKind {
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[must_use = "the decoded credential source is the reason for the call"]
pub fn decode_vertex_credentials(bytes: &[u8]) -> Result<CredentialSource, CredentialDecodeError> {
    if bytes.len() > MAX_CREDENTIALS_BYTES {
        return Err(CredentialDecodeError::TooLarge {
            limit: MAX_CREDENTIALS_BYTES,
        });
    }
    let text = from_utf8(bytes).map_err(CredentialDecodeError::Utf8)?;
    let kind = serde_json::from_str::<CredentialKind>(text).map_err(CredentialDecodeError::Json)?;
    match kind.kind.as_deref() {
        Some("external_account") => {
            let source = serde_json::from_str::<ExternalAccountSource>(text)
                .map_err(CredentialDecodeError::Json)?;
            if source.is_executable() {
                return Err(CredentialDecodeError::Executable);
            }
            Ok(CredentialSource::ExternalAccount(Box::new(source)))
        }
        Some("impersonated_service_account") => Ok(CredentialSource::Impersonated(Secret::new(
            Box::new(KeyText(text.to_owned())),
        ))),
        Some(unsupported @ ("external_account_authorized_user" | "gdch_service_account")) => {
            Err(CredentialDecodeError::Unsupported {
                kind: unsupported.to_owned(),
            })
        }
        Some(AUTHORIZED_USER_TYPE) => serde_json::from_str::<AuthorizedUserSource>(text)
            .map(CredentialSource::AuthorizedUser)
            .map_err(CredentialDecodeError::Json),
        _ => serde_json::from_str::<ServiceAccountSource>(text)
            .map(CredentialSource::ServiceAccount)
            .map_err(CredentialDecodeError::Json),
    }
}

mod tests;
