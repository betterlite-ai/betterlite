pub mod aws_params;

use std::str::{Utf8Error, from_utf8};

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Error as JsonError, json};

use aws_params::{AwsParams, ExternalCredentialSource};

pub const MAX_CREDENTIALS_BYTES: usize = 1024 * 1024;
const DEFAULT_TOKEN_URL: &str = "https://sts.googleapis.com/v1/token";
const AUTHORIZED_USER_TYPE: &str = "authorized_user";

// the key material stays out of Debug and Display
#[derive(Clone, Debug, Deserialize)]
#[serde(transparent)]
pub struct Secret(pub SecretString);

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

impl ServiceAccountSource {
    #[must_use]
    pub fn document(&self) -> String {
        json!({
            "type": "service_account",
            "project_id": self.project_id.clone().unwrap_or_default(),
            "private_key_id": self.private_key_id.clone().unwrap_or_default(),
            "private_key": self.private_key.0.expose_secret(),
            "client_email": self.client_email,
            "universe_domain": self.universe_domain,
        })
        .to_string()
    }
}

#[derive(Debug, Clone, Deserialize)]
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

impl AuthorizedUserSource {
    #[must_use]
    pub fn document(&self) -> String {
        json!({
            "type": self.credential_type,
            "client_id": self.client_id,
            "client_secret": self.client_secret.0.expose_secret(),
            "refresh_token": self.refresh_token.0.expose_secret(),
            "token_uri": self.token_uri,
            "quota_project_id": self.quota_project_id,
            "universe_domain": self.universe_domain,
        })
        .to_string()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExternalAccountSource {
    #[serde(rename = "type", default = "default_type_external")]
    pub credential_type: String,
    pub audience: String,
    pub subject_token_type: String,
    #[serde(default = "default_token_url")]
    pub token_url: String,
    #[serde(default, rename = "service_account_impersonation_url")]
    pub impersonation_url: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_secret: Option<Secret>,
    #[serde(default)]
    pub scopes: Option<Vec<String>>,
    pub credential_source: ExternalCredentialSource,
    #[serde(default)]
    pub workforce_pool_user_project: Option<String>,
    #[serde(default)]
    pub universe_domain: Option<String>,
    #[serde(flatten)]
    pub aws: AwsParams,
}

fn default_type_external() -> String {
    "external_account".to_owned()
}

fn default_token_url() -> String {
    DEFAULT_TOKEN_URL.to_owned()
}

impl ExternalAccountSource {
    #[must_use]
    pub fn aws_environment(&self) -> Option<&str> {
        match &self.credential_source {
            ExternalCredentialSource::Aws { environment_id, .. } => Some(environment_id),
            ExternalCredentialSource::Executable { .. }
            | ExternalCredentialSource::Url { .. }
            | ExternalCredentialSource::File { .. } => None,
        }
    }

    #[must_use]
    pub const fn is_executable(&self) -> bool {
        matches!(
            self.credential_source,
            ExternalCredentialSource::Executable { .. }
        )
    }

    #[must_use]
    pub fn document(&self) -> String {
        let aws = &self.aws;
        json!({
            "type": self.credential_type,
            "audience": self.audience,
            "subject_token_type": self.subject_token_type,
            "token_url": self.token_url,
            "service_account_impersonation_url": self.impersonation_url,
            "client_id": self.client_id,
            "client_secret": self.client_secret.as_ref().map(|secret| secret.0.expose_secret()),
            "scopes": self.scopes,
            "workforce_pool_user_project": self.workforce_pool_user_project,
            "universe_domain": self.universe_domain,
            "credential_source": match &self.credential_source {
                ExternalCredentialSource::Aws {
                    environment_id,
                    region_url,
                    url,
                    regional_cred_verification_url,
                    imdsv2_session_token_url,
                } => json!({
                    "environment_id": environment_id,
                    "region_url": region_url,
                    "url": url,
                    "regional_cred_verification_url": regional_cred_verification_url,
                    "imdsv2_session_token_url": imdsv2_session_token_url,
                }),
                ExternalCredentialSource::Executable { executable } => json!({
                    "executable": {
                        "command": executable.command,
                        "timeout_millis": executable.timeout_millis,
                        "output_file": executable.output_file,
                    },
                }),
                ExternalCredentialSource::Url { url, headers, format } => json!({
                    "url": url,
                    "headers": headers,
                    "format": match format {
                        Some(format) => json!({
                            "type": format.format_type,
                            "subject_token_field_name": format.subject_token_field_name,
                        }),
                        None => json!(null),
                    },
                }),
                ExternalCredentialSource::File { file, format } => json!({
                    "file": file,
                    "format": match format {
                        Some(format) => json!({
                            "type": format.format_type,
                            "subject_token_field_name": format.subject_token_field_name,
                        }),
                        None => json!(null),
                    },
                }),
            },
            "aws_access_key_id": aws.access_key_id.as_ref().map(|secret| secret.0.expose_secret()),
            "aws_secret_access_key": aws.secret_access_key.as_ref().map(|secret| secret.0.expose_secret()),
            "aws_session_token": aws.session_token.as_ref().map(|secret| secret.0.expose_secret()),
            "aws_region_name": aws.region_name,
            "aws_session_name": aws.session_name,
            "aws_profile_name": aws.profile_name,
            "aws_role_name": aws.role_name,
            "aws_web_identity_token": aws.web_identity_token.as_ref().map(|secret| secret.0.expose_secret()),
            "aws_sts_endpoint": aws.sts_endpoint,
            "aws_external_id": aws.external_id,
        })
        .to_string()
    }
}

#[derive(Debug, Clone)]
pub enum CredentialSource {
    ServiceAccount(ServiceAccountSource),
    AuthorizedUser(AuthorizedUserSource),
    ExternalAccount(Box<ExternalAccountSource>),
}

impl CredentialSource {
    #[must_use]
    pub fn project_id(&self) -> Option<&str> {
        match self {
            Self::ServiceAccount(account) => account.project_id.as_deref(),
            Self::AuthorizedUser(_) | Self::ExternalAccount(_) => None,
        }
    }

    #[must_use]
    pub fn quota_project_id(&self) -> Option<&str> {
        match self {
            Self::AuthorizedUser(user) => user.quota_project_id.as_deref(),
            Self::ServiceAccount(_) | Self::ExternalAccount(_) => None,
        }
    }

    #[must_use]
    pub fn document(&self) -> String {
        match self {
            Self::ServiceAccount(account) => account.document(),
            Self::AuthorizedUser(user) => user.document(),
            Self::ExternalAccount(account) => account.document(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ConfiguredCredentials {
    Source(CredentialSource),
    Text(String),
}

impl ConfiguredCredentials {
    pub fn from_text(text: String) -> Result<Self, CredentialDecodeError> {
        if serde_json::from_str::<CredentialKind>(&text).is_err() {
            return Ok(Self::Text(text));
        }
        decode_vertex_credentials(text.as_bytes()).map(Self::Source)
    }

    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
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
        Some(AUTHORIZED_USER_TYPE) => serde_json::from_str::<AuthorizedUserSource>(text)
            .map(CredentialSource::AuthorizedUser)
            .map_err(CredentialDecodeError::Json),
        _ => serde_json::from_str::<ServiceAccountSource>(text)
            .map(CredentialSource::ServiceAccount)
            .map_err(CredentialDecodeError::Json),
    }
}

mod tests;
